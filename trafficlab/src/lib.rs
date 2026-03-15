//! TrafficLab WASM entry point.
//!
//! Scaffold for T-000107: exposes the `TrafficLab` struct to JavaScript via
//! wasm-bindgen.  Simulation physics are stubbed here; T-000108 implements the
//! full engine, T-000109 wires MuonCache into it.

use wasm_bindgen::prelude::*;

// ── Configuration ─────────────────────────────────────────────────────────────

/// Simulation configuration passed from the JS control panel.
#[wasm_bindgen]
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

// ── Signal phase ───────────────────────────────────────────────────────────────

/// Current traffic-signal phase for one intersection.
#[wasm_bindgen]
#[derive(Clone, Copy, PartialEq)]
pub enum SignalPhase {
    /// North-South green, East-West red.
    NSGreen = 0,
    /// All red (yellow transition).
    AllRed = 1,
    /// East-West green, North-South red.
    EWGreen = 2,
}

// ── Main simulation struct ─────────────────────────────────────────────────────

/// TrafficLab: runs two simulation instances side-by-side.
///
/// * `nocache` — recomputes all state every tick (no cache).
/// * `cached`  — uses MuonCache for state lookups (T-000109 wires the cache).
///
/// For the scaffold (T-000107) both instances run the same stub loop so the
/// build pipeline and rendering shell can be proven out independently of the
/// physics engine (T-000108).
#[wasm_bindgen]
pub struct TrafficLab {
    config: SimConfig,

    // ── no-cache instance ──────────────────────────────────────────────────
    nocache_tick: u64,
    nocache_sim_ms: f64,   // simulated time elapsed (ms)
    nocache_vehicles: u64,
    nocache_queues: QueueSnapshot,
    nocache_signal: SignalPhase,
    nocache_signal_elapsed: f64,

    // ── cached instance ────────────────────────────────────────────────────
    cached_tick: u64,
    cached_sim_ms: f64,
    cached_vehicles: u64,
    cached_queues: QueueSnapshot,
    cached_signal: SignalPhase,
    cached_signal_elapsed: f64,

    // ── MuonCache metrics (populated by T-000109) ──────────────────────────
    cache_hits: u64,
    cache_misses: u64,
    cache_keys: u64,
    cache_avg_latency_us: f64,

    // ── internal timing ───────────────────────────────────────────────────
    /// Accumulated wall-clock ms since last tps sample.
    wall_ms_accum: f64,
    /// Tick count at last tps sample.
    tps_tick_snapshot: u64,
    /// Measured ticks-per-second for each instance.
    tps_nocache: f64,
    tps_cached: f64,
}

