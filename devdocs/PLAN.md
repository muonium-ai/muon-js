# Port Plan: MQuickJS -> Rust (Muon JS)

## Goals
- Implement a **native Rust** JavaScript runtime that is **behaviorally compatible** with MQuickJS.
- Preserve the **memory model**, **value tagging**, **GC semantics**, and **bytecode** behaviors as the primary targets.
- Provide a **small, embeddable** API surface suitable for embedded Rust projects.

## Phase 0 — Compatibility inventory
- Catalog the MQuickJS public C API in `vendor/mquickjs/mquickjs.h`.
- Enumerate JS feature subset and strict-mode constraints from `vendor/mquickjs/README.md`.
- Identify bytecode structures and relocation APIs.
- Record memory/GC constraints and value tagging rules.

## Phase 1 — Core runtime architecture
- Define Rust equivalents for:
  - `JSContext` (allocator, GC state, global objects, stdlib table)
  - `JSValue` tagging layout for 32-bit and 64-bit targets
  - Atom table and string interning (WTF-8)
  - Object/property layout and array invariants (no holes)
- Decide on internal module layout and minimal public API.

## Phase 2 — Execution pipeline
- Port parser + bytecode compiler (non-recursive parse, single-pass codegen).
- Port the bytecode VM (stack-based, atom indirection).
- Implement `JS_Run`, `JS_Eval`, `JS_Parse`, and interrupt handling.

## Phase 3 — GC and allocator
- Implement compacting GC with movable objects.
- Provide GC-safe references analogous to `JSGCRef` semantics.
- Add DEBUG_GC style mode to stress relocation.

## Phase 4 — Standard library and tooling
- Port stdlib generator (`mquickjs_build.c`) into a Rust tool or keep a build-time C tool with Rust-readable output.
- Provide stdlib definition tables compatible with the runtime.
- Provide a tiny CLI (`mqjs`-like) for testing, bytecode emission, and REPL.

## Phase 5 — Bytecode persistence
- Implement bytecode layout, relocation, and loading APIs.
- Support 32-bit bytecode output on 64-bit hosts (compat parity).

## Phase 6 — Testing + benchmarks
- Port MQuickJS tests in `vendor/mquickjs/tests`.
- Add Rust-side tests for C API behavioral parity.
- Provide microbench and optional Octane compatibility path.

## Build & release workflow (Makefile-first)
- `make build` for library + CLI
- `make test` for unit + JS tests
- `make release` for tagged build artifacts

## Deliverables
- A Rust crate `muon-js` (library) with C-like API surface for embedders.
- Optional `mqjs` CLI for parity testing and bytecode workflows.
- Documentation of compatibility guarantees and any deviations.
