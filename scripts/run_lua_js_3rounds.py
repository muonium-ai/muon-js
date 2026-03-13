import json
import re
import statistics
import subprocess
import time
from pathlib import Path
import socket

ROOT = Path('.').resolve()
TMP = ROOT / 'tmp'
summary_re = re.compile(r'^SUMMARY_JSON=(\{.*\})$')


def run_cmd(cmd: str):
    print(f"\n>>> {cmd}", flush=True)
    subprocess.run(cmd, shell=True, check=True)


def redis_ready(port: int, timeout_s: float = 20.0) -> bool:
    deadline = time.time() + timeout_s
    while time.time() < deadline:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.5) as s:
                s.sendall(b"*1\r\n$4\r\nPING\r\n")
                data = s.recv(128)
                if b"+PONG" in data:
                    out = subprocess.run(
                        f"redis-cli -p {port} INFO persistence",
                        shell=True,
                        check=False,
                        capture_output=True,
                        text=True,
                    )
                    if "loading:0" in out.stdout:
                        return True
        except Exception:
            pass
        time.sleep(0.2)
    return False


def latest(pattern: str) -> Path:
    files = sorted(TMP.glob(pattern), key=lambda p: p.stat().st_mtime)
    if not files:
        raise RuntimeError(f'No files for pattern: {pattern}')
    return files[-1]


def load_summary(log_path: Path):
    text = log_path.read_text(errors='ignore')
    for line in reversed(text.splitlines()):
        m = summary_re.match(line.strip())
        if m:
            payload = json.loads(m.group(1))
            return {r['name']: float(r['rps']) for r in payload['results']}
    raise RuntimeError(f'No SUMMARY_JSON found in {log_path}')


def main():
    rounds = []
    start = time.time()

    for i in range(1, 4):
        redis_port = 6385 + i
        redis_pidfile = TMP / f"redis_round_{i}.pid"
        redis_log = TMP / f"redis_round_{i}.log"
        run_cmd(f"redis-server --port {redis_port} --daemonize yes --pidfile {redis_pidfile} --logfile {redis_log}")
        if not redis_ready(redis_port):
            raise RuntimeError(f"Redis on port {redis_port} did not become ready")

        lua_ts = time.strftime('%Y%m%d_%H%M%S')
        lua_log_path = TMP / f"redis_lua_script_bench_{lua_ts}.log"
        run_cmd(
            f"python3 scripts/bench_scripting.py --host 127.0.0.1 --port {redis_port} --suite tests/scripting/bench_suite.json | tee {lua_log_path}"
        )
        run_cmd(f"redis-cli -p {redis_port} SHUTDOWN NOSAVE || true")

        run_cmd('make muoncache-js-scripting-bench')

        lua_log = latest('redis_lua_script_bench_*.log')
        js_log = latest('muon_cache_js_faithful_bench_*.log')
        lua = load_summary(lua_log)
        js = load_summary(js_log)
        shared = sorted(set(lua) & set(js))

        ratios = {k: (js[k] / lua[k] if lua[k] else 0.0) for k in shared}
        rounds.append(
            {
                'round': i,
                'redis_port': redis_port,
                'lua_log': lua_log.name,
                'js_log': js_log.name,
                'ratios': ratios,
                'avg_ratio': sum(ratios.values()) / len(ratios) if ratios else 0.0,
            }
        )

    cases = sorted(rounds[0]['ratios'].keys())

    lines = []
    lines.append('Lua vs JavaScript scripting benchmark (3-run comparison)')
    lines.append(f"generated_at={time.strftime('%Y-%m-%d %H:%M:%S')}")
    lines.append(f"total_seconds={time.time()-start:.2f}")
    lines.append('')

    for r in rounds:
        lines.append(f"Round {r['round']} (redis_port={r['redis_port']})")
        lines.append(f"  lua_log={r['lua_log']}")
        lines.append(f"  js_log={r['js_log']}")
        lines.append(f"  avg_js_lua_ratio={r['avg_ratio']:.4f}x")
        lines.append('')

    header = f"{'CASE':20} {'R1':>8} {'R2':>8} {'R3':>8} {'MEAN':>8} {'MEDIAN':>8}"
    lines.append(header)
    lines.append('-' * len(header))
    for case in cases:
        vals = [r['ratios'][case] for r in rounds]
        mean_v = statistics.mean(vals)
        med_v = statistics.median(vals)
        lines.append(f"{case:20} {vals[0]:8.2f} {vals[1]:8.2f} {vals[2]:8.2f} {mean_v:8.2f} {med_v:8.2f}")

    avg_vals = [r['avg_ratio'] for r in rounds]
    lines.append('')
    lines.append(f"overall_avg_ratio_mean={statistics.mean(avg_vals):.4f}x")
    lines.append(f"overall_avg_ratio_median={statistics.median(avg_vals):.4f}x")

    out = TMP / f"lua_js_comparison_3runs_{time.strftime('%Y%m%d_%H%M%S')}.txt"
    out.write_text('\n'.join(lines) + '\n')
    print(f"\nREPORT_PATH={out}")
    print('\n'.join(lines))


if __name__ == '__main__':
    main()
