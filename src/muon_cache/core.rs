//! Socketless muoncache core command executor for embedded/WASM use.

use std::collections::{HashMap, VecDeque};

#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

use crate::muon_cache::store::Db;

#[cfg(feature = "muoncache-wasm")]
use wasm_bindgen::prelude::*;

#[cfg(feature = "muoncache-wasm")]
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = performance, js_name = now)]
    fn perf_now() -> f64;
}

const WRONGTYPE: &str = "WRONGTYPE Operation against a key holding the wrong kind of value";
const ERR_HASH_INT: &str = "ERR hash value is not an integer";
const MAX_LATENCY_SAMPLES: usize = 4096;

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "muoncache-wasm", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "muoncache-wasm", serde(rename_all = "snake_case", tag = "kind"))]
pub enum CoreCommand {
    Set {
        key: String,
        value: String,
    },
    Get {
        key: String,
    },
    Hset {
        key: String,
        field: String,
        value: String,
    },
    Hget {
        key: String,
        field: String,
    },
    Hincrby {
        key: String,
        field: String,
        delta: i64,
    },
    Zadd {
        key: String,
        score: f64,
        member: String,
    },
    Zrange {
        key: String,
        start: i64,
        stop: i64,
    },
    Zcard {
        key: String,
    },
    Del {
        keys: Vec<String>,
    },
    Flushdb,
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "muoncache-wasm", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "muoncache-wasm", serde(untagged))]
pub enum CoreData {
    Null,
    Integer(i64),
    String(String),
    Strings(Vec<String>),
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "muoncache-wasm", derive(serde::Serialize, serde::Deserialize))]
pub struct CoreResponse {
    pub ok: bool,
    #[cfg_attr(feature = "muoncache-wasm", serde(skip_serializing_if = "Option::is_none"))]
    pub data: Option<CoreData>,
    #[cfg_attr(feature = "muoncache-wasm", serde(skip_serializing_if = "Option::is_none"))]
    pub error: Option<String>,
}

impl CoreResponse {
    fn ok(data: CoreData) -> Self {
        Self {
            ok: true,
            data: Some(data),
            error: None,
        }
    }

    fn err(msg: &str) -> Self {
        Self {
            ok: false,
            data: None,
            error: Some(msg.to_string()),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "muoncache-wasm", derive(serde::Serialize, serde::Deserialize))]
pub struct CoreMetricsSnapshot {
    pub ops_total: u64,
    pub ops_window_1s: u64,
    pub batch_size_avg: f64,
    pub latency_p50_us: u64,
    pub latency_p95_us: u64,
    pub latency_p99_us: u64,
    pub queue_depth: u32,
    pub errors_total: u64,
    pub command_mix: HashMap<String, u64>,
}

#[derive(Default)]
struct CoreMetrics {
    ops_total: u64,
    errors_total: u64,
    command_mix: HashMap<String, u64>,
    op_timestamps_us: VecDeque<u64>,
    latencies_us: VecDeque<u32>,
    queue_depth: u32,
    batch_ops_total: u64,
    batch_count: u64,
}

impl CoreMetrics {
    fn record_batch(&mut self, size: usize) {
        if size > 0 {
            self.batch_ops_total += size as u64;
            self.batch_count += 1;
        }
    }

    fn record_op(&mut self, now_us: u64, command: &str, latency_us: u64, is_error: bool) {
        self.ops_total += 1;
        if is_error {
            self.errors_total += 1;
        }
        *self.command_mix.entry(command.to_string()).or_insert(0) += 1;
        self.op_timestamps_us.push_back(now_us);

        let cutoff = now_us.saturating_sub(1_000_000);
        while let Some(front) = self.op_timestamps_us.front().copied() {
            if front >= cutoff {
                break;
            }
            self.op_timestamps_us.pop_front();
        }

        self.latencies_us
            .push_back(latency_us.min(u32::MAX as u64) as u32);
        while self.latencies_us.len() > MAX_LATENCY_SAMPLES {
            self.latencies_us.pop_front();
        }
    }

