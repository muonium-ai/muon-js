#!/usr/bin/env python3
import argparse
import json
import re
import statistics
import socket
import subprocess
import sys
import time
from pathlib import Path

SUMMARY_RE = re.compile(r"^SUMMARY_JSON=(\{.*\})$")


def run(cmd: str) -> None:
    subprocess.run(cmd, shell=True, check=True)


def run_capture_to_log(cmd: str, log_path: Path) -> None:
    proc = subprocess.run(cmd, shell=True, check=False, capture_output=True, text=True)
    output = (proc.stdout or "") + (proc.stderr or "")
    log_path.write_text(output)
    if proc.returncode != 0:
        raise RuntimeError(f"Command failed ({proc.returncode}): {cmd}\nSee log: {log_path}")


def load_summary_from_log(path: Path) -> dict[str, float]:
    text = path.read_text(errors="ignore")
    for line in reversed(text.splitlines()):
        m = SUMMARY_RE.match(line.strip())
        if m:
            payload = json.loads(m.group(1))
            return {row["name"]: float(row["rps"]) for row in payload["results"]}
    raise RuntimeError(f"No SUMMARY_JSON found in {path}")


def pick_port() -> int:
    out = subprocess.run("python3 scripts/pick_port.py", shell=True, check=True, capture_output=True, text=True)
    return int(out.stdout.strip())


def wait_tcp(host: str, port: int, timeout_s: float = 20.0) -> bool:
    deadline = time.time() + timeout_s
    while time.time() < deadline:
        try:
            with socket.create_connection((host, port), timeout=0.5):
                return True
        except OSError:
            time.sleep(0.1)
    return False


def wait_redis_ready(port: int, timeout_s: float = 20.0) -> bool:
    deadline = time.time() + timeout_s
    while time.time() < deadline:
        try:
            ping = subprocess.run(
                f"redis-cli -p {port} PING",
                shell=True,
                check=False,
                capture_output=True,
                text=True,
            )
            info = subprocess.run(
                f"redis-cli -p {port} INFO persistence",
                shell=True,
                check=False,
                capture_output=True,
                text=True,
            )
            if ping.stdout.strip().upper() == "PONG" and "loading:0" in info.stdout:
                return True
        except Exception:
            pass
        time.sleep(0.1)
    return False


def run_round(round_idx: int, redis_port: int, log_dir: Path) -> dict:
    stamp = time.strftime("%Y%m%d_%H%M%S")
    lua_log = log_dir / f"redis_lua_round{round_idx}_{stamp}.log"
    js_log = log_dir / f"mini_js_round{round_idx}_{stamp}.log"

    redis_pidfile = log_dir / f"redis_round{round_idx}.pid"
    redis_log = log_dir / f"redis_round{round_idx}.server.log"
    run(f"redis-server --port {redis_port} --daemonize yes --pidfile {redis_pidfile} --logfile {redis_log}")
    if not wait_redis_ready(redis_port):
        run(f"redis-cli -p {redis_port} SHUTDOWN NOSAVE || true")
        raise RuntimeError(f"Redis on port {redis_port} did not become ready")

    run_capture_to_log(
        f"python3 scripts/bench_scripting.py --host 127.0.0.1 --port {redis_port} --suite tests/scripting/bench_suite.json",
        lua_log,
    )
    run(f"redis-cli -p {redis_port} SHUTDOWN NOSAVE || true")

    mini_port = pick_port()
    mini_server = subprocess.Popen(
        [
            "target/release/muon_cache",
            "--bind",
            "127.0.0.1",
            "--port",
            str(mini_port),
        ],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        cwd=Path.cwd(),
    )
    try:
        if not wait_tcp("127.0.0.1", mini_port):
            raise RuntimeError(f"muoncache on port {mini_port} did not become ready")
        run_capture_to_log(
            f"python3 scripts/bench_scripting.py --host 127.0.0.1 --port {mini_port} --suite tests/scripting_js_faithful/bench_suite.json",
            js_log,
        )
    finally:
        mini_server.terminate()
        try:
            mini_server.wait(timeout=3)
        except subprocess.TimeoutExpired:
            mini_server.kill()

    lua = load_summary_from_log(lua_log)
    js = load_summary_from_log(js_log)
    shared_cases = sorted(set(lua) & set(js))
    ratios = {case: (js[case] / lua[case] if lua[case] else 0.0) for case in shared_cases}

    return {
        "round": round_idx,
        "redis_port": redis_port,
        "muon_cache_port": mini_port,
        "lua_log": str(lua_log),
        "js_log": str(js_log),
        "lua_rps": lua,
        "js_rps": js,
        "ratios": ratios,
        "avg_ratio": statistics.mean(ratios.values()) if ratios else 0.0,
    }


