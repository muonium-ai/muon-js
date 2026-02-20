#!/usr/bin/env python3
import argparse
import json
import sys
from pathlib import Path


def load_results(path: Path):
    data = json.loads(path.read_text())
    rows = data.get("results", [])
    out = {}
    for row in rows:
        name = row.get("name")
        med = row.get("median_ops_per_sec")
        if isinstance(name, str) and isinstance(med, (int, float)):
            out[name] = float(med)
    return out


def main() -> int:
    parser = argparse.ArgumentParser(description="Check JS runtime benchmark regressions")
    parser.add_argument("--baseline", required=True, help="Baseline benchmark JSON path")
    parser.add_argument("--current", required=True, help="Current benchmark JSON path")
    parser.add_argument(
        "--max-regression",
        type=float,
        default=0.20,
        help="Allowed fractional regression (e.g. 0.20 = 20%%)",
    )
    args = parser.parse_args()

    baseline_path = Path(args.baseline)
    current_path = Path(args.current)

    if not baseline_path.exists():
        print(f"Baseline file not found: {baseline_path}")
        return 2
    if not current_path.exists():
        print(f"Current file not found: {current_path}")
        return 2

    baseline = load_results(baseline_path)
    current = load_results(current_path)

    baseline_cases = sorted(baseline.keys())
    current_cases = set(current.keys())
    missing = [case for case in baseline_cases if case not in current_cases]
    if missing:
        print("Missing benchmark cases in current results:")
        for case in missing:
            print(f"  - {case}")
        return 2

    failures = []
    print("JS runtime benchmark regression check")
    print(f"  baseline: {baseline_path}")
    print(f"  current : {current_path}")
    print(f"  threshold: {args.max_regression * 100:.1f}% max slowdown")
    print("")

    for case in baseline_cases:
        base = baseline[case]
        curr = current[case]
        if base <= 0:
            status = "SKIP"
            delta_pct = 0.0
        else:
            ratio = curr / base
            delta_pct = (ratio - 1.0) * 100.0
            if curr < base * (1.0 - args.max_regression):
                status = "FAIL"
                failures.append((case, base, curr, delta_pct))
            else:
                status = "PASS"
        print(f"{status:4} {case:28} base={base:10.2f} curr={curr:10.2f} delta={delta_pct:7.2f}%")

    if failures:
        print("\nRegression check failed:")
        for case, base, curr, delta_pct in failures:
            print(f"  - {case}: base={base:.2f}, curr={curr:.2f}, delta={delta_pct:.2f}%")
        return 1

    print("\nRegression check passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
