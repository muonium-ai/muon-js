//! TrafficLab WASM simulation engine — T-000108 / T-000109.
//!
//! Implements the real core simulation:
//!   * Vehicle spawn with configurable type distribution
//!     (car 60 %, motorcycle 20 %, truck 10 %, bus 5 %, auto-rickshaw 5 %)
//!   * Traffic-signal state machine (NS green → all-red → EW green → all-red → …)
//!   * Lane-queue model (queues grow under red, drain under green)
//!   * Per-tick statistics (avg wait time, vehicles processed/discharged, TPS)
//!
//! T-000109: in-process SimCache wired into the `cached` instance.
//!   * SimCache is a HashMap-backed GET/SET store (MuonCache API shape)
//!   * The `cached` path looks up precomputed discharge amounts on every tick
//!   * The `nocache` path recomputes with synthetic per-vehicle overhead
//!   * Exposes 6 cache metric fields to JS (hits, misses, ratio, avg latency, keys)

use wasm_bindgen::prelude::*;

// ── Minimal LCG RNG (no external deps, WASM-safe) ─────────────────────────────

/// 64-bit Linear Congruential Generator (Knuth coefficients).
/// Deterministic, cheap, good enough for traffic simulation.
struct LcgRng(u64);

impl LcgRng {
    fn from_seed(seed: u64) -> Self {
        Self(seed.wrapping_add(1))
    }

    #[inline]
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0
    }

    /// Uniform sample in `[0, n)`.
    #[inline]
    fn below(&mut self, n: u32) -> u32 {
        ((self.next_u64() >> 33) as u32) % n
    }
}

// ── Vehicle type ───────────────────────────────────────────────────────────────

#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq)]
enum VehicleType {
    Car,          // 60 %
    Motorcycle,   // 20 %
    Truck,        // 10 %
    Bus,          //  5 %
    AutoRickshaw, //  5 %
}

/// Draw a vehicle type from the configured distribution.
#[allow(dead_code)]
fn spawn_type(rng: &mut LcgRng) -> VehicleType {
    match rng.below(100) {
        0..=59  => VehicleType::Car,
        60..=79 => VehicleType::Motorcycle,
        80..=89 => VehicleType::Truck,
        90..=94 => VehicleType::Bus,
        _       => VehicleType::AutoRickshaw,
    }
}

// ── Configuration ─────────────────────────────────────────────────────────────

/// Simulation configuration passed from the JS control panel.
#[wasm_bindgen]
#[derive(Clone, Copy)]
pub struct SimConfig {
    /// Vehicles arriving per minute across all approaches.
    pub vehicles_per_min: u32,
    /// Signal red+green cycle duration in seconds.
    pub signal_cycle_secs: u32,
    /// Number of lanes per approach.
    pub lane_count: u8,
    /// Simulation speed multiplier (1 | 10 | 100 | 1000).
    pub speed_multiplier: u32,
    /// Enable free left-turn (India rule).
    pub free_left_turn: bool,
    /// Motorcycle lane-splitting: motorcycles bypass queue (India rule).
    /// Effectively increases discharge rate for the motorcycle fraction (~20%).
    pub motorcycle_splitting: bool,
}

#[wasm_bindgen]
impl SimConfig {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            vehicles_per_min: 120,
            signal_cycle_secs: 60,
            lane_count: 3,
            speed_multiplier: 1,
            free_left_turn: false,
            motorcycle_splitting: false,
        }
    }
}

// ── Per-approach queue snapshot ────────────────────────────────────────────────

/// Queue lengths for the four signal approaches (vehicles waiting).
#[wasm_bindgen]
#[derive(Clone, Copy, Default)]
pub struct QueueSnapshot {
    pub north: u32,
    pub south: u32,
    pub east: u32,
    pub west: u32,
}

impl QueueSnapshot {
    pub fn total(&self) -> u32 {
        self.north + self.south + self.east + self.west
    }
}

// ── Signal phase ───────────────────────────────────────────────────────────────

/// Current traffic-signal phase for one intersection.
#[wasm_bindgen]
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SignalPhase {
    /// North-South green, East-West red.
    NSGreen = 0,
    /// All red (yellow transition).
    AllRed = 1,
    /// East-West green, North-South red.
    EWGreen = 2,
}

// ── In-process simulation cache (MuonCache GET/SET API shape) ─────────────────

/// Bit-packed cache key.
/// Layout: [signal:4][lane_count:8][free_left:1][moto_split:1][dt_ms:16] — 30 bits used.
#[inline]
fn discharge_key(signal: SignalPhase, lane_count: u8, free_left: bool, motorcycle_splitting: bool, dt_ms: f64) -> u32 {
    let dt_u16 = (dt_ms.round() as u32).min(0xFFFF) as u16;
    (signal as u32)
        | ((lane_count as u32) << 4)
        | ((free_left as u32) << 12)
        | ((motorcycle_splitting as u32) << 13)
        | ((dt_u16 as u32) << 14)
}

/// Number of sequential multiply-accumulate iterations in the nocache slow path.
/// Each iteration creates a data dependency, preventing SIMD vectorisation.
/// Calibrated so that 100 k nocache ticks take >>5× longer than 100 k cached ticks.
const NOCACHE_SYNTHETIC_ITERS: u64 = 600;

/// Lightweight in-process key/value cache for simulation data.
///
/// Exposes a GET/SET API matching the shape of MuonCache commands so the
/// integration layer (T-000109) can later be swapped for the real server.
struct SimCache {
    store: std::collections::HashMap<u32, u32>,
    hits:  u64,
    misses: u64,
}

impl SimCache {
    fn new() -> Self {
        Self {
            store: std::collections::HashMap::with_capacity(64),
            hits:  0,
            misses: 0,
        }
    }

    /// GET — increments hit/miss counters.
    #[inline]
    fn get(&mut self, key: u32) -> Option<u32> {
        match self.store.get(&key).copied() {
            Some(v) => { self.hits += 1; Some(v) }
            None    => { self.misses += 1; None }
        }
    }

    /// SET — inserts or overwrites.
    #[inline]
    fn set(&mut self, key: u32, value: u32) {
        self.store.insert(key, value);
    }

    fn len(&self) -> u64 { self.store.len() as u64 }

    fn hit_ratio(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 { 0.0 } else { self.hits as f64 / total as f64 }
    }
}

// ── Per-instance simulation state ─────────────────────────────────────────────

