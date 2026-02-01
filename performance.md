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
