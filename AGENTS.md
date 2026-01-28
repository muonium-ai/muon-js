# AGENTS.md

## Project goal
Build a **tiny, embeddable JavaScript runtime in Rust** that is a **true port of mquickjs** (not a wrapper or FFI shim). Compatibility with mquickjs is the primary success metric.

## Compatibility requirements
- Treat mquickjs behavior, API shape, and semantics as the source of truth.
- Prefer fidelity over Rust-idiomatic redesigns when the two conflict.
- Changes that reduce compatibility must be explicitly called out and justified.

## Scope constraints
- No wrapper or binding over the C library; implement the runtime in Rust.
- Keep the runtime minimal and embeddable for other Rust projects.
- Avoid unnecessary dependencies and large feature creep.

## Build and release
- Prefer **Makefiles** for build and release workflows.
- Provide `make build`, `make test`, and `make release` targets where applicable.
- Keep build outputs small and deterministic.

## Contributor expectations
- When in doubt, verify behavior against the upstream mquickjs repo in `vendor/`.
- Add tests that lock in mquickjs-compatible behavior.
- Document any intentionally unsupported features.

## Communication
- Be explicit about compatibility tradeoffs.
- Use concise, actionable notes and avoid speculative changes.
