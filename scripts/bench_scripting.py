#!/usr/bin/env python3
import argparse
import csv
import json
import socket
import time
from pathlib import Path


def resp_encode(args):
    out = b"*" + str(len(args)).encode() + b"\r\n"
    for arg in args:
        if isinstance(arg, bytes):
            data = arg
        else:
            data = str(arg).encode()
        out += b"$" + str(len(data)).encode() + b"\r\n" + data + b"\r\n"
    return out


def resp_read(sock):
    def read_line():
        buf = b""
        while not buf.endswith(b"\r\n"):
            chunk = sock.recv(1)
            if not chunk:
                return None
            buf += chunk
        return buf[:-2]

    prefix = sock.recv(1)
    if not prefix:
        return None
    if prefix == b"+":
        return ("simple", read_line().decode())
    if prefix == b"-":
        return ("error", read_line().decode())
    if prefix == b":":
        return ("int", int(read_line()))
    if prefix == b"_":
        _ = read_line()
        return ("null", None)
    if prefix == b"$":
        ln = int(read_line())
        if ln < 0:
            return ("null", None)
        data = b""
        while len(data) < ln:
            data += sock.recv(ln - len(data))
        _ = sock.recv(2)
        return ("blob", data)
    if prefix == b"*":
        ln = int(read_line())
        if ln < 0:
            return ("null", None)
        arr = []
        for _ in range(ln):
            arr.append(resp_read(sock))
        return ("array", arr)
    return ("error", "ERR unknown RESP type")


def send_cmd(sock, *args):
    sock.sendall(resp_encode(args))
    return resp_read(sock)


def run_case(sock, case, global_iters=None, warmup=100):
    script_path = Path(case["script"])
    script = script_path.read_text()
    numkeys = int(case.get("numkeys", 0))
    keys = case.get("keys", [])
    argv = case.get("argv", [])
    iters = int(global_iters or case.get("iterations", 1000))

    if numkeys != len(keys):
        raise ValueError(f"numkeys mismatch for {case['name']}: expected {numkeys}, got {len(keys)}")

    # Warmup
    for warm_idx in range(max(0, warmup)):
        resp = send_cmd(sock, "EVAL", script, numkeys, *keys, *argv)
        if resp is None:
            raise RuntimeError(f"No response during warmup for {case['name']} at {warm_idx}")
        if resp[0] == "error":
            raise RuntimeError(
                f"Server error during warmup for {case['name']} at {warm_idx}: {resp[1]}"
            )

    start = time.perf_counter()
    for idx in range(iters):
        resp = send_cmd(sock, "EVAL", script, numkeys, *keys, *argv)
        if resp is None:
            raise RuntimeError(f"No response for {case['name']} at {idx}")
        if resp[0] == "error":
            raise RuntimeError(f"Server error for {case['name']} at {idx}: {resp[1]}")
    elapsed = time.perf_counter() - start
    rps = iters / elapsed if elapsed > 0 else 0.0
    return {
        "name": case["name"],
        "iterations": iters,
        "seconds": elapsed,
        "rps": rps,
    }


def main():
    parser = argparse.ArgumentParser(description="Benchmark scripting EVAL performance")
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--suite", required=True, help="Path to bench suite JSON")
    parser.add_argument("--iterations", type=int, default=None, help="Override per-test iterations")
    parser.add_argument("--warmup", type=int, default=100)
    parser.add_argument(
        "--cases",
        nargs="+",
        default=None,
        help="Optional list of case names to run (default: all cases in suite)",
    )
    parser.add_argument("--out-json", default=None, help="Optional path to write summary JSON")
    parser.add_argument("--out-csv", default=None, help="Optional path to write per-case CSV")
    args = parser.parse_args()

    suite_path = Path(args.suite)
    suite = json.loads(suite_path.read_text())
    cases = suite["cases"]
    if args.cases:
        selected = set(args.cases)
        cases = [case for case in cases if case.get("name") in selected]
        missing = selected - {case.get("name") for case in cases}
        if missing:
            raise ValueError(f"Unknown benchmark case(s): {', '.join(sorted(missing))}")

    sock = socket.create_connection((args.host, args.port))

    results = []
    for case in cases:
        res = run_case(sock, case, global_iters=args.iterations, warmup=args.warmup)
        results.append(res)
        print(f"{res['name']}: {res['rps']:.2f} rps ({res['iterations']} iters, {res['seconds']:.3f}s)")

    sock.close()

    summary = {
        "host": args.host,
        "port": args.port,
        "suite": str(suite_path),
        "warmup": args.warmup,
        "iterations_override": args.iterations,
        "results": results,
    }

    if args.out_json:
        out_json_path = Path(args.out_json)
        out_json_path.parent.mkdir(parents=True, exist_ok=True)
        out_json_path.write_text(json.dumps(summary, indent=2) + "\n")

    if args.out_csv:
        out_csv_path = Path(args.out_csv)
        out_csv_path.parent.mkdir(parents=True, exist_ok=True)
        with out_csv_path.open("w", newline="") as f:
            writer = csv.writer(f)
            writer.writerow(["name", "iterations", "seconds", "rps"])
            for row in results:
                writer.writerow([row["name"], row["iterations"], f"{row['seconds']:.9f}", f"{row['rps']:.6f}"])

    print("\nSUMMARY_JSON=" + json.dumps(summary))


if __name__ == "__main__":
    main()
