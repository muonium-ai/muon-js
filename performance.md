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

## Code review quick wins
- Replace list storage with `VecDeque` to make LPUSH/LPOP $O(1)$ (currently `Vec` + `insert/remove` at index 0 is $O(n)$). See [src/mini_redis/store.rs](src/mini_redis/store.rs#L284-L337).
- Optimize `LRANGE` to avoid index-heavy access patterns and reduce cloning overhead. See [src/mini_redis/store.rs](src/mini_redis/store.rs#L331-L370).
- Avoid per-command `String` allocation for `to_upper_ascii` by matching case-insensitively on bytes. See [src/mini_redis/server.rs](src/mini_redis/server.rs#L1616-L1618).
- Skip `build_cmd` unless AOF is enabled; it currently clones args for every mutating command. See [src/mini_redis/server.rs](src/mini_redis/server.rs#L1632-L1639).
