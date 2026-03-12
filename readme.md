# Muon JS

Muon JS is a tiny, embeddable JavaScript runtime written in Rust, implemented as a true port of [mquickjs](https://github.com/bellard/quickjs) semantics and API shape (not a C wrapper/FFI shim).

## Features

- Native Rust runtime with mquickjs-style C API surface (`JS_NewContext`, `JS_Eval`, `JS_GetProperty*`, etc.).
- Bytecode compiler + VM path (`Compiler`, `VM`) for hot execution loops.
- Minimal default build (`default = []`) for embedders.
- Optional `mini-redis` server implementation over RESP.
- Optional `mini-redis-libsql` persistence backend for snapshots/AOF.
- Built-in benchmark workflows for:
  - Redis vs mini-redis command throughput.
  - Redis Lua vs mini-redis MuonJS scripting performance.
  - JS runtime microbench regression checks.

## Cargo Features

From `Cargo.toml`:

- `mini-redis`: enables the RESP server binary (`tokio`, `ctrlc`, `mimalloc`).
- `mini-redis-libsql`: enables persistent storage (`libsql`, `crossbeam-channel`).

Examples:

```bash
# Library-only (minimal)
cargo build

# mini-redis server
cargo build --features mini-redis

# mini-redis + persistence
cargo build --features "mini-redis mini-redis-libsql"
```

## Build And Test

Preferred workflow is through `Makefile` targets:

```bash
make build
make test
make release
```

Compatibility/integration targets:

```bash
make test-integration
make test-mquickjs
make test-mquickjs-detailed
make test-all
```

## Runtime Usage (Embedding)

Minimal example using the public API:

```rust
use muon_js::{
    JSCStringBuf, JS_EVAL_RETVAL, JS_Eval, JS_NewContext, JS_ToCString, JS_ToString,
};

fn main() {
    let mut mem = vec![0u8; 64 * 1024];
    let mut ctx = JS_NewContext(&mut mem);

    let val = JS_Eval(&mut ctx, "1 + 2", "main.js", JS_EVAL_RETVAL);
    let s = JS_ToString(&mut ctx, val);

    let mut buf = JSCStringBuf { buf: [0u8; 5] };
    let out = JS_ToCString(&mut ctx, s, &mut buf);
    println!("{out}");
}
```

## Mini-Redis

> **Release note:** mini-redis is being extracted into a standalone product. The
> implementation here is the reference codebase for that release. The sections
> below describe the current architecture, testing, and performance baselines.

### Architecture

The server lives in `src/mini_redis/` and is built as an optional feature-gated
binary. Module layout after the T-000078 / T-000083 refactoring:

| Module | Responsibility |
|--------|---------------|
| `resp.rs` | RESP protocol encode/decode |
| `server.rs` | async accept loop, connection state, scripting bridge |
| `server/handle_command.rs` | command dispatch and routing |
| `server/handle_no_db_command.rs` | server-level commands (`INFO`, `CONFIG`, `FUNCTION`, …) |
| `store.rs` | in-memory data structures and command semantics |
| `persist.rs` | persistence abstraction and libsql-backed durability |

### Supported command families

- **Core/control**: `PING`, `ECHO`, `INFO`, `SELECT`, `DBSIZE`, `QUIT`, `MULTI/EXEC/DISCARD`.
- **Strings**: `GET`, `SET`, `SETNX`, `MSET`, `MGET`, `GETSET`, `APPEND`, `INCR/DECR`, `INCRBY/DECRBY`, `STRLEN`.
- **Hashes**: `HSET`, `HGET`, `HDEL`, `HGETALL`, `HLEN`, `HEXISTS`, `HINCRBY`, `HSETNX`.
- **Lists**: `LPUSH/RPUSH`, `LPOP/RPOP`, `LRANGE`, `LLEN`, `LINDEX`, `LSET`, `LINSERT`, `LREM`, `LPUSHX/RPUSHX`, `LTRIM`.
- **Sets**: `SADD`, `SREM`, `SMEMBERS`, `SISMEMBER`, `SCARD`, `SMOVE`, `SUNION`, `SINTER`.
- **Sorted sets**: `ZADD`, `ZRANGE`, `ZREM`, `ZCARD`.
- **Streams**: `XADD`, `XRANGE`, `XREVRANGE`, `XLEN`, `XDEL`.
- **Keyspace/TTL**: `DEL`, `EXISTS`, `EXPIRE`, `PEXPIRE`, `PERSIST`, `TTL`, `PTTL`, `TYPE`, `KEYS`, `SCAN`, `FLUSHDB`, `FLUSHALL`.
- **Pub/Sub**: `SUBSCRIBE`, `PUBLISH`.
- **Scripting/admin**: `EVAL`, `EVALSHA`, `SCRIPT`, `FUNCTION LOAD/LIST/DELETE/FLUSH`, `CONFIG`, `CLIENT`, `SLOWLOG`, `SAVE`, `BGSAVE`, `REPLICAOF`.

### Scripting (MuonJS)

mini-redis uses the embedded MuonJS engine (this runtime) as its scripting layer:

- `EVAL script numkeys ...` — runs a JS snippet with `redis`, `KEYS`, `ARGV` bound; 1-based indexing matching the Redis/Lua convention.
- `FUNCTION LOAD "#!lua name=lib\n..."` — loads a named function library; the `#!lua` shebang is stripped before execution, accepting the same body format as Redis 7+.
- `CLIENT SETNAME`, `SCRIPT FLUSH`, `FUNCTION LIST/DELETE/FLUSH` are supported.
- Memory limits are configurable via `--script-mem` (bytes) and `--script-reset-threshold` (%).

### Running mini-redis

```bash
# Development (debug build)
make mini-redis

# Release build
make mini-redis-release

# Release + libsql persistence
make mini-redis-persist-release
```

Direct binary invocation:

```bash
cargo run --release --features "mini-redis mini-redis-libsql" --bin mini_redis -- \
  --bind 127.0.0.1 \
  --port 6379 \
  --databases 16 \
  --persist tmp/mini_redis.db \
  --aof \
  --script-mem 4194304 \
  --script-reset-threshold 90
```

If the binary is built without `--features mini-redis`, it exits with a message to rebuild with the feature enabled.

### Parity testing

The parity test suite (`tests/mini_redis_parity.py`) exercises 121 commands against
both a live Redis instance and mini-redis, comparing responses:

```bash
# Requires Redis on :6379; mini-redis is started automatically
make mini-redis-parity

# Verbose output
make mini-redis-parity-verbose
```

**Current parity score: 121/121** (Redis 8.6.1 and mini-redis both pass all 121 tests).

### Code structure after T-000078/T-000083 refactoring

The original monolithic `src/api.rs` (~2,500 lines) and `handle_command` dispatch
were split into focused modules to prepare for the standalone release:

- `src/api/mod.rs` — re-exports and shared helpers
- `src/api/eval_expr.rs` — expression evaluator
- `src/api/eval_program.rs` — program/statement evaluator
- `src/mini_redis/server/handle_command.rs` — Redis command router
- `src/mini_redis/server/handle_no_db_command.rs` — server-level command handler

## Benchmark and Comparison

### Pipelined throughput vs Redis (best-of-5, March 2026)

Methodology: `redis-benchmark -c 50 -n 1,000,000 -P 16 --csv`, 5 runs per server,
best RPS per test kept for the report. Run with `make perf-benchmark`.

| Test | mini-redis | Redis | mini/redis |
|------|----------:|------:|:----------:|
| GET   | 2,061,856 | 1,992,032 | **1.04×** |
| SET   | 1,355,014 | 1,592,357 | 0.85× ⚠ |
| INCR  | 2,164,502 | 1,883,239 | **1.15×** |
| LPUSH | 2,024,292 | 1,582,278 | **1.28×** |
| RPUSH | 2,016,129 | 1,721,170 | **1.17×** |
| LPOP  | 2,079,002 | 1,519,757 | **1.37×** |
| RPOP  | 2,036,660 | 1,636,661 | **1.24×** |
| SADD  | 2,183,406 | 1,757,469 | **1.24×** |
| HSET  | 1,259,446 | 1,572,327 | 0.80× ⚠ |

7 of 9 operations are at parity or faster than Redis. SET (−15%) and HSET (−20%)
are known regressions under pipelined write load — tracked for the standalone release.

### MuonJS vs Lua scripting throughput (February 2026)

3-round median, `make lua-js-perf-check`:

| Script case | Redis+Lua (rps) | mini-redis+MuonJS (rps) | Ratio |
|-------------|----------------:|------------------------:|:-----:|
| hello | 13,159 | 24,354 | **1.85×** |
| redis_call | 13,411 | 25,669 | **2.05×** |
| incrby | 13,494 | 25,744 | **1.90×** |
| keys_argv | 10,686 | 17,787 | **1.67×** |
| lrange | 11,614 | 20,320 | **1.75×** |
| hash_sum | 4,897 | 5,262 | **1.08×** |
| set_members | 7,266 | 8,474 | **1.17×** |
| bulk_incr | 10,389 | 9,746 | 0.94× |

Overall: **7 of 8 cases faster than Redis+Lua. Mean 1.56×, median 1.55×.**
MuonJS eliminates per-call re-parsing via its bytecode VM.

### Running benchmarks

```bash
# Pipelined throughput — mini-redis vs Redis (best of 5 runs)
# Requires Redis running on :6379
make perf-benchmark

# mini-redis only (CI-safe, no Redis dependency)
make perf-benchmark-no-redis

# Override run count or request volume
make perf-benchmark-no-redis PERF_BENCH_RUNS=3 PERF_BENCH_REQUESTS=500000

# Lua vs MuonJS scripting gate
make lua-js-perf-baseline   # capture new baseline
make lua-js-perf-check      # check against baseline (fails if >10% regression)

# JS runtime microbench (regression gate)
make js-runtime-bench
make js-runtime-bench-baseline
make js-runtime-bench-check
```

Benchmark logs are written to `tmp/` and comparison reports to `tmp/benchmark_comparison_*.txt`.
Full history is tracked in `performance.md`.

## Browser Demo (WASM + WebGPU)

The repo includes a socketless browser demo under `web/demo`:

- mini-redis runs inside a dedicated Web Worker.
- Rust/WASM exposes a typed command API (`exec`, `exec_batch`, `metrics_snapshot`).
- The UI uses a typed JS client and renders metrics on a WebGPU canvas.
- No disk persistence, no TCP sockets, no RESP networking in the browser path.

### Browser demo targets

```bash
# Root-level wrappers
make web-demo-wasm
make web-demo-dev
make web-demo-build
make web-demo-test

# Or run directly from the dedicated web/demo Makefile
cd web/demo
make wasm
make dev
make build
make test
```

### Browser demo workflow

1. Run `make web-demo-dev`.
2. Open the Vite URL (default `http://127.0.0.1:5173`).
3. Click `Start Simulation`.
4. Watch live throughput/latency/command-mix metrics update every 100ms.

## Project Notes

- Compatibility with mquickjs behavior is the primary goal.
- Validate behavior against upstream references in `vendor/` when changing semantics.
- Keep the runtime minimal and embeddable; prefer feature-gated additions.
