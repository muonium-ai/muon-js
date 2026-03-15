export type MuonCacheCommand =
  | { kind: "set"; key: string; value: string }
  | { kind: "get"; key: string }
  | { kind: "hset"; key: string; field: string; value: string }
  | { kind: "hget"; key: string; field: string }
  | { kind: "hincrby"; key: string; field: string; delta: number }
  | { kind: "zadd"; key: string; score: number; member: string }
  | { kind: "zrange"; key: string; start: number; stop: number }
  | { kind: "zcard"; key: string }
  | { kind: "del"; keys: string[] }
  | { kind: "flushdb" };

export type WorkerRequest =
  | { id: number; kind: "exec"; command: MuonCacheCommand }
  | { id: number; kind: "batch"; commands: MuonCacheCommand[] }
  | { id: number; kind: "metrics" }
  | { id: number; kind: "reset" }
  | { id: number; kind: "js_eval"; source: string };

export type WorkerResponse =
  | { id: number; ok: true; data: unknown }
  | { id: number; ok: false; error: string };

export type WorkerPush = {
  kind: "metrics_push";
  data: MuonCacheMetrics;
};

export type MuonCacheMetrics = {
  ops_total: number;
  ops_window_1s: number;
  batch_size_avg: number;
  latency_p50_us: number;
  latency_p95_us: number;
  latency_p99_us: number;
  queue_depth: number;
  errors_total: number;
  command_mix: Record<string, number>;
};

export type CoreResponse = {
  ok: boolean;
  data?: unknown;
  error?: string;
};
