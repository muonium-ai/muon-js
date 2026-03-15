#!/usr/bin/env python3
"""TrafficLab benchmark harness (T-000112).

Runs `cargo test --release bench_trafficlab_` with --nocapture, parses the
eprintln! output from bench_trafficlab_summary_table, writes results to
devdocs/trafficlab_bench_baseline.json, and asserts the required CI gates.

Usage:
    python3 trafficlab/bench.py [--baseline]

Options:
    --baseline   Overwrite devdocs/trafficlab_bench_baseline.json with current
                 results (use on first run or after deliberate perf changes).

Exit codes:
    0  All CI gates pass.
    1  One or more gates failed (ratio < 10x at 1000x warp).
    2  Subprocess or parse error.
"""

import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path

BASELINE_FILE = Path(__file__).parent.parent / "devdocs" / "trafficlab_bench_baseline.json"
CARGO_MANIFEST = Path(__file__).parent / "Cargo.toml"

# CI gate: cached must be >= this many times faster than nocache at 1000x warp.
RATIO_GATE = 10.0
# Regression gate vs stored baseline (fraction tolerated, e.g. 0.20 = 20% slower).
REGRESSION_GATE = 0.20


def run_bench_tests() -> str:
    """Run the bench_trafficlab_* tests and return combined stderr+stdout."""
    cmd = [
        "cargo", "test", "--release",
        "--manifest-path", str(CARGO_MANIFEST),
        "bench_trafficlab_",
        "--", "--nocapture",
    ]
    print(f"[bench] running: {' '.join(cmd)}")
    t0 = time.monotonic()
    result = subprocess.run(cmd, capture_output=True, text=True)
    elapsed = time.monotonic() - t0
    print(f"[bench] finished in {elapsed:.1f}s  exit={result.returncode}")
    combined = result.stdout + result.stderr
    if result.returncode != 0:
        print("[bench] FAILED — cargo test output:")
        print(combined)
        sys.exit(2)
    return combined


def parse_table(output: str) -> list[dict]:
    """Parse rows emitted by bench_trafficlab_summary_table.

    Expected format (eprintln!):
        warp     nc_tps     ca_tps   ratio    hit%       keys
        1x     12345678   87654321   7.1x   100.0%        6
        100x   ...
        1000x  ...
    """
    rows = []
    # Match lines like: "1x    123456    234567    19.0x    100.0%    6"
    pattern = re.compile(
        r"^\s*(\d+x)\s+([\d.]+)\s+([\d.]+)\s+([\d.]+)x\s+([\d.]+)%\s+(\d+)"
    )
    for line in output.splitlines():
        m = pattern.match(line)
        if m:
            warp, nc_tps, ca_tps, ratio, hit_pct, keys = m.groups()
            rows.append({
                "warp": warp,
                "nc_tps": float(nc_tps),
                "ca_tps": float(ca_tps),
                "ratio": float(ratio),
                "hit_pct": float(hit_pct),
                "keys": int(keys),
            })
    return rows


def print_table(rows: list[dict]) -> None:
    header = f"{'warp':<8}  {'nc_tps':>12}  {'ca_tps':>12}  {'ratio':>8}  {'hit%':>8}  {'keys':>6}"
    sep = "-" * len(header)
    print(f"\n{sep}")
    print(header)
    print(sep)
    for r in rows:
        print(
            f"{r['warp']:<8}  {r['nc_tps']:>12,.0f}  {r['ca_tps']:>12,.0f}  "
            f"{r['ratio']:>7.1f}x  {r['hit_pct']:>7.1f}%  {r['keys']:>6}"
        )
    print(sep)


def check_gates(rows: list[dict]) -> bool:
    """Return True if all CI gates pass."""
    ok = True
    for r in rows:
        if r["warp"] == "1000x":
            if r["ratio"] < RATIO_GATE:
                print(
                    f"[FAIL] 1000x warp ratio {r['ratio']:.2f}x < required {RATIO_GATE:.0f}x"
                )
                ok = False
            else:
                print(f"[PASS] 1000x warp ratio {r['ratio']:.1f}x >= {RATIO_GATE:.0f}x")
    if not any(r["warp"] == "1000x" for r in rows):
        print("[WARN] no 1000x warp row found in bench output — gate not checked")
    return ok


def check_regression(rows: list[dict], baseline: dict) -> bool:
    """Compare each row against stored baseline. Warn but don't fail on first run."""
    if "rows" not in baseline:
        return True
    baseline_by_warp = {r["warp"]: r for r in baseline["rows"]}
    ok = True
    for r in rows:
        w = r["warp"]
        if w not in baseline_by_warp:
            continue
        b = baseline_by_warp[w]
        # Check cached TPS didn't regress.
        allowed_min = b["ca_tps"] * (1.0 - REGRESSION_GATE)
        if r["ca_tps"] < allowed_min:
            print(
                f"[REGRESSION] {w} cached_tps {r['ca_tps']:,.0f} < allowed {allowed_min:,.0f} "
                f"(baseline {b['ca_tps']:,.0f}, -{REGRESSION_GATE*100:.0f}% gate)"
            )
            ok = False
        else:
            print(f"[OK] {w} cached_tps {r['ca_tps']:,.0f}  (baseline {b['ca_tps']:,.0f})")
    return ok


def write_baseline(rows: list[dict]) -> None:
    payload = {
        "schema_version": 1,
        "description": "TrafficLab benchmark baseline — cached vs nocache simulation throughput",
        "generated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "gates": {
            "min_ratio_at_1000x_warp": RATIO_GATE,
            "regression_tolerance": REGRESSION_GATE,
        },
        "rows": rows,
    }
    BASELINE_FILE.parent.mkdir(parents=True, exist_ok=True)
    BASELINE_FILE.write_text(json.dumps(payload, indent=2) + "\n")
    print(f"[bench] baseline written to {BASELINE_FILE}")


def main() -> None:
    write_baseline_mode = "--baseline" in sys.argv

    output = run_bench_tests()
    rows = parse_table(output)

    if not rows:
        print("[bench] WARNING: no table rows parsed — bench_trafficlab_summary_table "
              "may not have printed output.  Check --nocapture is working.")
        # Still check the 10x gate via the 1000x bench test (already asserted in Rust).
        print("[bench] cargo test bench_trafficlab_1000x_throughput_10x passed (assert in Rust).")
        # Write a minimal baseline so the JSON file always exists.
        rows = [{"warp": "1000x", "nc_tps": 0, "ca_tps": 0, "ratio": 0,
                 "hit_pct": 0, "keys": 0, "_note": "no parsed output"}]

    print_table(rows)

    # Read existing baseline for regression check (if present and not overwriting).
    baseline: dict = {}
    if BASELINE_FILE.exists() and not write_baseline_mode:
        try:
            baseline = json.loads(BASELINE_FILE.read_text())
        except json.JSONDecodeError:
            print(f"[bench] could not parse existing baseline at {BASELINE_FILE}")

    gates_ok = check_gates(rows)
    regression_ok = check_regression(rows, baseline)

    if write_baseline_mode or not BASELINE_FILE.exists():
        write_baseline(rows)

    if not gates_ok:
        print("\n[bench] FAILED — one or more CI gates not met.")
        sys.exit(1)

    if not regression_ok:
        print("\n[bench] REGRESSION detected — cached throughput dropped > "
              f"{REGRESSION_GATE*100:.0f}% vs baseline.")
        sys.exit(1)

    print("\n[bench] All gates passed.")


if __name__ == "__main__":
    main()
