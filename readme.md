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

## Mini-Redis Implementation

Muon JS includes a Redis-compatible server implementation in `src/mini_redis/`:

- `resp.rs`: RESP protocol encode/decode.
- `server.rs`: async server loop, command dispatcher, scripting bridge.
- `store.rs`: in-memory data structures and command semantics.
- `persist.rs`: persistence abstraction and libsql-backed durability.

### Supported command families

- Core/control: `PING`, `ECHO`, `INFO`, `SELECT`, `DBSIZE`, `QUIT`, `MULTI/EXEC/DISCARD`.
- Strings: `GET`, `SET`, `SETNX`, `MSET`, `MGET`, `GETSET`, `APPEND`, `INCR/DECR`, `INCRBY/DECRBY`, `STRLEN`.
- Hashes: `HSET`, `HGET`, `HDEL`, `HGETALL`, `HLEN`, `HEXISTS`, `HINCRBY`, `HSETNX`.
- Lists: `LPUSH/RPUSH`, `LPOP/RPOP`, `LRANGE`, `LLEN`, `LINDEX`, `LSET`, `LINSERT`, `LREM`, `LPUSHX/RPUSHX`, `LTRIM`.
- Sets: `SADD`, `SREM`, `SMEMBERS`, `SISMEMBER`, `SCARD`, `SMOVE`, `SUNION`, `SINTER`.
- Sorted sets: `ZADD`, `ZRANGE`, `ZREM`, `ZCARD`.
- Streams: `XADD`, `XRANGE`, `XREVRANGE`, `XLEN`, `XDEL`.
- Keyspace/TTL: `DEL`, `EXISTS`, `EXPIRE`, `PEXPIRE`, `PERSIST`, `TTL`, `PTTL`, `TYPE`, `KEYS`, `SCAN`, `FLUSHDB`, `FLUSHALL`.
- Pub/Sub: `SUBSCRIBE`, `PUBLISH`.
- Scripting/admin: `EVAL`, `EVALSHA`, `SCRIPT`, `FUNCTION`, `CONFIG`, `CLIENT`, `SLOWLOG`, `SAVE`, `BGSAVE`, `REPLICAOF`.

### Run mini-redis

```bash
# Development run
make mini-redis

# Release run
make mini-redis-release

# Release + persistence (libsql)
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

If built without `mini-redis`, the binary exits with a message to rebuild using `--features mini-redis`.

## Benchmark And Comparison Usage

Redis vs mini-redis pipelined throughput:

```bash
make pipelined-benchmark-compare
```

Lua (Redis) vs MuonJS (mini-redis) scripting perf gate:

```bash
make lua-js-perf-baseline
make lua-js-perf-check
```

JS runtime microbench:

```bash
make js-runtime-bench
make js-runtime-bench-baseline
make js-runtime-bench-check
```

Latest benchmark notes and captured reports are tracked in `performance.md` and `tmp/` artifacts.

## Project Notes

- Compatibility with mquickjs behavior is the primary goal.
- Validate behavior against upstream references in `vendor/` when changing semantics.
- Keep the runtime minimal and embeddable; prefer feature-gated additions.
