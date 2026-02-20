# Performance notes

## Summary
Current benchmarks show mini-redis is between 9x and 300x slower than Redis (C) depending on command and workload. The likely causes below are common in early-stage Rust ports and are consistent with the observed slowdown range.

## Likely causes (hypotheses)
- Build configuration not optimized (missing LTO, target-cpu tuning, or release-only flags).
- Per-command allocations/copies in hot paths (Vec growth, String conversions, cloning).
- Data structure overhead (hashing, boxing, indirection, cache-miss heavy layouts).
- Persistence/logging work on the hot path (AOF/snapshot writes or sync points).
- Coarse locking and async scheduler overhead (context switches, mutex contention).
- Parsing overhead (excess validation and conversions per command).
- Single-threaded request handling vs Redis’s highly optimized event loop.

## Low-hanging improvements
- Ensure release build with LTO and target-cpu=native for benchmarks.
- Reduce allocations: reuse buffers, preallocate, avoid String for binary keys.
- Reduce cloning: pass slices, use shared byte buffers where safe.
- Gate logging/persistence in hot path; batch or move to background.
- Avoid unnecessary async on hot path; keep critical path sync and lean.
- Replace heavy data structures with cache-friendly layouts where possible.

## Next validation steps
- Run benchmarks in strict release mode and compare again.
- Add microbenchmarks around parsing and command execution.
- Profile CPU (sampling) to confirm top hotspots.
- Toggle persistence/logging to measure impact.

## JS runtime microbenchmark harness
- Command: `make js-runtime-bench`
- Optional overrides:
	- `JS_BENCH_ITERS` (default `5000`)
	- `JS_BENCH_WARMUP` (default `500`)
	- `JS_BENCH_RUNS` (default `5`)
	- `JS_BENCH_OUT` (default `tmp/comparison/js_runtime_benchmark_<timestamp>.json`)
- Workloads covered:
	- parser arithmetic expression workload
	- parser function declaration workload
	- eval for-loop workload
	- builtin identifier lookup workload
	- global property roundtrip workload
	- VM global load/store workload (`x = x + 1` bytecode path)
	- string replaceAll workload
	- string regex replace workload
	- object property access workload

### Baseline capture policy
- Run at least 3-5 runs per workload and use median ops/s as primary signal.
- Save output JSON files under `tmp/comparison/` for local comparison.
- Compare before/after numbers for each optimization ticket to avoid regressions.

## Regression gate workflow
- Create or refresh baseline (lightweight settings):
	- `make js-runtime-bench-baseline`
- Run regression check against baseline:
	- `make js-runtime-bench-check`
- Key settings:
	- `JS_BENCH_BASELINE` (default `devdocs/js_runtime_benchmark_baseline.json`)
	- `JS_BENCH_CHECK_ITERS` / `JS_BENCH_CHECK_WARMUP` / `JS_BENCH_CHECK_RUNS`
	- `JS_BENCH_MAX_REGRESSION` (default `0.20`, i.e. max 20% slowdown per case)

### Handling intentional changes
- If performance changes are intentional and justified, regenerate baseline after review/approval:
	- `make js-runtime-bench-baseline`
- Keep rationale in PR/ticket notes when baseline is updated.

## Profiling results (CPU sampling)
- Dominant time spent in persistence logging: `Persist::log_command` → libsql/sqlite `execute` → `sqlite3_step` → `vdbeCommit` → `fsync`.
- Async runtime overhead visible: `async_global_executor` scheduling and `async_io::reactor` wait/park cycles.
- Additional overhead from allocations (`RawVec::grow_one`, queue `push`).

These indicate persistence write/commit/fsync is a major bottleneck under load, with non-trivial runtime scheduling overhead.

## Targeted toggles for next tests
- Run without persistence (no libsql logging) and compare throughput.
- Run with persistence but batch/async logging (if supported) to avoid per-command fsync.
- Reduce logging verbosity on hot path.
- Compare async vs sync server loop (if feasible) to isolate runtime overhead.

## Scripting hotspot microbench workflow
- Full faithful JS scripting suite:
	- `make mini-redis-js-scripting-bench`
- Hotspot-only suite (hash/set/incr density):
	- `make mini-redis-js-scripting-bench-hotspots`

## Lua-vs-JS performance gate
- Generate/update baseline (3 rounds by default):
	- `make lua-js-perf-baseline`
- Run regression check against baseline:
	- `make lua-js-perf-check`

### Gate settings
- `LUA_JS_GATE_ROUNDS` (default `3`)
- `LUA_JS_GATE_REDIS_BASE_PORT` (default `6385`)
- `LUA_JS_GATE_BASELINE` (default `devdocs/lua_js_perf_baseline.json`)
- `LUA_JS_GATE_OUT` (default `tmp/comparison/lua_js_perf_gate_<timestamp>.json`)
- `LUA_JS_GATE_MAX_REGRESSION` (default `0.10` = max 10% median-ratio regression)
- `LUA_JS_GATE_CRITICAL_CASES` (default `hash_sum set_members bulk_incr`)

### Gate outputs
- JSON report (full per-round data + aggregates)
- Text summary (same path, `.txt` extension)
- Check mode exits non-zero when critical cases regress beyond threshold.

### Latest 3-run comparison snapshot (2026-02-21)
- Command run: `python3 tmp/run_lua_js_3rounds.py`
- Report: `tmp/lua_js_comparison_3runs_20260221_001157.txt`
- Aggregate ratios (mini-redis JS / Redis Lua):
	- `overall_avg_ratio_mean=1.2152x`
	- `overall_avg_ratio_median=1.2202x`
- Per-case mean ratios:
	- Faster than Redis Lua: `hello=1.90x`, `incrby=2.05x`, `keys_argv=1.64x`, `lrange=1.65x`, `redis_call=1.89x`
	- Slower than Redis Lua: `hash_sum=0.16x`, `set_members=0.24x`, `bulk_incr=0.19x`

### Hotspot benchmark defaults
- `MINI_REDIS_JS_HOTSPOT_CASES` default: `hash_sum set_members bulk_incr`
- `MINI_REDIS_JS_HOTSPOT_ITERS` default: `1000`
- `MINI_REDIS_JS_HOTSPOT_WARMUP` default: `200`
- Structured outputs:
	- JSON: `MINI_REDIS_JS_HOTSPOT_JSON` (default `tmp/mini_redis_js_hotspots_<timestamp>.json`)
	- CSV: `MINI_REDIS_JS_HOTSPOT_CSV` (default `tmp/mini_redis_js_hotspots_<timestamp>.csv`)

### Script-level options
- `scripts/bench_scripting.py` supports:
	- `--cases <name ...>` to run selected benchmark cases only.
	- `--out-json <path>` for machine-readable summary output.
	- `--out-csv <path>` for per-case tabular output.

## Code review quick wins
- Replace list storage with `VecDeque` to make LPUSH/LPOP $O(1)$ (currently `Vec` + `insert/remove` at index 0 is $O(n)$). See [src/mini_redis/store.rs](src/mini_redis/store.rs#L284-L337).
- Optimize `LRANGE` to avoid index-heavy access patterns and reduce cloning overhead. See [src/mini_redis/store.rs](src/mini_redis/store.rs#L331-L370).
- Avoid per-command `String` allocation for `to_upper_ascii` by matching case-insensitively on bytes. See [src/mini_redis/server.rs](src/mini_redis/server.rs#L1616-L1618).
- Skip `build_cmd` unless AOF is enabled; it currently clones args for every mutating command. See [src/mini_redis/server.rs](src/mini_redis/server.rs#L1632-L1639).