    fn snapshot(&self) -> CoreMetricsSnapshot {
        let mut latencies: Vec<u32> = self.latencies_us.iter().copied().collect();
        latencies.sort_unstable();

        let p50 = percentile(&latencies, 0.50);
        let p95 = percentile(&latencies, 0.95);
        let p99 = percentile(&latencies, 0.99);

        CoreMetricsSnapshot {
            ops_total: self.ops_total,
            ops_window_1s: self.op_timestamps_us.len() as u64,
            batch_size_avg: if self.batch_count > 0 {
                self.batch_ops_total as f64 / self.batch_count as f64
            } else {
                0.0
            },
            latency_p50_us: p50,
            latency_p95_us: p95,
            latency_p99_us: p99,
            queue_depth: self.queue_depth,
            errors_total: self.errors_total,
            command_mix: self.command_mix.clone(),
        }
    }
}

fn percentile(samples: &[u32], p: f64) -> u64 {
    if samples.is_empty() {
        return 0;
    }
    let idx = ((samples.len() - 1) as f64 * p).round() as usize;
    samples[idx.min(samples.len() - 1)] as u64
}

pub struct CoreExecutor {
    dbs: Vec<Db>,
    current_db: usize,
    metrics: CoreMetrics,
    #[cfg(not(target_arch = "wasm32"))]
    start: Instant,
}

impl CoreExecutor {
    pub fn new(databases: usize) -> Self {
        let count = databases.max(1);
        let mut dbs = Vec::with_capacity(count);
        for _ in 0..count {
            dbs.push(Db::new());
        }
        Self {
            dbs,
            current_db: 0,
            metrics: CoreMetrics::default(),
            #[cfg(not(target_arch = "wasm32"))]
            start: Instant::now(),
        }
    }

    pub fn reset(&mut self) {
        let db_count = self.dbs.len().max(1);
        self.dbs.clear();
        self.dbs.reserve(db_count);
        for _ in 0..db_count {
            self.dbs.push(Db::new());
        }
        self.current_db = 0;
        self.metrics = CoreMetrics::default();
        self.reset_clock();
    }

    pub fn set_queue_depth(&mut self, depth: u32) {
        self.metrics.queue_depth = depth;
    }

    pub fn metrics_snapshot(&self) -> CoreMetricsSnapshot {
        self.metrics.snapshot()
    }

