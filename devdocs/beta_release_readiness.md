# Beta Release Readiness (mquickjs Compatibility)

**Date**: January 31, 2026
**Scope**: muon-js compatibility with mquickjs (feature parity + semantics), plus test signal and code review highlights.

---

## Test Results (Latest Run)

### Unit + Samples
- `make test`
  - ✅ Rust unit tests: **64/64 passing**
  - ✅ samples: **2/2 passing**

### Integration
- `make test-integration`
  - ✅ **9/10 passing**
  - ❌ **08_prime_checker.js** → `Exception`
    - Current script uses implicit globals (`i`, `primes`, `num`) which may be rejected depending on strict-mode behavior or global handling.

### mquickjs Compatibility
- `make test-mquickjs-detailed`
  - ❌ **0/42 passing**
  - All suites (`test_language.js`, `test_builtin.js`, `test_loop.js`, `test_closure.js`, mandelbrot, test_rect) currently throw `Exception`

### mini-redis parity
- `make mini-redis-parity`
  - ❌ Unable to run in current environment: `PermissionError` when binding a free port
  - Also emitted compiler warnings in `src/mini_redis/store.rs` (unused `mut`)

---

## Deduplicated Open Issues (Combined from PROGRESS/PORTING_STATUS)

### Core Architecture (mquickjs Parity Blockers)
- **Bytecode compiler + VM**: scaffolding only; no codegen or execution path
- **GC**: no tracing/compacting GC; JSGCRef is stubbed
- **ROM stdlib**: stdlib ROM generation not implemented
- **Memory model**: still Rust heap + 64-bit enum values; mquickjs uses fixed buffer + 32-bit tagging
- **Bytecode persistence**: header/relocation only; no compiler integration

### Language Semantics
- **Strict mode**: mquickjs is always strict; muon-js behavior is not fully strict
- **`new` + constructors**: object construction semantics missing
- **Prototypes / inheritance**: `prototype` / `__proto__` not implemented
- **Typed arrays**: not implemented
- **Template literals / destructuring / spread / classes / modules / async / generators / symbols / Map/Set/Proxy**: missing

### Parser & Expression Handling
- **Method chaining in complex expressions** still fails in some contexts
- **Operator precedence** and **nested call** handling still inconsistent in edge cases

### Standard Library Gaps
- **RegExp**: only subset (no lookaround/backreferences, limited flags)
- **JSON**: not yet audited for full mquickjs behavior (edge cases, error parity)
- **Date**: partial (`Date.now()` only)

### Known Mismatches / Documentation Drift
- `PROGRESS.md` and `PORTING_STATUS.md` contain items marked missing that are already implemented (e.g., arrow functions, default/rest params, let/const).
- Integration test status tables are stale (current run is 9/10, not 8/10).

---

## Code Review Notes (High-Level)

- **TODO marker**: `src/api.rs` references pending float64 typed array support.
- **`unwrap`/`expect` usage** is confined to tests and controlled paths; no critical runtime panic points found in a quick scan.
- **Mini-redis**: unused `mut` warnings in `src/mini_redis/store.rs`.
- **Error visibility**: `examples/eval.rs` prints only `Exception` without error details, which makes failures (like 08_prime_checker) hard to debug.

---

## Beta Readiness Summary

**Current readiness for mquickjs-compatible beta: Not ready.**

Primary blockers:
1. **mquickjs test suite: 0% pass** (core compatibility gap).
2. **Missing VM + GC architecture** (mquickjs-critical).
3. **Missing constructor/prototype semantics** (ubiquitous in mquickjs tests).
4. **Parser edge cases** (method chaining, precedence).

---

## Recommended Next Steps (Ordered)

1. **Fix 08_prime_checker** (integration test) by handling implicit globals or enforcing strict-mode with clearer errors.
2. **Add mquickjs error output** in `examples/eval.rs` to show exception details during test runs.
3. **Bring `PORTING_STATUS.md` / `PROGRESS.md` in sync** to reduce tracking noise.
4. **Start mquickjs test-driven porting**: implement missing semantics in the order tests exercise them.
5. **Plan architecture work**: VM + GC milestones, with compatibility checkpoints.
6. **Rerun mini-redis parity** in an environment that allows local socket binds, or add a non-sandboxed port selection path for CI.

---

## Notes on Deduplication
This document consolidates overlapping items from `PROGRESS.md` and `PORTING_STATUS.md` into a single list above. Items already implemented are not repeated; remaining issues are grouped by impact area.