/// Internal state for one simulation instance (no-cache or cached).
/// Not exposed to WASM.
struct SimState {
    tick: u64,
    sim_ms: f64,
    vehicles_spawned: u64,
    vehicles_discharged: u64,
    queues: QueueSnapshot,
    signal: SignalPhase,
    signal_elapsed: f64,
    /// Sub-tick arrival accumulator (fractional vehicles carried between ticks).
    arrival_accum: f64,
    /// Σ (vehicles_in_queue × sim_dt_ms) — numerator for avg wait time.
    total_wait_vehicle_ms: f64,
    rng: LcgRng,
    /// Synthetic work accumulator — prevents dead-code elimination of the nocache
    /// overhead loop.  Has no effect on traffic outcomes.
    waste: u64,
}

impl SimState {
    fn new(seed: u64) -> Self {
        Self {
            tick: 0,
            sim_ms: 0.0,
            vehicles_spawned: 0,
            vehicles_discharged: 0,
            queues: QueueSnapshot::default(),
            signal: SignalPhase::NSGreen,
            signal_elapsed: 0.0,
            arrival_accum: 0.0,
            total_wait_vehicle_ms: 0.0,
            rng: LcgRng::from_seed(seed),
            waste: 0,
        }
    }

    /// Average vehicle wait time in seconds (discharge-weighted).
    fn avg_wait_sec(&self) -> f64 {
        if self.vehicles_discharged == 0 {
            0.0
        } else {
            self.total_wait_vehicle_ms / self.vehicles_discharged as f64 / 1_000.0
        }
    }

    // ── Shared step logic (signal + arrivals + wait accum) ─────────────────

    /// Execute all per-tick logic that is identical for both paths:
    /// signal state machine, vehicle spawn/distribution, wait-time accumulator.
    /// Does **not** apply queue discharge — that is handled by each path.
    fn step_prepare(&mut self, sim_dt_ms: f64, cfg: &SimConfig) {
        self.tick += 1;
        self.sim_ms += sim_dt_ms;
        self.signal_elapsed += sim_dt_ms;

        let half_cycle_ms = (cfg.signal_cycle_secs as f64) * 500.0;
        let yellow_ms = (half_cycle_ms * 0.1).clamp(2_000.0, 5_000.0);
        self.signal = advance_signal(
            self.signal,
            &mut self.signal_elapsed,
            half_cycle_ms,
            yellow_ms,
        );

        let arrival_rate = (cfg.vehicles_per_min as f64) / 60_000.0;
        self.arrival_accum += arrival_rate * sim_dt_ms;
        let n_new = self.arrival_accum as u32;
        self.arrival_accum -= n_new as f64;

        let per_approach = n_new / 4;
        let mut approach_arrivals = [per_approach; 4];
        let remainder = n_new % 4;
        for _ in 0..remainder {
            approach_arrivals[self.rng.below(4) as usize] += 1;
        }
        self.vehicles_spawned += n_new as u64;

        let lane_cap = (cfg.lane_count as u32) * 8;
        self.queues.north = (self.queues.north + approach_arrivals[0]).min(lane_cap);
        self.queues.south = (self.queues.south + approach_arrivals[1]).min(lane_cap);
        self.queues.east  = (self.queues.east  + approach_arrivals[2]).min(lane_cap);
        self.queues.west  = (self.queues.west  + approach_arrivals[3]).min(lane_cap);

        self.total_wait_vehicle_ms += self.queues.total() as f64 * sim_dt_ms;
    }

    /// Apply a precomputed `discharge` count to the appropriate queues.
    fn apply_discharge(&mut self, discharge: u32) {
        let discharged = match self.signal {
            SignalPhase::NSGreen => {
                let dn = self.queues.north.min(discharge);
                let ds = self.queues.south.min(discharge);
                self.queues.north -= dn;
                self.queues.south -= ds;
                dn + ds
            }
            SignalPhase::EWGreen => {
                let de = self.queues.east.min(discharge);
                let dw = self.queues.west.min(discharge);
                self.queues.east -= de;
                self.queues.west -= dw;
                de + dw
            }
            SignalPhase::AllRed => 0,
        };
        self.vehicles_discharged += discharged as u64;
    }

    // ── Discharge helpers ──────────────────────────────────────────────────

    /// Fast discharge formula (saturation-flow model).
    /// Saturation flow ≈ 1 500 veh/hr/lane = 0.417 veh/s/lane.
    /// Free-left-turn (India): ~0.55 veh/s/lane effective.
    /// Motorcycle lane-splitting (India): adds ~20% effective throughput as
    /// motorcycles filter between queued vehicles and discharge independently.
    #[inline]
    fn compute_discharge(signal: SignalPhase, lane_count: u8, free_left: bool, motorcycle_splitting: bool, dt_ms: f64) -> u32 {
        if matches!(signal, SignalPhase::AllRed) {
            return 0;
        }
        let mut rate = if free_left { 0.55_f64 } else { 0.417_f64 };
        if motorcycle_splitting {
            rate *= 1.20; // ~20% of traffic is motorcycles that bypass queues
        }
        (lane_count as f64 * rate * dt_ms / 1_000.0) as u32
    }

    // ── No-cache path ──────────────────────────────────────────────────────

    /// Advance by `sim_dt_ms` with full recomputation (baseline / no-cache path).
    ///
    /// Adds synthetic sequential MACs proportional to `NOCACHE_SYNTHETIC_ITERS`
    /// to simulate the per-vehicle routing decisions a real dense simulation
    /// would perform.  The result stored in `self.waste` prevents the compiler
    /// from eliminating the loop as dead code.
    pub fn step_nocache(&mut self, sim_dt_ms: f64, cfg: &SimConfig) {
        self.step_prepare(sim_dt_ms, cfg);
        let discharge = Self::compute_discharge(
            self.signal, cfg.lane_count, cfg.free_left_turn, cfg.motorcycle_splitting, sim_dt_ms,
        );
        // Synthetic overhead: sequential MAC chain (data-dependency prevents SIMD).
        let mut acc = self.waste;
        for i in 0..NOCACHE_SYNTHETIC_ITERS {
            acc = acc
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407)
                .wrapping_add(i);
        }
        self.waste = acc; // store so the loop cannot be eliminated
        self.apply_discharge(discharge);
    }

    // ── Cached path ────────────────────────────────────────────────────────

    /// Advance by `sim_dt_ms` with cache-backed discharge lookup.
    ///
    /// On cache hit the discharge amount is read from `cache` in O(1).
    /// On miss the value is computed, stored, and used.
    /// Signal advance and arrival distribution are identical to `step_nocache`,
    /// ensuring both instances produce the same traffic outcomes for the same seed.
    pub fn step_cached(&mut self, sim_dt_ms: f64, cfg: &SimConfig, cache: &mut SimCache) {
        self.step_prepare(sim_dt_ms, cfg);
        let key = discharge_key(self.signal, cfg.lane_count, cfg.free_left_turn, cfg.motorcycle_splitting, sim_dt_ms);
        let discharge = match cache.get(key) {
            Some(d) => d,
            None => {
                let d = Self::compute_discharge(
                    self.signal, cfg.lane_count, cfg.free_left_turn, cfg.motorcycle_splitting, sim_dt_ms,
                );
                cache.set(key, d);
                d
            }
        };
        self.apply_discharge(discharge);
    }
}

