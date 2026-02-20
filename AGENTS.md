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

## Sandbox restrictions
- File operations are restricted to the project directory.
- Use the `tmp/` folder (inside the project root) for temporary test files.
- Paths outside the project folder are denied by the sandbox.

## MuonTickets agent workflow
Use MuonTickets to coordinate work through ticket files in `tickets/`.

### Ticket states
- `ready`: not started
- `claimed`: currently being implemented
- `blocked`: waiting on dependency or input
- `needs_review`: implementation complete, awaiting review or merge
- `done`: merged and complete

### Standard loop
1. Pull latest changes.
2. Pick and claim one ticket.
3. Implement on the branch recorded in the ticket.
4. Add progress comments as you work.
5. Move to `needs_review` when coding is complete.
6. Run validation before commit or push.
7. After merge, mark ticket `done`.

### Commands (uv)
```bash
# Pick best available ticket for this agent
uv run python3 tickets/mt/muontickets/muontickets/mt.py pick --owner agent-1

# List my claimed tickets
uv run python3 tickets/mt/muontickets/muontickets/mt.py ls --status claimed --owner agent-1

# Add a progress update
uv run python3 tickets/mt/muontickets/muontickets/mt.py comment T-000001 "Implemented API and tests"

# Move ticket to review
uv run python3 tickets/mt/muontickets/muontickets/mt.py set-status T-000001 needs_review

# Validate board consistency
uv run python3 tickets/mt/muontickets/muontickets/mt.py validate

# After merge, mark complete
uv run python3 tickets/mt/muontickets/muontickets/mt.py done T-000001
```

### Rules for agents
- Do not start work without claiming a ticket.
- Respect `depends_on`; do not bypass unless explicitly instructed.
- Keep ticket updates small, frequent, and deterministic.
- Always run validation before commit or push.
- Keep one active ticket at a time unless team policy allows higher WIP.

### Backlog shape (throughput-first)
- Avoid creating hundreds of micro-tickets; this usually hurts throughput.
- Too many tickets increase context switching, duplicate work, and stale status churn.
- Prefer a layered backlog:
	- a small set of epics,
	- each epic with a few implementation tickets,
	- optional checklist items inside each ticket.
- Keep only about 10-30 `ready` tickets active at a time.
- Park overflow work in a backlog or archive doc and promote items when capacity opens.
- Use strict ticket templates with required fields:
	- goal,
	- acceptance criteria,
	- dependencies,
	- test plan.
