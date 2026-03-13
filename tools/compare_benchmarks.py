import argparse
import re
import time
from pathlib import Path

section_re = re.compile(r'^======\s+(.*?)\s+======\s*$')
throughput_re = re.compile(r'throughput summary:\s*([0-9.]+)\s+requests per second')


def parse_log(path: Path):
    data = {}
    current = None
    for line in path.read_text(errors='ignore').splitlines():
        m = section_re.match(line.strip())
        if m:
            current = m.group(1).strip()
            continue
        if current:
            t = throughput_re.search(line)
            if t:
                data[current] = float(t.group(1))
                current = None
    return data


def norm(k: str):
    return re.sub(r'\s+', ' ', k.strip().upper())
def latest_log(tmp_dir: Path, pattern: str) -> Path:
    candidates = list(tmp_dir.glob(pattern))
    if not candidates:
        raise FileNotFoundError(f"No files found for pattern {pattern} in {tmp_dir}")
    return max(candidates, key=lambda p: p.stat().st_mtime)


def main():
    repo_root = Path(__file__).resolve().parents[1]
    tmp_dir = repo_root / "tmp"

    parser = argparse.ArgumentParser(description="Compare redis and muoncache benchmarks")
    parser.add_argument("--mini", help="path to muoncache benchmark log")
    parser.add_argument("--redis", help="path to redis-benchmark log")
    parser.add_argument("--out", help="output report path")
    args = parser.parse_args()

    mini_path = Path(args.mini) if args.mini else latest_log(tmp_dir, "muon_cache_benchmark_*.log")
    redis_path = Path(args.redis) if args.redis else latest_log(tmp_dir, "redis_benchmark_*.log")

    mini = parse_log(mini_path)
    redis = parse_log(redis_path)

    mini_norm = {norm(k): (k, v) for k, v in mini.items()}
    redis_norm = {norm(k): (k, v) for k, v in redis.items()}

    common = sorted(set(mini_norm) & set(redis_norm))
    rows = []
    for nk in common:
        mk, mv = mini_norm[nk]
        rk, rv = redis_norm[nk]
        ratio = (rv / mv) if mv else None
        rows.append((mk, mv, rv, ratio))

    out_lines = []
    out_lines.append('Benchmark comparison: redis-benchmark vs muoncache benchmark')
    out_lines.append(f'mini log: {mini_path.name}')
    out_lines.append(f'redis log: {redis_path.name}')
    out_lines.append('')
    out_lines.append('Legend: ratio = redis_rps / mini_rps; e.g., 5.0x means mini is 5x slower')
    out_lines.append('')
    out_lines.append('Results (common tests):')

    if not rows:
        out_lines.append('No common tests found.')
    else:
        header = f"{'TEST':30} {'MINI_RPS':>12} {'REDIS_RPS':>12} {'SLOWER_BY':>10}"
        out_lines.append(header)
        out_lines.append('-' * len(header))
        for name, mv, rv, ratio in rows:
            ratio_str = 'n/a' if ratio is None else f'{ratio:.2f}x'
            out_lines.append(f"{name:30} {mv:12.2f} {rv:12.2f} {ratio_str:>10}")

    if args.out:
        out_path = Path(args.out)
    else:
        out_path = tmp_dir / f"benchmark_comparison_{time.strftime('%Y%m%d_%H%M%S')}.txt"
    out_path.write_text('\n'.join(out_lines))
    print(out_path)


if __name__ == "__main__":
    main()
