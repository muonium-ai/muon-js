#!/usr/bin/env bash
# run_perf_benchmark.sh
#
# Runs redis-benchmark (-c 50 -n 1,000,000 -P 16) against both Redis and
# muoncache, writes CSV logs to tmp/, then prints a comparison table.
# Each server is benchmarked --runs times and the best RPS per test is
# used for the final report (eliminates JIT warm-up and OS scheduling noise).
#
# Usage:
#   ./tests/run_perf_benchmark.sh [--mini-port PORT] [--redis-port PORT]
#                                 [--clients N] [--requests N] [--pipeline N]
#                                 [--runs N]  (default: 5)
#                                 [--no-redis] [--baseline FILE] [--max-regression F]
#
# Exit codes:
#   0  pass (or --no-redis mode where only muoncache is benchmarked)
#   1  regression detected (muoncache slower than baseline beyond threshold)
#   2  setup / tool error

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
TOOLS_DIR="$PROJECT_ROOT/tools"
MINI_BINARY="$PROJECT_ROOT/target/release/muon_cache"

# ── defaults ────────────────────────────────────────────────────────────────
MINI_PORT=6380
REDIS_PORT=6379
CLIENTS=50
REQUESTS=1000000
PIPELINE=16
TESTS="get,set,incr,lpush,rpush,lpop,rpop,sadd,hset"
NO_REDIS=0
RUNS=5
BASELINE=""
MAX_REGRESSION=0.10   # 10% throughput drop relative to baseline triggers failure
START_MINI=1          # start our own muoncache instance
START_REDIS=0         # start our own redis-server instance

# ── argument parsing ─────────────────────────────────────────────────────────
while [[ $# -gt 0 ]]; do
    case "$1" in
        --mini-port)      MINI_PORT="$2";       shift 2 ;;
        --redis-port)     REDIS_PORT="$2";      shift 2 ;;
        --clients)        CLIENTS="$2";         shift 2 ;;
        --requests)       REQUESTS="$2";        shift 2 ;;
        --pipeline)       PIPELINE="$2";        shift 2 ;;
        --tests)          TESTS="$2";           shift 2 ;;
        --runs)           RUNS="$2";            shift 2 ;;
        --no-redis)       NO_REDIS=1;           shift   ;;
        --baseline)       BASELINE="$2";        shift 2 ;;
        --max-regression) MAX_REGRESSION="$2";  shift 2 ;;
        --no-start-mini)  START_MINI=0;         shift   ;;
        --start-redis)    START_REDIS=1;        shift   ;;
        *) echo "Unknown option: $1" >&2; exit 2 ;;
    esac
done

# ── colours ──────────────────────────────────────────────────────────────────
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'
BLUE='\033[0;34m'; BOLD='\033[1m'; NC='\033[0m'

log()  { echo -e "${BLUE}[bench]${NC} $*"; }
ok()   { echo -e "${GREEN}[ok]${NC} $*"; }
warn() { echo -e "${YELLOW}[warn]${NC} $*"; }
fail() { echo -e "${RED}[fail]${NC} $*" >&2; }

# ── pre-flight checks ────────────────────────────────────────────────────────
if ! command -v redis-benchmark &>/dev/null; then
    fail "redis-benchmark not found. Install redis-tools (apt) or redis (brew)."
    exit 2
fi

mkdir -p "$PROJECT_ROOT/tmp"

TS=$(date +%Y%m%d_%H%M%S)
MINI_LOG="$PROJECT_ROOT/tmp/muon_cache_pipelined_bench_${TS}.log"
REDIS_LOG="$PROJECT_ROOT/tmp/redis_pipelined_bench_${TS}.log"
COMPARE_OUT="$PROJECT_ROOT/tmp/benchmark_comparison_${TS}.txt"

MINI_PID=""
REDIS_PID=""

cleanup() {
    if [[ -n "$MINI_PID" ]]; then
        log "Stopping muoncache (pid=$MINI_PID)"
        kill "$MINI_PID" 2>/dev/null || true
        wait "$MINI_PID" 2>/dev/null || true
    fi
    if [[ -n "$REDIS_PID" ]]; then
        log "Stopping redis-server (pid=$REDIS_PID)"
        kill "$REDIS_PID" 2>/dev/null || true
        wait "$REDIS_PID" 2>/dev/null || true
    fi
}
trap cleanup EXIT

wait_for_port() {
    local host="$1" port="$2" retries=40
    while [[ $retries -gt 0 ]]; do
        if python3 -c "
import socket, sys
s = socket.socket()
s.settimeout(0.2)
rc = s.connect_ex(('$host', $port))
s.close()
sys.exit(0 if rc == 0 else 1)
" 2>/dev/null; then
            return 0
        fi
        retries=$((retries-1))
        sleep 0.25
    done
    return 1
}