def aggregate(rounds: list[dict]) -> dict:
    cases = sorted(rounds[0]["ratios"].keys()) if rounds else []
    case_summary: dict[str, dict[str, float]] = {}
    for case in cases:
        vals = [r["ratios"][case] for r in rounds]
        case_summary[case] = {
            "mean": statistics.mean(vals),
            "median": statistics.median(vals),
            "min": min(vals),
            "max": max(vals),
        }

    avg_vals = [r["avg_ratio"] for r in rounds]
    return {
        "case_summary": case_summary,
        "overall": {
            "mean": statistics.mean(avg_vals) if avg_vals else 0.0,
            "median": statistics.median(avg_vals) if avg_vals else 0.0,
            "min": min(avg_vals) if avg_vals else 0.0,
            "max": max(avg_vals) if avg_vals else 0.0,
        },
    }


def check_regressions(
    current: dict,
    baseline: dict,
    critical_cases: list[str],
    max_regression: float,
) -> list[str]:
    failures: list[str] = []
    cur_cases = current["aggregate"]["case_summary"]
    base_cases = baseline.get("aggregate", {}).get("case_summary", {})

    for case in critical_cases:
        if case not in cur_cases:
            failures.append(f"Missing case in current run: {case}")
            continue
        if case not in base_cases:
            failures.append(f"Missing case in baseline: {case}")
            continue

        cur = float(cur_cases[case]["median"])
        base = float(base_cases[case]["median"])
        allowed = base * (1.0 - max_regression)
        if cur < allowed:
            pct = ((base - cur) / base * 100.0) if base else 0.0
            failures.append(
                f"{case}: median ratio regressed {pct:.2f}% (baseline={base:.4f}, current={cur:.4f}, allowed>={allowed:.4f})"
            )

    return failures


def render_text(report: dict) -> str:
    lines: list[str] = []
    lines.append("Lua vs JavaScript scripting benchmark gate")
    lines.append(f"generated_at={report['generated_at']}")
    lines.append(f"rounds={report['rounds']}")
    lines.append("")

    for r in report["results"]:
        lines.append(f"Round {r['round']} (redis_port={r['redis_port']}): avg_ratio={r['avg_ratio']:.4f}x")
        lines.append(f"  lua_log={r['lua_log']}")
        lines.append(f"  js_log={r['js_log']}")

    lines.append("")
    header = f"{'CASE':20} {'MEAN':>8} {'MEDIAN':>8} {'MIN':>8} {'MAX':>8}"
    lines.append(header)
    lines.append("-" * len(header))

    for case, s in report["aggregate"]["case_summary"].items():
        lines.append(f"{case:20} {s['mean']:8.2f} {s['median']:8.2f} {s['min']:8.2f} {s['max']:8.2f}")

    ov = report["aggregate"]["overall"]
    lines.append("")
    lines.append(f"overall_mean={ov['mean']:.4f}x")
    lines.append(f"overall_median={ov['median']:.4f}x")

    if report.get("check"):
        lines.append("")
        lines.append(f"check_baseline={report['check']['baseline']}")
        lines.append(f"check_max_regression={report['check']['max_regression']:.4f}")
        if report["check"]["failures"]:
            lines.append("check_status=FAIL")
            for failure in report["check"]["failures"]:
                lines.append(f"- {failure}")
        else:
            lines.append("check_status=PASS")

    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(description="Run 3-round Redis Lua vs muoncache JS performance gate")
    parser.add_argument("--rounds", type=int, default=3)
    parser.add_argument("--redis-base-port", type=int, default=6385)
    parser.add_argument("--log-dir", default="tmp/comparison/lua_js_gate", help="Directory for per-round logs")
    parser.add_argument("--out", required=True, help="Output JSON report path")
    parser.add_argument("--baseline", default=None, help="Optional baseline JSON path for regression checks")
    parser.add_argument("--max-regression", type=float, default=0.10)
    parser.add_argument(
        "--critical-cases",
        nargs="+",
        default=["hash_sum", "set_members", "bulk_incr"],
    )
    args = parser.parse_args()

    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    log_dir = Path(args.log_dir)
    log_dir.mkdir(parents=True, exist_ok=True)

    run("cargo build --release --features muoncache --bin muon_cache")

    results = []
    for idx in range(args.rounds):
        round_id = idx + 1
        redis_port = args.redis_base_port + idx
        results.append(run_round(round_id, redis_port, log_dir))

    report = {
        "generated_at": time.strftime("%Y-%m-%d %H:%M:%S"),
        "rounds": args.rounds,
        "results": results,
        "aggregate": aggregate(results),
    }

    if args.baseline:
        baseline_path = Path(args.baseline)
        baseline = json.loads(baseline_path.read_text())
        failures = check_regressions(report, baseline, args.critical_cases, args.max_regression)
        report["check"] = {
            "baseline": str(baseline_path),
            "critical_cases": args.critical_cases,
            "max_regression": args.max_regression,
            "failures": failures,
        }

    out_path.write_text(json.dumps(report, indent=2) + "\n")

    text_report = render_text(report)
    txt_path = out_path.with_suffix(".txt")
    txt_path.write_text(text_report)
    print(text_report, end="")
    print(f"REPORT_JSON={out_path}")
    print(f"REPORT_TXT={txt_path}")

    if report.get("check", {}).get("failures"):
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