    pub fn execute_batch(&mut self, cmds: &[CoreCommand]) -> Vec<CoreResponse> {
        self.metrics.record_batch(cmds.len());
        let batch_start = self.now_us();
        let mut out = Vec::with_capacity(cmds.len());
        let mut names: Vec<(&'static str, bool)> = Vec::with_capacity(cmds.len());
        for cmd in cmds {
            let (resp, name) = self.execute_inner(cmd);
            names.push((name, !resp.ok));
            out.push(resp);
        }
        let batch_elapsed = self.now_us().saturating_sub(batch_start);
        let per_cmd_us = if cmds.is_empty() {
            0
        } else {
            batch_elapsed / cmds.len() as u64
        };
        let now = self.now_us();
        for (name, is_error) in names {
            self.metrics.record_op(now, name, per_cmd_us, is_error);
        }
        out
    }

    pub fn execute(&mut self, cmd: &CoreCommand) -> CoreResponse {
        let start_us = self.now_us();
        let (resp, name) = self.execute_inner(cmd);
        let elapsed_us = self.now_us().saturating_sub(start_us);
        self.metrics
            .record_op(self.now_us(), name, elapsed_us, !resp.ok);
        resp
    }

    fn execute_inner(&mut self, cmd: &CoreCommand) -> (CoreResponse, &'static str) {
        let db = &self.dbs[self.current_db];

        match cmd {
            CoreCommand::Set { key, value } => {
                db.set_string_from_slices(key.as_bytes(), value.as_bytes(), None);
                (CoreResponse::ok(CoreData::String("OK".to_string())), "SET")
            }
            CoreCommand::Get { key } => {
                let resp = match db.get_string(key.as_bytes()) {
                    Ok(Some(v)) => {
                        CoreResponse::ok(CoreData::String(String::from_utf8_lossy(v.as_ref()).to_string()))
                    }
                    Ok(None) => CoreResponse::ok(CoreData::Null),
                    Err(_) => CoreResponse::err(WRONGTYPE),
                };
                (resp, "GET")
            }
            CoreCommand::Hset { key, field, value } => {
                let resp = match db.hash_set_bytes(key.as_bytes(), field.as_bytes(), value.as_bytes()) {
                    Ok(inserted) => CoreResponse::ok(CoreData::Integer(if inserted { 1 } else { 0 })),
                    Err(_) => CoreResponse::err(WRONGTYPE),
                };
                (resp, "HSET")
            }
            CoreCommand::Hget { key, field } => {
                let resp = match db.hash_get(key.as_bytes(), field.as_bytes()) {
                    Ok(Some(v)) => {
                        CoreResponse::ok(CoreData::String(String::from_utf8_lossy(v.as_ref()).to_string()))
                    }
                    Ok(None) => CoreResponse::ok(CoreData::Null),
                    Err(_) => CoreResponse::err(WRONGTYPE),
                };
                (resp, "HGET")
            }
            CoreCommand::Hincrby { key, field, delta } => {
                let resp = match db.hash_incr_by(key.as_bytes(), field.as_bytes(), *delta) {
                    Ok(value) => CoreResponse::ok(CoreData::Integer(value)),
                    Err(_) => CoreResponse::err(ERR_HASH_INT),
                };
                (resp, "HINCRBY")
            }
            CoreCommand::Zadd { key, score, member } => {
                let resp = match db.zadd(key.as_bytes(), *score, member.as_bytes().to_vec()) {
                    Ok(inserted) => CoreResponse::ok(CoreData::Integer(if inserted { 1 } else { 0 })),
                    Err(_) => CoreResponse::err(WRONGTYPE),
                };
                (resp, "ZADD")
            }
            CoreCommand::Zrange { key, start, stop } => {
                let resp = match db.zrange(key.as_bytes(), *start, *stop) {
                    Ok(items) => CoreResponse::ok(CoreData::Strings(
                        items
                            .iter()
                            .map(|v| String::from_utf8_lossy(v).to_string())
                            .collect(),
                    )),
                    Err(_) => CoreResponse::err(WRONGTYPE),
                };
                (resp, "ZRANGE")
            }
            CoreCommand::Zcard { key } => {
                let resp = match db.zcard(key.as_bytes()) {
                    Ok(card) => CoreResponse::ok(CoreData::Integer(card)),
                    Err(_) => CoreResponse::err(WRONGTYPE),
                };
                (resp, "ZCARD")
            }
            CoreCommand::Del { keys } => {
                let mut removed = 0i64;
                for key in keys {
                    if db.remove(key.as_bytes()) {
                        removed += 1;
                    }
                }
                (CoreResponse::ok(CoreData::Integer(removed)), "DEL")
            }
            CoreCommand::Flushdb => {
                db.flush();
                (CoreResponse::ok(CoreData::String("OK".to_string())), "FLUSHDB")
            }
        }
    }

    fn now_us(&self) -> u64 {
        #[cfg(target_arch = "wasm32")]
        {
            // performance.now() returns DOMHighResTimeStamp in milliseconds
            // with microsecond precision, available in both window and worker scopes.
            (perf_now() * 1_000.0) as u64
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
        self.start.elapsed().as_micros() as u64
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn reset_clock(&mut self) {
        self.start = Instant::now();
    }

    #[cfg(target_arch = "wasm32")]
    fn reset_clock(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::{CoreCommand, CoreData, CoreExecutor};

    #[test]
    fn command_correctness_subset() {
        let mut core = CoreExecutor::new(1);

        let set = core.execute(&CoreCommand::Set {
            key: "player:1".to_string(),
            value: "10".to_string(),
        });
        assert!(set.ok);

        let get = core.execute(&CoreCommand::Get {
            key: "player:1".to_string(),
        });
        assert_eq!(get.data, Some(CoreData::String("10".to_string())));

        let hset = core.execute(&CoreCommand::Hset {
            key: "profile:1".to_string(),
            field: "name".to_string(),
            value: "alice".to_string(),
        });
        assert_eq!(hset.data, Some(CoreData::Integer(1)));

        let hget = core.execute(&CoreCommand::Hget {
            key: "profile:1".to_string(),
            field: "name".to_string(),
        });
        assert_eq!(hget.data, Some(CoreData::String("alice".to_string())));

        let zadd = core.execute(&CoreCommand::Zadd {
            key: "lb".to_string(),
            score: 100.0,
            member: "player:1".to_string(),
        });
        assert_eq!(zadd.data, Some(CoreData::Integer(1)));

        let zrange = core.execute(&CoreCommand::Zrange {
            key: "lb".to_string(),
            start: 0,
            stop: -1,
        });
        assert_eq!(
            zrange.data,
            Some(CoreData::Strings(vec!["player:1".to_string()]))
        );
    }

    #[test]
    fn type_mismatch_errors() {
        let mut core = CoreExecutor::new(1);
        let _ = core.execute(&CoreCommand::Set {
            key: "same".to_string(),
            value: "x".to_string(),
        });
        let bad = core.execute(&CoreCommand::Hset {
            key: "same".to_string(),
            field: "f".to_string(),
            value: "v".to_string(),
        });
        assert!(!bad.ok);
        assert!(bad.error.unwrap_or_default().contains("WRONGTYPE"));
    }

    #[test]
    fn batch_ordering_and_metrics() {
        let mut core = CoreExecutor::new(1);
        let batch = vec![
            CoreCommand::Set {
                key: "k1".to_string(),
                value: "v1".to_string(),
            },
            CoreCommand::Get {
                key: "k1".to_string(),
            },
            CoreCommand::Del {
                keys: vec!["k1".to_string()],
            },
            CoreCommand::Get {
                key: "k1".to_string(),
            },
        ];

        let out = core.execute_batch(&batch);
        assert_eq!(out.len(), 4);
        assert_eq!(out[1].data, Some(CoreData::String("v1".to_string())));
        assert_eq!(out[2].data, Some(CoreData::Integer(1)));
        assert_eq!(out[3].data, Some(CoreData::Null));

        let metrics = core.metrics_snapshot();
        assert_eq!(metrics.ops_total, 4);
        assert!(metrics.batch_size_avg >= 4.0);
    }
}
