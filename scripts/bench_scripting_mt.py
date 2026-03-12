#!/usr/bin/env python3
"""Multi-threaded scripting benchmark — 1M EVAL requests across N connections.

Each thread opens its own TCP socket and sends EVAL commands in a tight loop.
Keys are sharded per-thread to avoid cross-thread contention on the same key.

Usage:
    python3 scripts/bench_scripting_mt.py \
        --host 127.0.0.1 --port 6379 \
        --suite tests/scripting/bench_suite.json \
        --threads 8 --total 1000000 --warmup 200
"""
import argparse
import json
import socket
import threading
import time
from pathlib import Path


def resp_encode(args):
    out = b"*" + str(len(args)).encode() + b"\r\n"
    for arg in args:
        data = arg if isinstance(arg, bytes) else str(arg).encode()
        out += b"$" + str(len(data)).encode() + b"\r\n" + data + b"\r\n"
    return out


class RespReader:
    """Buffered RESP reader — much faster than byte-by-byte recv."""
    __slots__ = ("_f",)

    def __init__(self, sock):
        self._f = sock.makefile("rb")

    def read(self):
        line = self._f.readline()
        if not line:
            return None
        prefix = chr(line[0])
        payload = line[1:].rstrip(b"\r\n")
        if prefix == "+":
            return ("simple", payload.decode())
        if prefix == "-":
            return ("error", payload.decode())
        if prefix == ":":
            return ("int", int(payload))
        if prefix == "_":
            return ("null", None)
        if prefix == "$":
            ln = int(payload)
            if ln < 0:
                return ("null", None)
            data = self._f.read(ln)
            self._f.read(2)  # trailing \r\n
            return ("blob", data)
        if prefix == "*":
            ln = int(payload)
            if ln < 0:
                return ("null", None)
            return ("array", [self.read() for _ in range(ln)])
        return ("error", "ERR unknown RESP type")

    def close(self):
        self._f.close()


def worker(host, port, script, numkeys, keys, argv, iters, warmup, tid, results):
    """Run EVAL in a tight loop on a dedicated connection."""
    thread_keys = [f"{k}:t{tid}" for k in keys]
    errors = 0
    try:
        sock = socket.create_connection((host, port))
        reader = RespReader(sock)
        cmd = resp_encode(["EVAL", script, str(numkeys)] + thread_keys + [str(a) for a in argv])
        # Warmup
        for _ in range(warmup):
            sock.sendall(cmd)
            resp = reader.read()
            if resp and resp[0] == "error":
                errors += 1

        start = time.perf_counter()
        for _ in range(iters):
            sock.sendall(cmd)
            resp = reader.read()
            if resp is None:
                errors += 1
            elif resp[0] == "error":
                errors += 1
        elapsed = time.perf_counter() - start
        reader.close()
        sock.close()
    except Exception as exc:
        elapsed = 0.0
        errors = iters
        print(f"  [thread-{tid}] error: {exc}")

    results[tid] = {"iters": iters, "elapsed": elapsed, "errors": errors}


def run_case_mt(host, port, case, num_threads, total_iters, warmup):
    script_path = Path(case["script"])
    script = script_path.read_text()
    numkeys = int(case.get("numkeys", 0))
    keys = case.get("keys", [])
    argv = case.get("argv", [])

    per_thread = total_iters // num_threads
    remainder = total_iters - per_thread * num_threads

    results = [None] * num_threads
    threads = []
    for tid in range(num_threads):
        t_iters = per_thread + (1 if tid < remainder else 0)
        t = threading.Thread(
            target=worker,
            args=(host, port, script, numkeys, keys, argv, t_iters, warmup, tid, results),
        )
        threads.append(t)

    wall_start = time.perf_counter()
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    wall_elapsed = time.perf_counter() - wall_start

    total_done = sum(r["iters"] for r in results if r)
    total_errors = sum(r["errors"] for r in results if r)
    aggregate_rps = total_done / wall_elapsed if wall_elapsed > 0 else 0.0

    return {
        "name": case["name"],
        "threads": num_threads,
        "total_iters": total_done,
        "wall_seconds": wall_elapsed,
        "rps": aggregate_rps,
        "errors": total_errors,
    }


def main():
    parser = argparse.ArgumentParser(description="Multi-threaded EVAL benchmark")
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--suite", required=True, help="Path to bench suite JSON")
    parser.add_argument("--threads", type=int, default=8, help="Number of concurrent connections")
    parser.add_argument("--total", type=int, default=1_000_000, help="Total requests per case")
    parser.add_argument("--warmup", type=int, default=200, help="Warmup iterations per thread")
    parser.add_argument("--cases", nargs="+", default=None, help="Subset of case names to run")
    parser.add_argument("--out-json", default=None, help="Write JSON report to file")
    args = parser.parse_args()

    suite = json.loads(Path(args.suite).read_text())
    cases = suite["cases"]
    if args.cases:
        selected = set(args.cases)
        cases = [c for c in cases if c["name"] in selected]

    print(f"Multi-threaded EVAL benchmark: {args.threads} threads, {args.total:,} requests/case")
    print(f"Server: {args.host}:{args.port}")
    print(f"Suite:  {args.suite} ({len(cases)} cases)")
    print()

    results = []
    for case in cases:
        res = run_case_mt(args.host, args.port, case, args.threads, args.total, args.warmup)
        err_note = f"  ({res['errors']} errors)" if res["errors"] else ""
        print(f"{res['name']:15s} {res['rps']:>12,.0f} rps  ({res['total_iters']:,} iters, "
              f"{res['wall_seconds']:.2f}s, {res['threads']}T){err_note}")
        results.append(res)

    total_req = sum(r["total_iters"] for r in results)
    total_wall = sum(r["wall_seconds"] for r in results)
    total_err = sum(r["errors"] for r in results)
    print()
    print(f"Total: {total_req:,} requests in {total_wall:.2f}s, {total_err} errors")

    summary = {
        "host": args.host,
        "port": args.port,
        "suite": args.suite,
        "threads": args.threads,
        "total_per_case": args.total,
        "warmup_per_thread": args.warmup,
        "results": results,
    }

    if args.out_json:
        p = Path(args.out_json)
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(json.dumps(summary, indent=2) + "\n")

    print("\nSUMMARY_JSON=" + json.dumps(summary))


if __name__ == "__main__":
    main()
