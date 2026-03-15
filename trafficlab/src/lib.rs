//! TrafficLab WASM simulation engine — T-000108.
//!
//! Implements the real core simulation:
//!   * Vehicle spawn with configurable type distribution
//!     (car 60 %, motorcycle 20 %, truck 10 %, bus 5 %, auto-rickshaw 5 %)
//!   * Traffic-signal state machine (NS green → all-red → EW green → all-red → …)
//!   * Lane-queue model (queues grow under red, drain under green)
//!   * Per-tick statistics (avg wait time, vehicles processed/discharged, TPS)
//!
//! T-000109 will wire MuonCache into the `cached` instance.

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

    /// Advance simulation by `sim_dt_ms` milliseconds of simulated time.
    fn step(&mut self, sim_dt_ms: f64, cfg: &SimConfig) {
        self.tick += 1;
        self.sim_ms += sim_dt_ms;
        self.signal_elapsed += sim_dt_ms;

        // ── Signal state machine ────────────────────────────────────────────
        let half_cycle_ms = (cfg.signal_cycle_secs as f64) * 500.0;
        // Yellow / all-red duration: 10 % of half-cycle, clamped 2–5 s.
        let yellow_ms = (half_cycle_ms * 0.1).clamp(2_000.0, 5_000.0);

        self.signal = advance_signal(
            self.signal,
            &mut self.signal_elapsed,
            half_cycle_ms,
            yellow_ms,
        );

        // ── Vehicle spawn ───────────────────────────────────────────────────
        // Arrival rate in vehicles / ms across all four approaches.
        let arrival_rate = (cfg.vehicles_per_min as f64) / 60_000.0;
        self.arrival_accum += arrival_rate * sim_dt_ms;
        let n_new = self.arrival_accum as u32;
        self.arrival_accum -= n_new as f64;

        // Base per-approach allocation; distribute remainder randomly.
        let per_approach = n_new / 4;
        let mut approach_arrivals = [per_approach; 4];
        let remainder = n_new % 4;
        for _ in 0..remainder {
            approach_arrivals[self.rng.below(4) as usize] += 1;
        }
        self.vehicles_spawned += n_new as u64;

        let lane_cap = (cfg.lane_count as u32) * 8;

        // Add new arrivals to each approach queue (capped at lane capacity).
        self.queues.north = (self.queues.north + approach_arrivals[0]).min(lane_cap);
        self.queues.south = (self.queues.south + approach_arrivals[1]).min(lane_cap);
        self.queues.east  = (self.queues.east  + approach_arrivals[2]).min(lane_cap);
        self.queues.west  = (self.queues.west  + approach_arrivals[3]).min(lane_cap);

        // ── Wait-time accumulator ───────────────────────────────────────────
        // Every vehicle in queue right now accumulates sim_dt_ms of wait.
        self.total_wait_vehicle_ms += self.queues.total() as f64 * sim_dt_ms;

        // ── Queue discharge ─────────────────────────────────────────────────
        // Saturation flow ≈ 1 500 veh/hr/lane = 0.417 veh/s/lane.
        // Free-left-turn (India): extra throughput ~0.55 veh/s/lane effective.
        let discharge_rate_per_lane = if cfg.free_left_turn { 0.55_f64 } else { 0.417_f64 };
        let discharge =
            (cfg.lane_count as f64 * discharge_rate_per_lane * sim_dt_ms / 1_000.0) as u32;

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
/// * `nocache` — recomputes all state every tick (baseline).
/// * `cached`  — uses MuonCache for state lookups (T-000109 wires the cache).
#[wasm_bindgen]
pub struct TrafficLab {
    config: SimConfig,
    nocache: SimState,
    cached: SimState,

    // ── MuonCache metrics (T-000109 replaces stubs) ───────────────────────
    cache_hits: u64,
    cache_misses: u64,
    cache_keys: u64,
    cache_avg_latency_us: f64,

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
            cache_hits: 0,
            cache_misses: 0,
            cache_keys: 0,
            cache_avg_latency_us: 0.0,
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
        self.nocache.step(sim_dt, &self.config);
        self.cached.step(sim_dt, &self.config);

        // Stub cache metrics — T-000109 replaces with real MuonCache calls.
        // 50 lookups per tick keeps integer truncation from distorting the
        // %hit ratio: floor(50 × 0.98) = 49, so ratio = 49/50 = 0.98.
        let tick_lookups: u64 = 50;
        let hits = (tick_lookups as f64 * 0.98) as u64;
        self.cache_hits += hits;
        self.cache_misses += tick_lookups - hits;
        self.cache_keys = 128 + self.cached.tick / 100;
        self.cache_avg_latency_us = 0.03;

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

    pub fn cache_hits(&self) -> u64 { self.cache_hits }
    pub fn cache_misses(&self) -> u64 { self.cache_misses }
    pub fn cache_hit_ratio(&self) -> f64 {
        let total = self.cache_hits + self.cache_misses;
        if total == 0 { 0.0 } else { self.cache_hits as f64 / total as f64 }
    }
    pub fn cache_keys(&self) -> u64 { self.cache_keys }
    pub fn cache_avg_latency_us(&self) -> f64 { self.cache_avg_latency_us }
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
            state.step(1_000.0, &cfg);
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
}