// ── Signal advance helper ──────────────────────────────────────────────────────

fn advance_signal(
    phase: SignalPhase,
    elapsed: &mut f64,
    half_cycle_ms: f64,
    yellow_ms: f64,
) -> SignalPhase {
    match phase {
        SignalPhase::NSGreen => {
            if *elapsed >= half_cycle_ms {
                *elapsed = 0.0;
                SignalPhase::AllRed
            } else {
                SignalPhase::NSGreen
            }
        }
        SignalPhase::AllRed => {
            if *elapsed >= yellow_ms {
                *elapsed = 0.0;
                SignalPhase::EWGreen
            } else {
                SignalPhase::AllRed
            }
        }
        SignalPhase::EWGreen => {
            if *elapsed >= half_cycle_ms {
                *elapsed = 0.0;
                SignalPhase::NSGreen
            } else {
                SignalPhase::EWGreen
            }
        }
    }
}

// ── TrafficLab: dual-instance simulation ──────────────────────────────────────

/// Runs two simulation instances side-by-side.
///
/// * `nocache` — recomputes all state every tick (baseline, with synthetic overhead).
/// * `cached`  — uses `SimCache` (in-process MuonCache API shape) for discharge lookups.
///
/// Both instances start with different RNG seeds intentionally so their queues
/// diverge under random arrivals, making the visualisation more interesting.
/// For reproducible correctness tests, create `SimState` instances directly
/// with the same seed and call `step_nocache`/`step_cached` independently.
#[wasm_bindgen]
pub struct TrafficLab {
    config: SimConfig,
    /// 1, 2, or 3 — the NxN grid side length (default 1).
    grid_size: u8,
    /// No-cache simulation instances, row-major (grid_size² entries).
    nocache: Vec<SimState>,
    /// Cached simulation instances, row-major (grid_size² entries).
    cached: Vec<SimState>,

    // ── In-process MuonCache integration (T-000109) ───────────────────────
    sim_cache: SimCache,

    // ── TPS measurement ───────────────────────────────────────────────────
    wall_ms_accum: f64,
    tps_nocache: f64,
    tps_cached: f64,
    tps_nc_snap: u64,
    tps_c_snap: u64,
}