# ── build muoncache ─────────────────────────────────────────────────────────
if [[ "$START_MINI" -eq 1 ]]; then
    log "Building muoncache (release)…"
    cd "$PROJECT_ROOT"
    cargo build --release --features muoncache --bin muon_cache 2>&1 \
        | grep -E "(Compiling|Finished|error)" || true

    if [[ ! -f "$MINI_BINARY" ]]; then
        fail "muoncache binary not found at $MINI_BINARY"
        exit 2
    fi
fi

# ── start muoncache ─────────────────────────────────────────────────────────
if [[ "$START_MINI" -eq 1 ]]; then
    # fail fast if port is already occupied
    if wait_for_port 127.0.0.1 "$MINI_PORT" 2>/dev/null; then
        fail "Port $MINI_PORT is already in use. Stop the existing server or use --mini-port."
        exit 2
    fi
    log "Starting muoncache on port $MINI_PORT"
    "$MINI_BINARY" --port "$MINI_PORT" 2>/dev/null &
    MINI_PID=$!
    if ! wait_for_port 127.0.0.1 "$MINI_PORT"; then
        fail "muoncache did not start on port $MINI_PORT"
        exit 2
    fi
    ok "muoncache running (pid=$MINI_PID)"
fi

# ── start redis-server (optional) ────────────────────────────────────────────
if [[ "$NO_REDIS" -eq 0 && "$START_REDIS" -eq 1 ]]; then
    if ! command -v redis-server &>/dev/null; then
        warn "redis-server not available; skipping Redis comparison"
        NO_REDIS=1
    else
        log "Starting redis-server on port $REDIS_PORT"
        redis-server --port "$REDIS_PORT" --daemonize no --loglevel warning 2>/dev/null &
        REDIS_PID=$!
        if ! wait_for_port 127.0.0.1 "$REDIS_PORT"; then
            fail "redis-server did not start on port $REDIS_PORT"
            exit 2
        fi
        ok "redis-server running (pid=$REDIS_PID)"
    fi
fi

# ── benchmark helpers ────────────────────────────────────────────────────────

# run one redis-benchmark pass, return CSV to stdout
_bench_once() {
    local port="$1"
    redis-benchmark -h 127.0.0.1 -p "$port" \
        -c "$CLIENTS" -n "$REQUESTS" -P "$PIPELINE" \
        -t "$TESTS" --csv 2>/dev/null
}

# best_of_runs LABEL PORT RUNS OUTFILE
# Runs redis-benchmark RUNS times, picks the highest RPS per test,
# writes a merged best-of CSV to OUTFILE.
best_of_runs() {
    local label="$1" port="$2" runs="$3" outfile="$4"
    local tmpdir
    tmpdir=$(mktemp -d)
    log "Benchmarking $label  (port $port  c=$CLIENTS n=$REQUESTS P=$PIPELINE  runs=$runs)"
    local i
    for (( i=1; i<=runs; i++ )); do
        log "  run $i/$runs …"
        if ! _bench_once "$port" > "$tmpdir/run_${i}.csv" 2>/dev/null; then
            warn "  run $i/$runs failed (redis-benchmark exited non-zero); skipping"
            rm -f "$tmpdir/run_${i}.csv"
        fi
    done
    # merge: pick max RPS per test across all run files
    python3 - "$tmpdir" "$outfile" <<'PYEOF'
import sys, csv, os, glob

tmpdir, outfile = sys.argv[1], sys.argv[2]
best = {}
for fpath in sorted(glob.glob(os.path.join(tmpdir, 'run_*.csv'))):
    with open(fpath) as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                row = next(csv.reader([line]))
            except Exception:
                continue
            if len(row) < 2:
                continue
            test = row[0].strip('"').split()[0].upper()
            try:
                rps = float(row[1].strip('"'))
            except ValueError:
                continue
            if test not in best or rps > best[test]:
                best[test] = rps
# write merged best-of CSV
with open(outfile, 'w') as out:
    for test, rps in best.items():
        out.write(f'"{test}","{rps:.2f}"\n')
PYEOF
    rm -rf "$tmpdir"
    log "  best-of-$runs log → $outfile"
}

# ── run benchmarks ────────────────────────────────────────────────────────────
echo ""
echo -e "${BOLD}=== muoncache benchmark (best of $RUNS runs) ===${NC}"
best_of_runs "muoncache" "$MINI_PORT" "$RUNS" "$MINI_LOG"

