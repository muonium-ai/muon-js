#!/usr/bin/env python3
"""Multi-threaded Lua vs JS comparison — runs bench_scripting_mt against
both Redis+Lua and mini-redis+JS, then produces a comparison report.

Usage:
    python3 tools/lua_js_mt_bench.py \
        --threads 8 --total 1000000 --rounds 1 \
        --out tmp/comparison/mt_bench.json
"""
import argparse
import json
import re
import socket
import subprocess
import sys
import time
from pathlib import Path

SUMMARY_RE = re.compile(r"^SUMMARY_JSON=(\{.*\})$")


def pick_port() -> int:
    out = subprocess.run(
        "python3 scripts/pick_port.py",
        shell=True, check=True, capture_output=True, text=True,
    )
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
                shell=True, check=False, capture_output=True, text=True,
            )
            info = subprocess.run(
                f"redis-cli -p {port} INFO persistence",
                shell=True, check=False, capture_output=True, text=True,
            )
            if ping.stdout.strip().upper() == "PONG" and "loading:0" in info.stdout:
                return True
        except Exception:
            pass
        time.sleep(0.1)
    return False


def run_capture(cmd: str, log_path: Path) -> None:
    proc = subprocess.run(cmd, shell=True, check=False, capture_output=True, text=True)
    output = (proc.stdout or "") + (proc.stderr or "")
    log_path.write_text(output)
    if proc.returncode != 0:
        raise RuntimeError(f"Command failed ({proc.returncode}): {cmd}\nSee: {log_path}")


def load_summary(path: Path) -> dict:
    text = path.read_text(errors="ignore")
    for line in reversed(text.splitlines()):
        m = SUMMARY_RE.match(line.strip())
        if m:
            return json.loads(m.group(1))
    raise RuntimeError(f"No SUMMARY_JSON in {path}")


def run_round(
    round_idx: int,
    redis_port: int,
    threads: int,
    total: int,
    warmup: int,
    log_dir: Path,
) -> dict:
    stamp = time.strftime("%Y%m%d_%H%M%S")
    lua_log = log_dir / f"redis_lua_mt_round{round_idx}_{stamp}.log"
    js_log = log_dir / f"mini_js_mt_round{round_idx}_{stamp}.log"

    # --- Redis + Lua ---
    redis_pidfile = log_dir / f"redis_mt_round{round_idx}.pid"
    redis_logfile = log_dir / f"redis_mt_round{round_idx}.server.log"
    subprocess.run(
        f"redis-server --port {redis_port} --daemonize yes "
        f"--pidfile {redis_pidfile} --logfile {redis_logfile}",
        shell=True, check=True,
    )
    if not wait_redis_ready(redis_port):
        subprocess.run(f"redis-cli -p {redis_port} SHUTDOWN NOSAVE || true", shell=True)
        raise RuntimeError(f"Redis on port {redis_port} not ready")

    run_capture(
        f"python3 scripts/bench_scripting_mt.py "
        f"--host 127.0.0.1 --port {redis_port} "
        f"--suite tests/scripting/bench_suite.json "
        f"--threads {threads} --total {total} --warmup {warmup}",
        lua_log,
    )
    subprocess.run(f"redis-cli -p {redis_port} SHUTDOWN NOSAVE || true", shell=True)
    time.sleep(0.5)

    # --- mini-redis + JS ---
    mini_port = pick_port()
    mini = subprocess.Popen(
        ["target/release/mini_redis", "--bind", "127.0.0.1", "--port", str(mini_port)],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        cwd=Path.cwd(),
    )
    try:
        if not wait_tcp("127.0.0.1", mini_port):
            raise RuntimeError(f"mini-redis on port {mini_port} not ready")
        run_capture(
            f"python3 scripts/bench_scripting_mt.py "
            f"--host 127.0.0.1 --port {mini_port} "
            f"--suite tests/scripting_js_faithful/bench_suite.json "
            f"--threads {threads} --total {total} --warmup {warmup}",
            js_log,
        )
    finally:
        mini.terminate()
        try:
            mini.wait(timeout=3)
        except subprocess.TimeoutExpired:
            mini.kill()

    lua_summary = load_summary(lua_log)
    js_summary = load_summary(js_log)

    lua_rps = {r["name"]: r["rps"] for r in lua_summary["results"]}
    js_rps = {r["name"]: r["rps"] for r in js_summary["results"]}
    shared = sorted(set(lua_rps) & set(js_rps))
    ratios = {c: (js_rps[c] / lua_rps[c] if lua_rps[c] else 0.0) for c in shared}

    import statistics as st
    return {
        "round": round_idx,
        "threads": threads,
        "total_per_case": total,
        "redis_port": redis_port,
        "mini_port": mini_port,
        "lua_rps": lua_rps,
        "js_rps": js_rps,
        "ratios": ratios,
        "avg_ratio": st.mean(ratios.values()) if ratios else 0.0,
    }