#[wasm_bindgen]
impl TrafficLab {
    // ── Construction / reset ───────────────────────────────────────────────

    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            config: SimConfig::new(),
            nocache_tick: 0,
            nocache_sim_ms: 0.0,
            nocache_vehicles: 0,
            nocache_queues: QueueSnapshot::default(),
            nocache_signal: SignalPhase::NSGreen,
            nocache_signal_elapsed: 0.0,
            cached_tick: 0,
            cached_sim_ms: 0.0,
            cached_vehicles: 0,
            cached_queues: QueueSnapshot::default(),
            cached_signal: SignalPhase::NSGreen,
            cached_signal_elapsed: 0.0,
            cache_hits: 0,
            cache_misses: 0,
            cache_keys: 0,
            cache_avg_latency_us: 0.0,
            wall_ms_accum: 0.0,
            tps_tick_snapshot: 0,
            tps_nocache: 0.0,
            tps_cached: 0.0,
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
        self.config = SimConfig {
            vehicles_per_min: self.config.vehicles_per_min,
            signal_cycle_secs: self.config.signal_cycle_secs,
            lane_count: self.config.lane_count,
            speed_multiplier: self.config.speed_multiplier,
            free_left_turn: self.config.free_left_turn,
        };
    }

    // ── Configuration setters ──────────────────────────────────────────────

    pub fn set_vehicles_per_min(&mut self, v: u32) {
        self.config.vehicles_per_min = v;
    }
    pub fn set_signal_cycle_secs(&mut self, s: u32) {
        self.config.signal_cycle_secs = s.max(10);
    }
    pub fn set_lane_count(&mut self, l: u8) {
        self.config.lane_count = l.max(1).min(6);
    }
    pub fn set_speed_multiplier(&mut self, s: u32) {
        self.config.speed_multiplier = match s {
            1 | 10 | 100 | 1000 => s,
            _ => 1,
        };
    }
    pub fn set_free_left_turn(&mut self, v: bool) {
        self.config.free_left_turn = v;
    }

    // ── Main step function (called each animation frame) ─────────────────

    /// Advance both simulations by `wall_dt_ms` wall-clock milliseconds.
    ///
    /// The speed multiplier converts wall time → simulated time.  In the
    /// scaffold both instances advance identically; T-000108 replaces this
    /// with real physics, and T-000109 makes the cached instance faster by
    /// skipping recomputation.
    pub fn step_frame(&mut self, wall_dt_ms: f64) {
        let sim_dt = wall_dt_ms * (self.config.speed_multiplier as f64);

        self.step_nocache(sim_dt);
        self.step_cached(sim_dt);

        // Measure TPS every ~500 ms of wall time.
        self.wall_ms_accum += wall_dt_ms;
        if self.wall_ms_accum >= 500.0 {
            let ticks_elapsed = self.cached_tick - self.tps_tick_snapshot;
            let secs = self.wall_ms_accum / 1000.0;
            self.tps_nocache = (self.nocache_tick - self.tps_tick_snapshot) as f64 / secs;
            self.tps_cached = ticks_elapsed as f64 / secs;
            self.tps_tick_snapshot = self.cached_tick;
            self.wall_ms_accum = 0.0;
        }
    }

    // ── Getters: no-cache instance ─────────────────────────────────────────

    pub fn nocache_tick(&self) -> u64 {
        self.nocache_tick
    }
    pub fn nocache_sim_seconds(&self) -> f64 {
        self.nocache_sim_ms / 1000.0
    }
    pub fn nocache_vehicles_processed(&self) -> u64 {
        self.nocache_vehicles
    }
    pub fn nocache_queue_north(&self) -> u32 {
        self.nocache_queues.north
    }
    pub fn nocache_queue_south(&self) -> u32 {
        self.nocache_queues.south
    }
    pub fn nocache_queue_east(&self) -> u32 {
        self.nocache_queues.east
    }
    pub fn nocache_queue_west(&self) -> u32 {
        self.nocache_queues.west
    }
    pub fn nocache_signal_phase(&self) -> u8 {
        self.nocache_signal as u8
    }
    pub fn tps_nocache(&self) -> f64 {
        self.tps_nocache
    }

    // ── Getters: cached instance ───────────────────────────────────────────

    pub fn cached_tick(&self) -> u64 {
        self.cached_tick
    }
    pub fn cached_sim_seconds(&self) -> f64 {
        self.cached_sim_ms / 1000.0
    }
    pub fn cached_vehicles_processed(&self) -> u64 {
        self.cached_vehicles
    }
    pub fn cached_queue_north(&self) -> u32 {
        self.cached_queues.north
    }
    pub fn cached_queue_south(&self) -> u32 {
        self.cached_queues.south
    }
    pub fn cached_queue_east(&self) -> u32 {
        self.cached_queues.east
    }
    pub fn cached_queue_west(&self) -> u32 {
        self.cached_queues.west
    }
    pub fn cached_signal_phase(&self) -> u8 {
        self.cached_signal as u8
    }
    pub fn tps_cached(&self) -> f64 {
        self.tps_cached
    }

    // ── Getters: MuonCache metrics ─────────────────────────────────────────

    pub fn cache_hits(&self) -> u64 {
        self.cache_hits
    }
    pub fn cache_misses(&self) -> u64 {
        self.cache_misses
    }
    pub fn cache_hit_ratio(&self) -> f64 {
        let total = self.cache_hits + self.cache_misses;
        if total == 0 {
            0.0
        } else {
            self.cache_hits as f64 / total as f64
        }
    }
    pub fn cache_keys(&self) -> u64 {
        self.cache_keys
    }
    pub fn cache_avg_latency_us(&self) -> f64 {
        self.cache_avg_latency_us
    }
}

// ── Private simulation helpers ─────────────────────────────────────────────────

