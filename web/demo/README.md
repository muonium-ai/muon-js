# MuonCache Browser Demo

Socketless in-browser MuonCache running via Rust/WASM with a WebGPU metrics dashboard and MuonJS eval playground.

## Prerequisites

- Rust stable toolchain with `wasm32-unknown-unknown` target
- `wasm-bindgen-cli` (install with `cargo install wasm-bindgen-cli --version 0.2.114 --locked`)
- Node.js 20+

## Local Development

```bash
# From repo root
make web-demo-dev

# Or from this directory
make wasm
npm install
npm run dev
```

Open http://127.0.0.1:5173 in Chrome or Firefox.

## Production Build

```bash
make build
```

Output goes to `dist/`.

## Features

- **Leaderboard simulation** — 1500 players, 320 score updates per tick at 20ms intervals
- **WebGPU dashboard** — real-time OPS throughput and latency percentile graphs
- **MuonJS playground** — evaluate JavaScript expressions using the MuonJS runtime
- **Web Worker architecture** — all WASM execution runs off the main thread