def render_report(report: dict) -> str:
    lines = []
    lines.append("=" * 70)
    lines.append("Multi-threaded Lua vs JS scripting benchmark")
    lines.append(f"Threads: {report['threads']}  |  Requests/case: {report['total_per_case']:,}  |  Rounds: {report['rounds']}")
    lines.append(f"Generated: {report['generated_at']}")
    lines.append("=" * 70)
    lines.append("")

    for r in report["results"]:
        lines.append(f"Round {r['round']}: avg ratio = {r['avg_ratio']:.2f}x")

    lines.append("")
    header = f"{'Case':20} {'Lua RPS':>12} {'JS RPS':>12} {'Ratio':>8}"
    lines.append(header)
    lines.append("-" * len(header))

    agg = report["aggregate"]
    for case, s in agg["case_summary"].items():
        lines.append(f"{case:20} {s['lua_rps_mean']:>12,.0f} {s['js_rps_mean']:>12,.0f} {s['ratio_mean']:>7.2f}x")

    ov = agg["overall"]
    lines.append("")
    lines.append(f"Overall: {ov['mean']:.2f}x (median {ov['median']:.2f}x)")
    lines.append("")
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description="MT Lua vs JS comparison benchmark")
    parser.add_argument("--rounds", type=int, default=1)
    parser.add_argument("--threads", type=int, default=8)
    parser.add_argument("--total", type=int, default=1_000_000)
    parser.add_argument("--warmup", type=int, default=200)
    parser.add_argument("--redis-base-port", type=int, default=6390)
    parser.add_argument("--log-dir", default="tmp/comparison/mt_bench")
    parser.add_argument("--out", required=True, help="Output JSON path")
    args = parser.parse_args()

    log_dir = Path(args.log_dir)
    log_dir.mkdir(parents=True, exist_ok=True)
    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)

    subprocess.run("cargo build --release --features mini-redis --bin mini_redis", shell=True, check=True)

    results = []
    for idx in range(args.rounds):
        port = args.redis_base_port + idx
        r = run_round(idx + 1, port, args.threads, args.total, args.warmup, log_dir)
        results.append(r)

    import statistics as st

    cases = sorted(results[0]["ratios"].keys()) if results else []
    case_summary = {}
    for case in cases:
        ratio_vals = [r["ratios"][case] for r in results]
        lua_vals = [r["lua_rps"][case] for r in results]
        js_vals = [r["js_rps"][case] for r in results]
        case_summary[case] = {
            "ratio_mean": st.mean(ratio_vals),
            "ratio_median": st.median(ratio_vals),
            "lua_rps_mean": st.mean(lua_vals),
            "js_rps_mean": st.mean(js_vals),
        }

    avg_vals = [r["avg_ratio"] for r in results]
    report = {
        "generated_at": time.strftime("%Y-%m-%d %H:%M:%S"),
        "rounds": args.rounds,
        "threads": args.threads,
        "total_per_case": args.total,
        "results": results,
        "aggregate": {
            "case_summary": case_summary,
            "overall": {
                "mean": st.mean(avg_vals) if avg_vals else 0.0,
                "median": st.median(avg_vals) if avg_vals else 0.0,
            },
        },
    }

    out_path.write_text(json.dumps(report, indent=2) + "\n")
    txt = render_report(report)
    txt_path = out_path.with_suffix(".txt")
    txt_path.write_text(txt)
    print(txt, end="")
    print(f"\nREPORT_JSON={out_path}")
    print(f"REPORT_TXT={txt_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