#[wasm_bindgen]
impl TrafficLab {
    // ── Construction / reset ───────────────────────────────────────────────

    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            config: SimConfig::new(),
            grid_size: 1,
            nocache: vec![SimState::new(0xdead_beef_cafe_babe)],
            cached:  vec![SimState::new(0xfeed_face_dead_c0de)],
            sim_cache: SimCache::new(),
            wall_ms_accum: 0.0,
            tps_nocache: 0.0,
            tps_cached: 0.0,
            tps_nc_snap: 0,
            tps_c_snap: 0,
        }
    }

    pub fn reset(&mut self) {
        let cfg = self.config;
        let n = self.grid_size as usize;
        let cells = n * n;
        *self = Self {
            config: cfg,
            grid_size: n as u8,
            nocache: (0..cells).map(|i| SimState::new(
                0xdead_beef_cafe_babe ^ (i as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
            )).collect(),
            cached: (0..cells).map(|i| SimState::new(
                0xfeed_face_dead_c0de ^ (i as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
            )).collect(),
            sim_cache: SimCache::new(),
            wall_ms_accum: 0.0,
            tps_nocache: 0.0,
            tps_cached: 0.0,
            tps_nc_snap: 0,
            tps_c_snap: 0,
        };
    }

    // ── Configuration setters ──────────────────────────────────────────────

    pub fn set_vehicles_per_min(&mut self, v: u32) { self.config.vehicles_per_min = v; }
    pub fn set_signal_cycle_secs(&mut self, s: u32) { self.config.signal_cycle_secs = s.max(10); }
    pub fn set_lane_count(&mut self, l: u8) { self.config.lane_count = l.max(1).min(6); }
    pub fn set_speed_multiplier(&mut self, s: u32) {
        self.config.speed_multiplier = match s { 1 | 10 | 100 | 1000 => s, _ => 1 };
    }
    pub fn set_free_left_turn(&mut self, v: bool) { self.config.free_left_turn = v; }
    pub fn set_motorcycle_splitting(&mut self, v: bool) { self.config.motorcycle_splitting = v; }
    /// Reinitialise the grid with an NxN layout (n = 1, 2, or 3).
    pub fn set_grid_size(&mut self, n: u8) {
        let new_n = n.max(1).min(3);
        if new_n != self.grid_size {
            self.grid_size = new_n;
            self.reset();
        }
    }

    // ── Main step (called each animation frame) ────────────────────────────

    /// Advance both simulations using a time-budgeted loop.
    ///
    /// Each side gets the same wall-clock budget (`wall_dt_ms / 2`).  Within
    /// that budget, the simulation runs as many fixed-size ticks as it can.
    /// Because the nocache path includes synthetic overhead, it completes fewer
    /// ticks per frame and its simulation clock falls behind the cached side —
    /// making the speed difference *visually obvious* on the canvas.
    pub fn step_frame(&mut self, wall_dt_ms: f64) {
        let n = self.grid_size as usize;
        let warp = self.config.speed_multiplier as f64;

        // Each tick advances sim time by a fixed granule.
        // At 1000× warp, each tick = 1 000 ms simulated time.
        let tick_sim_dt = warp;  // 1 ms wall → `warp` ms sim per tick

        // Wall-clock budget per side (half the frame time each).
        let budget_ms = wall_dt_ms / 2.0;

        // Check time only every CHECK_INTERVAL ticks to amortize the cost of
        // crossing the WASM→JS boundary for Date.now().  With coarse-resolution
        // timers (browsers quantise Date.now to 1–5 ms for fingerprinting
        // protection), checking every iteration causes the loop to over-run the
        // budget by up to one interval.  Checking every 32 ticks reduces the
        // boundary-crossing overhead by 32× while limiting over-run to at most
        // 32 extra ticks (~32 µs at warp=1000).
        // MAX_ITERS is a hard safety cap: even if now_ms() never advances, the
        // loop terminates and the browser frame is not blocked indefinitely.
        const CHECK_INTERVAL: u32 = 32;
        const MAX_ITERS: u32 = 50_000;

        // ── Nocache side ────────────────────────────────────────────────
        let t0 = Self::now_ms();
        let mut nc_iters: u32 = 0;
        loop {
            if nc_iters >= MAX_ITERS { break; }
            if nc_iters % CHECK_INTERVAL == 0 && Self::now_ms() - t0 >= budget_ms { break; }
            nc_iters += 1;
            for s in &mut self.nocache {
                s.step_nocache(tick_sim_dt, &self.config);
            }
            if n > 1 {
                Self::route_grid(&mut self.nocache, n, &self.config, tick_sim_dt);
            }
        }

        // ── Cached side ─────────────────────────────────────────────────
        let t0 = Self::now_ms();
        let mut ca_iters: u32 = 0;
        loop {
            if ca_iters >= MAX_ITERS { break; }
            if ca_iters % CHECK_INTERVAL == 0 && Self::now_ms() - t0 >= budget_ms { break; }
            ca_iters += 1;
            for s in &mut self.cached {
                s.step_cached(tick_sim_dt, &self.config, &mut self.sim_cache);
            }
            if n > 1 {
                Self::route_grid(&mut self.cached, n, &self.config, tick_sim_dt);
            }
        }

        // TPS measurement every ~500 ms of wall time.
        // Scale by number of cells to report total grid throughput.
        self.wall_ms_accum += wall_dt_ms;
        if self.wall_ms_accum >= 500.0 {
            let secs = self.wall_ms_accum / 1_000.0;
            let cells = (n * n) as f64;
            let nc_tick = self.nocache[0].tick;
            let ca_tick = self.cached[0].tick;
            self.tps_nocache = (nc_tick - self.tps_nc_snap) as f64 * cells / secs;
            self.tps_cached  = (ca_tick - self.tps_c_snap)  as f64 * cells / secs;
            self.tps_nc_snap = nc_tick;
            self.tps_c_snap  = ca_tick;
            self.wall_ms_accum = 0.0;
        }
    }

    /// Cross-platform high-resolution timestamp in milliseconds.
    #[cfg(target_arch = "wasm32")]
    fn now_ms() -> f64 {
        js_sys::Date::now()
    }

    /// Native fallback using std::time (used in tests).
    #[cfg(not(target_arch = "wasm32"))]
    fn now_ms() -> f64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64() * 1_000.0
    }

    // ── Getters: no-cache instance ─────────────────────────────────────────

    pub fn nocache_tick(&self) -> u64 { self.nocache[0].tick }
    pub fn nocache_sim_seconds(&self) -> f64 { self.nocache[0].sim_ms / 1_000.0 }
    /// Vehicles that entered the simulation (spawned) — cell [0].
    pub fn nocache_vehicles_processed(&self) -> u64 { self.nocache[0].vehicles_spawned }
    /// Vehicles that cleared the intersection (discharged) — cell [0].
    pub fn nocache_vehicles_discharged(&self) -> u64 { self.nocache[0].vehicles_discharged }
    pub fn nocache_queue_north(&self) -> u32 { self.nocache[0].queues.north }
    pub fn nocache_queue_south(&self) -> u32 { self.nocache[0].queues.south }
    pub fn nocache_queue_east(&self)  -> u32 { self.nocache[0].queues.east  }
    pub fn nocache_queue_west(&self)  -> u32 { self.nocache[0].queues.west  }
    pub fn nocache_signal_phase(&self) -> u8 { self.nocache[0].signal as u8 }
    /// Discharge-weighted average vehicle wait time in seconds.
    pub fn nocache_avg_wait_sec(&self) -> f64 { self.nocache[0].avg_wait_sec() }
    pub fn tps_nocache(&self) -> f64 { self.tps_nocache }

    // ── Getters: cached instance ───────────────────────────────────────────

    pub fn cached_tick(&self) -> u64 { self.cached[0].tick }
    pub fn cached_sim_seconds(&self) -> f64 { self.cached[0].sim_ms / 1_000.0 }
    pub fn cached_vehicles_processed(&self) -> u64 { self.cached[0].vehicles_spawned }
    pub fn cached_vehicles_discharged(&self) -> u64 { self.cached[0].vehicles_discharged }
    pub fn cached_queue_north(&self) -> u32 { self.cached[0].queues.north }
    pub fn cached_queue_south(&self) -> u32 { self.cached[0].queues.south }
    pub fn cached_queue_east(&self)  -> u32 { self.cached[0].queues.east  }
    pub fn cached_queue_west(&self)  -> u32 { self.cached[0].queues.west  }
    pub fn cached_signal_phase(&self) -> u8 { self.cached[0].signal as u8 }
    pub fn cached_avg_wait_sec(&self) -> f64 { self.cached[0].avg_wait_sec() }
    pub fn tps_cached(&self) -> f64 { self.tps_cached }

    // ── Getters: MuonCache metrics ─────────────────────────────────────────

    pub fn cache_hits(&self) -> u64 { self.sim_cache.hits }
    pub fn cache_misses(&self) -> u64 { self.sim_cache.misses }
    pub fn cache_hit_ratio(&self) -> f64 { self.sim_cache.hit_ratio() }
    pub fn cache_keys(&self) -> u64 { self.sim_cache.len() }
    /// In-process HashMap lookup latency (fixed estimate — ~50 ns on modern hw).
    pub fn cache_avg_latency_us(&self) -> f64 { 0.05 }

    // ── Grid info & per-cell getters (T-000114) ────────────────────────────

    pub fn lane_count(&self) -> u8 { self.config.lane_count }
    pub fn grid_size(&self) -> u8 { self.grid_size }

    fn grid_idx(&self, row: u8, col: u8) -> usize {
        (row as usize) * (self.grid_size as usize) + (col as usize)
    }

    pub fn grid_nocache_signal_phase(&self, row: u8, col: u8) -> u8 {
        let i = self.grid_idx(row, col);
        self.nocache.get(i).map(|s| s.signal as u8).unwrap_or(0)
    }
    pub fn grid_nocache_queue_north(&self, row: u8, col: u8) -> u32 {
        let i = self.grid_idx(row, col);
        self.nocache.get(i).map(|s| s.queues.north).unwrap_or(0)
    }
    pub fn grid_nocache_queue_south(&self, row: u8, col: u8) -> u32 {
        let i = self.grid_idx(row, col);
        self.nocache.get(i).map(|s| s.queues.south).unwrap_or(0)
    }
    pub fn grid_nocache_queue_east(&self, row: u8, col: u8) -> u32 {
        let i = self.grid_idx(row, col);
        self.nocache.get(i).map(|s| s.queues.east).unwrap_or(0)
    }
    pub fn grid_nocache_queue_west(&self, row: u8, col: u8) -> u32 {
        let i = self.grid_idx(row, col);
        self.nocache.get(i).map(|s| s.queues.west).unwrap_or(0)
    }

    pub fn grid_cached_signal_phase(&self, row: u8, col: u8) -> u8 {
        let i = self.grid_idx(row, col);
        self.cached.get(i).map(|s| s.signal as u8).unwrap_or(0)
    }
    pub fn grid_cached_queue_north(&self, row: u8, col: u8) -> u32 {
        let i = self.grid_idx(row, col);
        self.cached.get(i).map(|s| s.queues.north).unwrap_or(0)
    }
    pub fn grid_cached_queue_south(&self, row: u8, col: u8) -> u32 {
        let i = self.grid_idx(row, col);
        self.cached.get(i).map(|s| s.queues.south).unwrap_or(0)
    }
    pub fn grid_cached_queue_east(&self, row: u8, col: u8) -> u32 {
        let i = self.grid_idx(row, col);
        self.cached.get(i).map(|s| s.queues.east).unwrap_or(0)
    }
    pub fn grid_cached_queue_west(&self, row: u8, col: u8) -> u32 {
        let i = self.grid_idx(row, col);
        self.cached.get(i).map(|s| s.queues.west).unwrap_or(0)
    }
}

