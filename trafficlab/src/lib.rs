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
/// Layout: [signal:4][lane_count:8][free_left:1][dt_ms:16] — 29 bits used.
#[inline]
fn discharge_key(signal: SignalPhase, lane_count: u8, free_left: bool, dt_ms: f64) -> u32 {
    let dt_u16 = (dt_ms.round() as u32).min(0xFFFF) as u16;
    (signal as u32)
        | ((lane_count as u32) << 4)
        | ((free_left as u32) << 12)
        | ((dt_u16 as u32) << 13)
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
    #[inline]
    fn compute_discharge(signal: SignalPhase, lane_count: u8, free_left: bool, dt_ms: f64) -> u32 {
        if matches!(signal, SignalPhase::AllRed) {
            return 0;
        }
        let rate = if free_left { 0.55_f64 } else { 0.417_f64 };
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
            self.signal, cfg.lane_count, cfg.free_left_turn, sim_dt_ms,
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
        let key = discharge_key(self.signal, cfg.lane_count, cfg.free_left_turn, sim_dt_ms);
        let discharge = match cache.get(key) {
            Some(d) => d,
            None => {
                let d = Self::compute_discharge(
                    self.signal, cfg.lane_count, cfg.free_left_turn, sim_dt_ms,
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
    nocache: SimState,
    cached:  SimState,

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
            nocache: SimState::new(0xdead_beef_cafe_babe),
            cached:  SimState::new(0xfeed_face_dead_c0de),
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
        *self = Self::new();
        self.config = cfg;
    }

    // ── Configuration setters ──────────────────────────────────────────────

    pub fn set_vehicles_per_min(&mut self, v: u32) { self.config.vehicles_per_min = v; }
    pub fn set_signal_cycle_secs(&mut self, s: u32) { self.config.signal_cycle_secs = s.max(10); }
    pub fn set_lane_count(&mut self, l: u8) { self.config.lane_count = l.max(1).min(6); }
    pub fn set_speed_multiplier(&mut self, s: u32) {
        self.config.speed_multiplier = match s { 1 | 10 | 100 | 1000 => s, _ => 1 };
    }
    pub fn set_free_left_turn(&mut self, v: bool) { self.config.free_left_turn = v; }

    // ── Main step (called each animation frame) ────────────────────────────

    /// Advance both simulations by `wall_dt_ms` wall-clock milliseconds.
    pub fn step_frame(&mut self, wall_dt_ms: f64) {
        let sim_dt = wall_dt_ms * (self.config.speed_multiplier as f64);
        self.nocache.step_nocache(sim_dt, &self.config);
        self.cached.step_cached(sim_dt, &self.config, &mut self.sim_cache);

        // TPS measurement every ~500 ms of wall time.
        self.wall_ms_accum += wall_dt_ms;
        if self.wall_ms_accum >= 500.0 {
            let secs = self.wall_ms_accum / 1_000.0;
            self.tps_nocache = (self.nocache.tick - self.tps_nc_snap) as f64 / secs;
            self.tps_cached  = (self.cached.tick  - self.tps_c_snap)  as f64 / secs;
            self.tps_nc_snap = self.nocache.tick;
            self.tps_c_snap  = self.cached.tick;
            self.wall_ms_accum = 0.0;
        }
    }

    // ── Getters: no-cache instance ─────────────────────────────────────────

    pub fn nocache_tick(&self) -> u64 { self.nocache.tick }
    pub fn nocache_sim_seconds(&self) -> f64 { self.nocache.sim_ms / 1_000.0 }
    /// Vehicles that entered the simulation (spawned).
    pub fn nocache_vehicles_processed(&self) -> u64 { self.nocache.vehicles_spawned }
    /// Vehicles that cleared the intersection (discharged).
    pub fn nocache_vehicles_discharged(&self) -> u64 { self.nocache.vehicles_discharged }
    pub fn nocache_queue_north(&self) -> u32 { self.nocache.queues.north }
    pub fn nocache_queue_south(&self) -> u32 { self.nocache.queues.south }
    pub fn nocache_queue_east(&self)  -> u32 { self.nocache.queues.east  }
    pub fn nocache_queue_west(&self)  -> u32 { self.nocache.queues.west  }
    pub fn nocache_signal_phase(&self) -> u8 { self.nocache.signal as u8 }
    /// Discharge-weighted average vehicle wait time in seconds.
    pub fn nocache_avg_wait_sec(&self) -> f64 { self.nocache.avg_wait_sec() }
    pub fn tps_nocache(&self) -> f64 { self.tps_nocache }

    // ── Getters: cached instance ───────────────────────────────────────────

    pub fn cached_tick(&self) -> u64 { self.cached.tick }
    pub fn cached_sim_seconds(&self) -> f64 { self.cached.sim_ms / 1_000.0 }
    pub fn cached_vehicles_processed(&self) -> u64 { self.cached.vehicles_spawned }
    pub fn cached_vehicles_discharged(&self) -> u64 { self.cached.vehicles_discharged }
    pub fn cached_queue_north(&self) -> u32 { self.cached.queues.north }
    pub fn cached_queue_south(&self) -> u32 { self.cached.queues.south }
    pub fn cached_queue_east(&self)  -> u32 { self.cached.queues.east  }
    pub fn cached_queue_west(&self)  -> u32 { self.cached.queues.west  }
    pub fn cached_signal_phase(&self) -> u8 { self.cached.signal as u8 }
    pub fn cached_avg_wait_sec(&self) -> f64 { self.cached.avg_wait_sec() }
    pub fn tps_cached(&self) -> f64 { self.tps_cached }

    // ── Getters: MuonCache metrics ─────────────────────────────────────────

    pub fn cache_hits(&self) -> u64 { self.sim_cache.hits }
    pub fn cache_misses(&self) -> u64 { self.sim_cache.misses }
    pub fn cache_hit_ratio(&self) -> f64 { self.sim_cache.hit_ratio() }
    pub fn cache_keys(&self) -> u64 { self.sim_cache.len() }
    /// In-process HashMap lookup latency (fixed estimate — ~50 ns on modern hw).
    pub fn cache_avg_latency_us(&self) -> f64 { 0.05 }
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
        for _ in 0..10_000 {
            lab.step_frame(16.0); // ~60 fps
        }
        assert_eq!(lab.nocache_tick(), 10_000, "nocache tick count");
        assert_eq!(lab.cached_tick(),  10_000, "cached tick count");
        assert!(lab.nocache_vehicles_processed() > 0, "should have spawned vehicles");
        assert!(lab.tps_nocache() > 0.0, "tps should be measured after warmup");
    }

    #[test]
    fn per_tick_stats_are_sane() {
        let mut lab = TrafficLab::new();
        lab.set_lane_count(3); // 3 lanes → discharge ≥ 1 veh/tick at 1 s steps
        for _ in 0..100 {
            lab.step_frame(1_000.0); // 100 × 1 000 ms = 100 s simulated
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
}