impl TrafficLab {
    /// Stub no-cache tick: advances signal state machine and queue model.
    /// T-000108 replaces this with the full vehicle spawn + movement engine.
    fn step_nocache(&mut self, sim_dt_ms: f64) {
        self.nocache_tick += 1;
        self.nocache_sim_ms += sim_dt_ms;
        self.nocache_signal_elapsed += sim_dt_ms;

        let half_cycle_ms = (self.config.signal_cycle_secs as f64) * 500.0;
        let yellow_ms = 2_000.0;

        self.nocache_signal = Self::advance_signal(
            self.nocache_signal,
            &mut self.nocache_signal_elapsed,
            half_cycle_ms,
            yellow_ms,
        );

        // Stub queue dynamics: grow under red, drain under green.
        let lane_cap = (self.config.lane_count as u32) * 8;
        let arrival_per_tick = (self.config.vehicles_per_min as f64) / 60_000.0 * sim_dt_ms;
        let n_arrivals = arrival_per_tick.round() as u32;
        self.nocache_queues = Self::update_queues(
            self.nocache_queues,
            self.nocache_signal,
            n_arrivals,
            lane_cap,
            self.config.free_left_turn,
        );
        self.nocache_vehicles += n_arrivals as u64;
    }

    /// Stub cached tick: same stub as no-cache for now.
    /// T-000109 replaces this with cache-backed state lookups that skip
    /// the expensive recomputation, making cached_tick advance faster.
    fn step_cached(&mut self, sim_dt_ms: f64) {
        self.cached_tick += 1;
        self.cached_sim_ms += sim_dt_ms;
        self.cached_signal_elapsed += sim_dt_ms;

        let half_cycle_ms = (self.config.signal_cycle_secs as f64) * 500.0;
        let yellow_ms = 2_000.0;

        self.cached_signal = Self::advance_signal(
            self.cached_signal,
            &mut self.cached_signal_elapsed,
            half_cycle_ms,
            yellow_ms,
        );

        let lane_cap = (self.config.lane_count as u32) * 8;
        let arrival_per_tick = (self.config.vehicles_per_min as f64) / 60_000.0 * sim_dt_ms;
        let n_arrivals = arrival_per_tick.round() as u32;
        self.cached_queues = Self::update_queues(
            self.cached_queues,
            self.cached_signal,
            n_arrivals,
            lane_cap,
            self.config.free_left_turn,
        );
        self.cached_vehicles += n_arrivals as u64;

        // Stub cache metrics: simulate warm cache with 98% hit ratio.
        // T-000109 replaces with real MuonCache lookups.
        let lookups = n_arrivals.max(1) as u64 * 4;
        let hits = (lookups as f64 * 0.98) as u64;
        self.cache_hits += hits;
        self.cache_misses += lookups - hits;
        self.cache_keys = 128 + (self.cached_tick / 100);
        self.cache_avg_latency_us = 0.03;
    }

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

    fn update_queues(
        mut q: QueueSnapshot,
        signal: SignalPhase,
        arrivals: u32,
        lane_cap: u32,
        free_left: bool,
    ) -> QueueSnapshot {
        // Distribute arrivals equally across four approaches.
        let per_approach = arrivals / 4;
        let drain_rate = if free_left { 4 } else { 3 };

        match signal {
            SignalPhase::NSGreen => {
                // N+S drain; E+W accumulate.
                q.north = q.north.saturating_sub(drain_rate) + per_approach;
                q.south = q.south.saturating_sub(drain_rate) + per_approach;
                q.east = (q.east + per_approach).min(lane_cap);
                q.west = (q.west + per_approach).min(lane_cap);
            }
            SignalPhase::EWGreen => {
                // E+W drain; N+S accumulate.
                q.east = q.east.saturating_sub(drain_rate) + per_approach;
                q.west = q.west.saturating_sub(drain_rate) + per_approach;
                q.north = (q.north + per_approach).min(lane_cap);
                q.south = (q.south + per_approach).min(lane_cap);
            }
            SignalPhase::AllRed => {
                // All accumulate during yellow.
                q.north = (q.north + per_approach).min(lane_cap);
                q.south = (q.south + per_approach).min(lane_cap);
                q.east = (q.east + per_approach).min(lane_cap);
                q.west = (q.west + per_approach).min(lane_cap);
            }
        }
        q
    }
}