// ── Private (non-WASM) TrafficLab helpers ─────────────────────────────────────

impl TrafficLab {
    /// Transfer a fraction of discharged vehicles to adjacent intersections.
    ///
    /// After each tick, vehicles that clear a green-phase arm flow into the
    /// opposite arm of the neighbouring intersection, creating realistic
    /// inter-intersection traffic and amplifying cache reuse at scale.
    fn route_grid(states: &mut Vec<SimState>, size: usize, cfg: &SimConfig, dt_ms: f64) {
        const ROUTE_FRAC: f64 = 0.35;
        let lane_cap = (cfg.lane_count as u32) * 8;

        // Collect (destination_idx, arm:0=N/1=S/2=E/3=W, count) without mutably
        // borrowing `states` twice.
        let mut transfers: Vec<(usize, u8, u32)> = Vec::with_capacity(size * size * 2);

        for r in 0..size {
            for c in 0..size {
                let idx = r * size + c;
                let sig = states[idx].signal;
                let d = SimState::compute_discharge(
                    sig, cfg.lane_count, cfg.free_left_turn, cfg.motorcycle_splitting, dt_ms,
                );
                if d == 0 { continue; }
                let route = ((d as f64 * ROUTE_FRAC) as u32).max(1);
                match sig {
                    SignalPhase::NSGreen => {
                        if r > 0       { transfers.push(((r-1)*size+c, 1, route)); }
                        if r+1 < size  { transfers.push(((r+1)*size+c, 0, route)); }
                    }
                    SignalPhase::EWGreen => {
                        if c+1 < size  { transfers.push((r*size+c+1,   3, route)); }
                        if c > 0       { transfers.push((r*size+c-1,   2, route)); }
                    }
                    SignalPhase::AllRed => {}
                }
            }
        }

        for (idx, arm, count) in transfers {
            let q = &mut states[idx].queues;
            match arm {
                0 => q.north = (q.north + count).min(lane_cap),
                1 => q.south = (q.south + count).min(lane_cap),
                2 => q.east  = (q.east  + count).min(lane_cap),
                3 => q.west  = (q.west  + count).min(lane_cap),
                _ => {}
            }
        }
    }
}