if [[ "$NO_REDIS" -eq 0 ]]; then
    # verify Redis is reachable (may be externally started)
    if ! wait_for_port 127.0.0.1 "$REDIS_PORT"; then
        warn "Redis not reachable on port $REDIS_PORT; skipping comparison"
        NO_REDIS=1
    else
        echo ""
        echo -e "${BOLD}=== Redis benchmark (best of $RUNS runs) ===${NC}"
        best_of_runs "redis" "$REDIS_PORT" "$RUNS" "$REDIS_LOG"
    fi
fi

# ── comparison table ──────────────────────────────────────────────────────────
echo ""
echo -e "${BOLD}=== Results ===${NC}"

if [[ "$NO_REDIS" -eq 0 ]]; then
    python3 - "$MINI_LOG" "$REDIS_LOG" "$COMPARE_OUT" <<'PYEOF'
import sys, csv, re

def parse_csv_bench(path):
    """Parse redis-benchmark --csv output → {test: rps}"""
    data = {}
    with open(path) as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                row = next(csv.reader([line]))
            except Exception:
                continue
            if len(row) < 2:
                continue
            test = row[0].strip('"').split()[0].upper()
            try:
                data[test] = float(row[1].strip('"'))
            except ValueError:
                pass
    return data

mini_path, redis_path, out_path = sys.argv[1], sys.argv[2], sys.argv[3]
mini = parse_csv_bench(mini_path)
redis = parse_csv_bench(redis_path)

order = ["GET","SET","INCR","LPUSH","RPUSH","LPOP","RPOP","SADD","HSET"]
tests = [t for t in order if t in mini or t in redis]

hdr  = f"  {'TEST':8}  {'muoncache':>14}  {'redis':>14}  {'muon/redis':>10}"
sep  = "  " + "-"*8 + "  " + "-"*14 + "  " + "-"*14 + "  " + "-"*10
rows = [
    "\nredis-benchmark  -c {c} -n {n} -P {p}  (requests/sec)".format(
        c=50, n="1,000,000", p=16),
    "=" * 60,
    hdr, sep
]
for t in tests:
    mn = mini.get(t)
    rn = redis.get(t)
    mn_s = f"{mn:>14,.0f}" if mn else f"{'n/a':>14}"
    rn_s = f"{rn:>14,.0f}" if rn else f"{'n/a':>14}"
    if mn and rn:
        ratio = mn / rn
        flag = "  <<" if ratio < 0.90 else ("  !!" if ratio > 1.10 else "    ")
        ratio_s = f"{ratio:>9.2f}x{flag}"
    else:
        ratio_s = f"{'n/a':>10}"
    rows.append(f"  {t:8}  {mn_s}  {rn_s}  {ratio_s}")

rows += [
    "",
    "  muon/redis > 1.0x  →  muoncache is FASTER",
    "  muon/redis < 1.0x  →  muoncache is SLOWER  (<<) flagged if >10% slower",
    "",
]
report = "\n".join(rows)
print(report)
with open(out_path, "w") as f:
    f.write(report + "\n")
print(f"\nComparison saved to: {out_path}")
PYEOF
else
    # mini-only: just pretty-print the CSV
    python3 - "$MINI_LOG" <<'PYEOF'
import sys, csv

def parse_csv_bench(path):
    data = {}
    with open(path) as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                row = next(csv.reader([line]))
            except Exception:
                continue
            if len(row) < 2:
                continue
            test = row[0].strip('"').split()[0].upper()
            try:
                data[test] = float(row[1].strip('"'))
            except ValueError:
                pass
    return data

mini = parse_csv_bench(sys.argv[1])
order = ["GET","SET","INCR","LPUSH","RPUSH","LPOP","RPOP","SADD","HSET"]
tests = [t for t in order if t in mini]
print(f"\n  {'TEST':8}  {'muoncache':>14} (requests/sec)")
print("  " + "-"*8 + "  " + "-"*14)
for t in tests:
    print(f"  {t:8}  {mini[t]:>14,.0f}")
PYEOF
fi

# ── optional baseline regression check ───────────────────────────────────────
if [[ -n "$BASELINE" ]]; then
    log "Checking regression against baseline: $BASELINE"
    python3 "$TOOLS_DIR/check_js_runtime_bench.py" \
        --baseline "$BASELINE" \
        --current "$MINI_LOG" \
        --max-regression "$MAX_REGRESSION" || {
        fail "Performance regression detected (threshold: ${MAX_REGRESSION})"
        exit 1
    }
    ok "No regressions beyond ${MAX_REGRESSION} threshold"
fi

ok "Benchmark complete"
ok "  muoncache log : $MINI_LOG"
if [[ "$NO_REDIS" -eq 0 ]]; then
    ok "  redis log      : $REDIS_LOG"
fi