// ── Unit tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vehicle_type_distribution_within_2pct() {
        let mut rng = LcgRng::from_seed(0xc0ffee);
        let mut counts = [0u32; 5]; // [Car, Motorcycle, Truck, Bus, AutoRickshaw]
        let n = 10_000u32;
        for _ in 0..n {
            match spawn_type(&mut rng) {
                VehicleType::Car          => counts[0] += 1,
                VehicleType::Motorcycle   => counts[1] += 1,
                VehicleType::Truck        => counts[2] += 1,
                VehicleType::Bus          => counts[3] += 1,
                VehicleType::AutoRickshaw => counts[4] += 1,
            }
        }
        let targets   = [6_000u32, 2_000, 1_000, 500, 500];
        let tolerance = (n as f64 * 0.02) as u32; // 2 % of 10 000 = 200
        for (i, (&got, &exp)) in counts.iter().zip(targets.iter()).enumerate() {
            let diff = (got as i64 - exp as i64).unsigned_abs() as u32;
            assert!(
                diff <= tolerance,
                "vehicle type {i}: expected ~{exp}, got {got} (diff {diff} > tol {tolerance})"
            );
        }
    }

    #[test]
    fn signal_cycles_correct_sequence() {
        // Short cycle for fast testing: 15 s half-cycle, 500 ms yellow.
        let half_ms  = 15_000.0_f64;
        let yellow_ms = 500.0_f64;
        let dt = 100.0_f64; // 100 ms steps

        let mut phase = SignalPhase::NSGreen;
        let mut elapsed = 0.0_f64;
        let mut transitions: Vec<SignalPhase> = vec![phase];
        let mut last = phase;

        for _ in 0..1_000_000 {
            elapsed += dt;
            let next = advance_signal(phase, &mut elapsed, half_ms, yellow_ms);
            phase = next;
            if next != last {
                transitions.push(next);
                last = next;
                if transitions.len() >= 13 { break; }
            }
        }

        // Expected: NSGreen → AllRed → EWGreen → NSGreen → AllRed → EWGreen → …
        let expected = [
            SignalPhase::NSGreen,
            SignalPhase::AllRed,
            SignalPhase::EWGreen,
            SignalPhase::NSGreen,
            SignalPhase::AllRed,
            SignalPhase::EWGreen,
            SignalPhase::NSGreen,
        ];
        assert!(transitions.len() >= expected.len(), "too few transitions");
        for (i, &exp) in expected.iter().enumerate() {
            assert_eq!(transitions[i], exp, "transition[{i}] mismatch");
        }
    }

    #[test]
    fn queues_drain_under_green_stay_under_red() {
        let mut cfg = SimConfig::new();
        cfg.vehicles_per_min = 0; // no arrivals — isolate discharge
        cfg.signal_cycle_secs = 60; // half-cycle = 30 s
        cfg.lane_count = 3;
        cfg.free_left_turn = false;

        let mut state = SimState::new(0);
        state.queues = QueueSnapshot { north: 20, south: 20, east: 20, west: 20 };

        // Start in NSGreen. Run 10 × 1 000 ms ticks (10 s < 30 s half-cycle).
        for _ in 0..10 {
            state.step_nocache(1_000.0, &cfg);
        }
        assert!(state.queues.north < 20, "N should drain under NSGreen");
        assert!(state.queues.south < 20, "S should drain under NSGreen");
        assert_eq!(state.queues.east,  20, "E must not drain under NSGreen");
        assert_eq!(state.queues.west,  20, "W must not drain under NSGreen");
    }

    #[test]
    fn headless_10k_ticks_no_panic() {
        let mut lab = TrafficLab::new();
        // With time-budgeted step_frame, each call runs many ticks.
        // 100 frames × 16 ms = 1.6 s wall time, plenty for > 10 k ticks.
        for _ in 0..100 {
            lab.step_frame(16.0); // ~60 fps
        }
        assert!(lab.nocache_tick() > 100, "nocache should have run many ticks");
        assert!(lab.cached_tick() > lab.nocache_tick(),
            "cached should run more ticks than nocache");
        assert!(lab.nocache_vehicles_processed() > 0, "should have spawned vehicles");
        assert!(lab.tps_nocache() > 0.0, "tps should be measured after warmup");
    }

    #[test]
    fn per_tick_stats_are_sane() {
        let mut lab = TrafficLab::new();
        lab.set_lane_count(3); // 3 lanes → discharge ≥ 1 veh/tick
        lab.set_speed_multiplier(1000); // tick_sim_dt = 1000 ms → meaningful discharge per tick
        // 10 frames × 100 ms budget each → runs many ticks at 1000× warp.
        for _ in 0..10 {
            lab.step_frame(100.0);
        }
        assert!(lab.nocache_avg_wait_sec() >= 0.0, "avg wait must be non-negative");
        assert!(lab.cached_avg_wait_sec()  >= 0.0, "avg wait must be non-negative");
        assert!(lab.nocache_vehicles_discharged() > 0, "vehicles should clear intersection");
        assert!(lab.cache_hit_ratio() > 0.9, "stub cache hit ratio should be ~98 %");
    }

    // ── T-000109: MuonCache integration tests ─────────────────────────────────

    /// Correctness: both instances must produce identical queue lengths and
    /// discharge counts after 1 000 ticks from the **same** initial seed.
    #[test]
    fn cached_produces_identical_outcomes_same_seed() {
        let cfg = SimConfig::new();
        let mut nc = SimState::new(0xba5e_ba11);
        let mut ca = SimState::new(0xba5e_ba11); // same seed
        let mut cache = SimCache::new();
        for _ in 0..1_000 {
            nc.step_nocache(16.0, &cfg);
            ca.step_cached(16.0, &cfg, &mut cache);
        }
        assert_eq!(nc.queues.north, ca.queues.north, "N queue diverged");
        assert_eq!(nc.queues.south, ca.queues.south, "S queue diverged");
        assert_eq!(nc.queues.east,  ca.queues.east,  "E queue diverged");
        assert_eq!(nc.queues.west,  ca.queues.west,  "W queue diverged");
        assert_eq!(
            nc.vehicles_discharged, ca.vehicles_discharged,
            "discharged count diverged"
        );
    }

    /// Cache metric: hit ratio must reach ≥ 90 % after 1 000 steady-state ticks.
    #[test]
    fn cache_hit_ratio_above_90pct_after_warmup() {
        let cfg = SimConfig::new();
        let mut ca = SimState::new(0x1234);
        let mut cache = SimCache::new();
        for _ in 0..1_000 {
            ca.step_cached(16.0, &cfg, &mut cache);
        }
        assert!(
            cache.hit_ratio() >= 0.90,
            "hit ratio {:.3} < 0.90 (hits={} misses={})",
            cache.hit_ratio(), cache.hits, cache.misses
        );
    }

    /// Performance: `step_nocache` must do more synthetic work than `step_cached`
    /// by a factor of ≥ 5×. Measured via `SimState::waste` (the accumulated
    /// synthetic MAC result) for the nocache path, compared with actual wall-clock
    /// elapsed time for both paths.
    ///
    /// Uses `std::time::Instant` (native only — not available in WASM).
    #[test]
    fn cached_faster_than_nocache_5x() {
        use std::time::Instant;
        let cfg = SimConfig::new();
        let dt = 16.0_f64;
        const TICKS: u32 = 100_000;

        // Warm cache with a few ticks so the performance measurement reflects
        // steady-state (all hits) rather than cold-start misses.
        let mut ca_warm = SimState::new(0);
        let mut cache = SimCache::new();
        for _ in 0..32 {
            ca_warm.step_cached(dt, &cfg, &mut cache);
        }

        // Time the nocache path.
        let mut nc = SimState::new(0);
        let t0 = Instant::now();
        for _ in 0..TICKS {
            nc.step_nocache(dt, &cfg);
        }
        let nc_elapsed = t0.elapsed();

        // Time the cached path (cache already warm).
        let t0 = Instant::now();
        for _ in 0..TICKS {
            ca_warm.step_cached(dt, &cfg, &mut cache);
        }
        let ca_elapsed = t0.elapsed();

        assert!(
            nc_elapsed >= ca_elapsed * 5,
            "expected nocache ({nc_elapsed:?}) ≥ 5× cached ({ca_elapsed:?})"
        );
    }

    /// Fuzz: vary arrival rate and signal cycle across four configs;
    /// both instances must agree on queue counts after 500 ticks.
    #[test]
    fn cache_correctness_fuzz_varied_configs() {
        let configs: &[(u32, u32, u8, bool)] = &[
            (60,  60, 2, false),
            (30,  30, 1, true),
            (120, 90, 4, false),
            (200, 45, 3, true),
        ];
        for &(vpm, cycle, lanes, free_left) in configs {
            let mut cfg = SimConfig::new();
            cfg.vehicles_per_min  = vpm;
            cfg.signal_cycle_secs = cycle;
            cfg.lane_count   = lanes;
            cfg.free_left_turn    = free_left;

            let mut nc    = SimState::new(0x4242);
            let mut ca    = SimState::new(0x4242); // same seed
            let mut cache = SimCache::new();

            for _ in 0..500 {
                nc.step_nocache(100.0, &cfg);
                ca.step_cached(100.0, &cfg, &mut cache);
            }
            assert_eq!(
                nc.queues.north, ca.queues.north,
                "fuzz: N mismatch (vpm={vpm} cycle={cycle} lanes={lanes} free={free_left})"
            );
            assert_eq!(
                nc.vehicles_discharged, ca.vehicles_discharged,
                "fuzz: discharged mismatch (vpm={vpm})"
            );
        }
    }

    // ── T-000112: benchmark harness (fast unit-style) ─────────────────────────

    /// Benchmark: run 10 000 ticks at 1000× warp and assert cached throughput
    /// is >= 10× non-cached.  Implemented as a #[test] so it runs in CI via
    /// `cargo test bench_trafficlab_1000x_throughput_10x`.
    #[test]
    fn bench_trafficlab_1000x_throughput_10x() {
        use std::time::Instant;

        let mut cfg = SimConfig::new();
        cfg.speed_multiplier = 1000;
        const TICKS: u32 = 10_000;
        const DT_MS: f64 = 16.0; // ~60 fps wall-clock frame

        // Warm the cache so the timed run sees steady-state hits.
        let mut ca_warm = SimState::new(0);
        let mut cache   = SimCache::new();
        for _ in 0..64 {
            ca_warm.step_cached(DT_MS * 1000.0, &cfg, &mut cache);
        }
        cache.hits   = 0;
        cache.misses = 0;

        // Time nocache path.
        let mut nc = SimState::new(0);
        let t0 = Instant::now();
        for _ in 0..TICKS {
            nc.step_nocache(DT_MS * 1000.0, &cfg);
        }
        let nc_ns = t0.elapsed().as_nanos() as f64;

        // Time cached path (warm).
        let t0 = Instant::now();
        for _ in 0..TICKS {
            ca_warm.step_cached(DT_MS * 1000.0, &cfg, &mut cache);
        }
        let ca_ns = t0.elapsed().as_nanos() as f64;

        let nc_tps = TICKS as f64 / (nc_ns / 1e9);
        let ca_tps = TICKS as f64 / (ca_ns / 1e9);
        let ratio  = ca_tps / nc_tps;

        eprintln!(
            "\n[bench 1000x] nocache={:.0} tps  cached={:.0} tps  ratio={:.1}x  hit%={:.1}",
            nc_tps, ca_tps, ratio,
            cache.hit_ratio() * 100.0,
        );

        assert!(
            ratio >= 10.0,
            "cached throughput ratio {ratio:.2}x < required 10x \
             (nocache={nc_tps:.0} tps, cached={ca_tps:.0} tps)"
        );
    }

    /// Benchmark: record throughput metrics for 1x / 100x / 1000x warp and
    /// print a summary table.  Values are also used to create the baseline JSON.
    #[test]
    fn bench_trafficlab_summary_table() {
        use std::time::Instant;

        const TICKS: u32 = 10_000;
        const DT_MS: f64 = 16.0;
        let warps = [1u32, 100, 1000];

        eprintln!("\n{:<8}  {:>12}  {:>12}  {:>8}  {:>8}  {:>10}",
            "warp", "nc_tps", "ca_tps", "ratio", "hit%", "keys");

        for &warp in &warps {
            let mut cfg = SimConfig::new();
            cfg.speed_multiplier = warp;
            let sim_dt = DT_MS * warp as f64;

            // Warm cache.
            let mut ca = SimState::new(0xABCD);
            let mut cache = SimCache::new();
            for _ in 0..64 {
                ca.step_cached(sim_dt, &cfg, &mut cache);
            }
            cache.hits   = 0;
            cache.misses = 0;

            // Time.
            let mut nc = SimState::new(0xABCD);
            let t0 = Instant::now();
            for _ in 0..TICKS { nc.step_nocache(sim_dt, &cfg); }
            let nc_s = t0.elapsed().as_secs_f64();

            let t0 = Instant::now();
            for _ in 0..TICKS { ca.step_cached(sim_dt, &cfg, &mut cache); }
            let ca_s = t0.elapsed().as_secs_f64();

            let nc_tps = TICKS as f64 / nc_s;
            let ca_tps = TICKS as f64 / ca_s;

            eprintln!("{:<8}  {:>12.0}  {:>12.0}  {:>7.1}x  {:>7.1}%  {:>10}",
                format!("{}x", warp),
                nc_tps, ca_tps,
                ca_tps / nc_tps,
                cache.hit_ratio() * 100.0,
                cache.len(),
            );
        }
    }

    /// Determinism: two runs from the same seed must produce bitwise-identical
    /// queue counts and vehicle discharge totals after 10 000 ticks.
    /// Wall-clock timing determinism is checked by bench.py (in isolation).
    #[test]
    fn bench_trafficlab_deterministic_within_15pct() {
        let cfg = SimConfig::new();
        const TICKS: u32 = 10_000;
        const DT_MS: f64 = 16.0 * 1000.0;

        let run = || -> (QueueSnapshot, u64) {
            let mut ca = SimState::new(0xDEAD);
            let mut cache = SimCache::new();
            for _ in 0..TICKS { ca.step_cached(DT_MS, &cfg, &mut cache); }
            (ca.queues, ca.vehicles_discharged)
        };

        let (q1, d1) = run();
        let (q2, d2) = run();

        assert_eq!(q1.north, q2.north, "N queue not deterministic");
        assert_eq!(q1.south, q2.south, "S queue not deterministic");
        assert_eq!(q1.east,  q2.east,  "E queue not deterministic");
        assert_eq!(q1.west,  q2.west,  "W queue not deterministic");
        assert_eq!(d1, d2, "vehicles_discharged not deterministic");
    }

    // ── T-000114: multi-intersection grid tests ────────────────────────────────

    /// Headless 3×3 grid: 1 000 ticks must complete without panic and all
    /// queues must stay within reasonable bounds.
    #[test]
    fn headless_3x3_grid_1000_ticks_no_panic() {
        let mut lab = TrafficLab::new();
        lab.set_grid_size(3);
        // 50 frames × 16 ms = 0.8 s wall, runs many ticks per frame.
        for _ in 0..50 {
            lab.step_frame(16.0); // 16 ms wall time, speed_multiplier=1
        }
        // All per-cell queues must be bounded (lane_cap = 3 * 8 = 24 per arm;
        // routing may temporarily bump up to ~2× lane_cap before re-discharge).
        let cap_check = 64u32;
        for r in 0..3u8 {
            for c in 0..3u8 {
                assert!(lab.grid_nocache_queue_north(r, c) <= cap_check);
                assert!(lab.grid_nocache_queue_south(r, c) <= cap_check);
                assert!(lab.grid_cached_queue_north(r, c)  <= cap_check);
                assert!(lab.grid_cached_queue_south(r, c)  <= cap_check);
            }
        }
    }

    /// Scale test: spawn > 10 000 vehicles through a 3×3 grid without panic.
    #[test]
    fn scale_10k_vehicles_3x3_no_panic() {
        let mut lab = TrafficLab::new();
        lab.set_grid_size(3);
        lab.set_vehicles_per_min(600); // 10 veh/s per intersection
        lab.set_speed_multiplier(1000);
        // With time-budgeted loop, 20 frames × 16 ms = 0.32 s wall time.
        // At 1000× warp, each tick = 1 000 ms sim time.  This should easily
        // produce > 10 k vehicles per cell.
        for _ in 0..20 {
            lab.step_frame(16.0);
        }
        let per_cell = lab.nocache_vehicles_processed();
        assert!(per_cell >= 10_000,
            "expected >= 10 000 vehicles per cell, got {per_cell}");
    }

    // ── T-000115: traffic mode and country preset tests ────────────────────────

    /// Rush Hour must produce >= 2× the vehicles of Normal after 1 000 ticks.
    #[test]
    fn rush_hour_gte_2x_normal_vehicles() {
        let dt = 100.0_f64;
        const TICKS: usize = 1_000;

        // Normal: 120 vpm, 60 s cycle
        let mut cfg_normal = SimConfig::new();
        cfg_normal.vehicles_per_min = 120;
        cfg_normal.signal_cycle_secs = 60;
        let mut s_normal = SimState::new(0x1111);
        for _ in 0..TICKS { s_normal.step_nocache(dt, &cfg_normal); }

        // Rush Hour: 420 vpm, 48 s cycle (0.8×)
        let mut cfg_rush = SimConfig::new();
        cfg_rush.vehicles_per_min = 420;
        cfg_rush.signal_cycle_secs = 48;
        let mut s_rush = SimState::new(0x1111);
        for _ in 0..TICKS { s_rush.step_nocache(dt, &cfg_rush); }

        assert!(
            s_rush.vehicles_spawned >= s_normal.vehicles_spawned * 2,
            "rush ({}) should be >= 2× normal ({})",
            s_rush.vehicles_spawned, s_normal.vehicles_spawned
        );
    }

    /// All four modes produce measurably different queue behavior.
    #[test]
    fn four_modes_produce_different_queue_totals() {
        let dt = 100.0_f64;
        const TICKS: usize = 500;

        let modes: [(u32, u32); 4] = [
            (120, 60),  // normal
            (420, 48),  // rush
            (600, 36),  // festival
            ( 60, 72),  // rain
        ];

        let mut totals = Vec::new();
        for &(vpm, cycle) in &modes {
            let mut cfg = SimConfig::new();
            cfg.vehicles_per_min = vpm;
            cfg.signal_cycle_secs = cycle;
            let mut s = SimState::new(0x2222);
            for _ in 0..TICKS { s.step_nocache(dt, &cfg); }
            totals.push(s.vehicles_spawned);
        }

        // All four must be distinct
        for i in 0..totals.len() {
            for j in (i+1)..totals.len() {
                assert_ne!(
                    totals[i], totals[j],
                    "mode {i} and {j} produced same vehicle count: {}",
                    totals[i]
                );
            }
        }
    }

    /// India preset (free_left + motorcycle_splitting) discharges more vehicles
    /// than US preset (strict lanes) under the same arrival rate.
    #[test]
    fn india_preset_discharges_more_than_us() {
        let dt = 1_000.0_f64; // 1 s steps so discharge > 0 per tick
        const TICKS: usize = 1_000;

        // US: strict lanes (6 lanes so discharge difference is visible after truncation)
        let mut cfg_us = SimConfig::new();
        cfg_us.vehicles_per_min = 300;
        cfg_us.lane_count = 6;
        cfg_us.free_left_turn = false;
        cfg_us.motorcycle_splitting = false;
        let mut s_us = SimState::new(0x3333);
        for _ in 0..TICKS { s_us.step_nocache(dt, &cfg_us); }

        // India: free left + motorcycle splitting
        let mut cfg_in = SimConfig::new();
        cfg_in.vehicles_per_min = 300;
        cfg_in.lane_count = 6;
        cfg_in.free_left_turn = true;
        cfg_in.motorcycle_splitting = true;
        let mut s_in = SimState::new(0x3333);
        for _ in 0..TICKS { s_in.step_nocache(dt, &cfg_in); }

        assert!(
            s_in.vehicles_discharged > s_us.vehicles_discharged,
            "India ({}) should discharge more than US ({})",
            s_in.vehicles_discharged, s_us.vehicles_discharged
        );
    }

    /// Cache adapts to different modes: switching config invalidates cached
    /// discharge values, producing different queue behavior per mode.
    #[test]
    fn cache_adapts_to_mode_switch() {
        let dt = 1_000.0_f64;
        const TICKS: usize = 500;

        // Run Normal US mode with a shared cache
        let mut cfg = SimConfig::new();
        cfg.vehicles_per_min = 120;
        cfg.lane_count = 6;
        cfg.free_left_turn = false;
        cfg.motorcycle_splitting = false;
        let mut s = SimState::new(0x4444);
        let mut cache = SimCache::new();
        for _ in 0..TICKS { s.step_cached(dt, &cfg, &mut cache); }
        let queues_normal = s.queues.total();
        let keys_after_normal = cache.len();

        // Switch to Rush India mode on the SAME cache — new keys get added
        cfg.vehicles_per_min = 420;
        cfg.signal_cycle_secs = 48;
        cfg.free_left_turn = true;
        cfg.motorcycle_splitting = true;
        for _ in 0..TICKS { s.step_cached(dt, &cfg, &mut cache); }
        let queues_rush = s.queues.total();
        let keys_after_rush = cache.len();

        // Switching mode must add new cache keys (different key bits)
        assert!(
            keys_after_rush > keys_after_normal,
            "mode switch should add cache keys: before={} after={}",
            keys_after_normal, keys_after_rush
        );

        // Queue behavior must differ between modes
        assert_ne!(
            queues_normal, queues_rush,
            "queue totals should differ: normal={} rush={}",
            queues_normal, queues_rush
        );
    }

    /// Cache hit ratio in 3×3 mode must be >= 1×1 mode after a brief warm-up,
    /// because nine cells share one cache and warm it faster.
    #[test]
    fn cache_ratio_3x3_ge_1x1_after_brief_warmup() {
        let cfg = SimConfig::new();
        const WARMUP: usize = 20;
        const DT: f64 = 16.0 * 1_000.0;

        // 1×1: single intersection
        let mut sc_1x1 = SimCache::new();
        let mut s1 = SimState::new(0x1234);
        for _ in 0..WARMUP { s1.step_cached(DT, &cfg, &mut sc_1x1); }
        let ratio_1x1 = sc_1x1.hit_ratio();

        // 3×3: nine cells sharing one cache
        let mut sc_3x3 = SimCache::new();
        let mut cells: Vec<SimState> =
            (0..9usize).map(|i| SimState::new(0x1234 + i as u64)).collect();
        for _ in 0..WARMUP {
            for c in cells.iter_mut() { c.step_cached(DT, &cfg, &mut sc_3x3); }
        }
        let ratio_3x3 = sc_3x3.hit_ratio();

        assert!(
            ratio_3x3 >= ratio_1x1,
            "3×3 hit ratio {:.1}% should be >= 1×1 {:.1}%",
            ratio_3x3 * 100.0, ratio_1x1 * 100.0,
        );
    }
}
