#!/usr/bin/env python3
import argparse
import socket
import threading
import time
from typing import Callable, List, Tuple


def resp_encode(args: List[bytes]) -> bytes:
    out = b"*" + str(len(args)).encode() + b"\r\n"
    for arg in args:
        if isinstance(arg, bytes):
            data = arg
        else:
            data = str(arg).encode()
        out += b"$" + str(len(data)).encode() + b"\r\n" + data + b"\r\n"
    return out


class RespReader:
    def __init__(self, sock: socket.socket):
        self.sock = sock
        self.buf = bytearray()

    def _fill(self) -> bool:
        chunk = self.sock.recv(4096)
        if not chunk:
            return False
        self.buf.extend(chunk)
        return True

    def read_exact(self, n: int) -> bytes:
        while len(self.buf) < n:
            if not self._fill():
                return b""
        out = bytes(self.buf[:n])
        del self.buf[:n]
        return out

    def read_line(self) -> bytes:
        while True:
            idx = self.buf.find(b"\r\n")
            if idx != -1:
                out = bytes(self.buf[:idx])
                del self.buf[:idx + 2]
                return out
            if not self._fill():
                return b""


def resp_read(reader: RespReader):
    prefix = reader.read_exact(1)
    if not prefix:
        return None
    if prefix == b"+":
        return ("simple", reader.read_line().decode())
    if prefix == b"-":
        return ("error", reader.read_line().decode())
    if prefix == b":":
        line = reader.read_line()
        return ("int", int(line))
    if prefix == b"_":
        _ = reader.read_line()
        return ("null", None)
    if prefix == b"$":
        ln = int(reader.read_line())
        if ln < 0:
            return ("null", None)
        data = reader.read_exact(ln)
        _ = reader.read_exact(2)
        return ("blob", data)
    if prefix == b"*":
        ln = int(reader.read_line())
        if ln < 0:
            return ("null", None)
        arr = []
        for _ in range(ln):
            arr.append(resp_read(reader))
        return ("array", arr)
    return ("error", "ERR unknown RESP type")


def send_cmd(reader: RespReader, args: List[bytes]):
    reader.sock.sendall(resp_encode(args))
    return resp_read(reader)


def percentile(values: List[float], pct: float) -> float:
    if not values:
        return 0.0
    idx = int(round((pct / 100.0) * (len(values) - 1)))
    idx = min(max(idx, 0), len(values) - 1)
    return values[idx]


def latency_percentiles(values: List[float]) -> List[Tuple[float, float]]:
    cutoffs = [
        50.000, 75.000, 87.500, 93.750, 96.875, 98.438, 99.219, 99.609,
        99.805, 99.902, 99.951, 99.976, 99.988, 99.994, 99.997, 99.998,
        99.999, 100.000,
    ]
    return [(p, percentile(values, p)) for p in cutoffs]


def summarize(latencies_ms: List[float]):
    if not latencies_ms:
        return {
            "avg": 0.0,
            "min": 0.0,
            "p50": 0.0,
            "p95": 0.0,
            "p99": 0.0,
            "max": 0.0,
        }
    values = sorted(latencies_ms)
    return {
        "avg": sum(values) / len(values),
        "min": values[0],
        "p50": percentile(values, 50.0),
        "p95": percentile(values, 95.0),
        "p99": percentile(values, 99.0),
        "max": values[-1],
    }


def run_workers(
    host: str,
    port: int,
    clients: int,
    requests: int,
    cmd_builder: Callable[[int, int], List[bytes]],
) -> Tuple[List[float], int]:
    per_client = requests // clients
    remainder = requests % clients
    all_latencies: List[float] = []
    errors = 0
    lock = threading.Lock()

    def worker(cid: int, count: int):
        nonlocal errors
        local_latencies: List[float] = []
        try:
            sock = socket.create_connection((host, port))
            reader = RespReader(sock)
        except Exception:
            with lock:
                errors += count
            return
        for i in range(count):
            cmd = cmd_builder(cid, i)
            start = time.perf_counter()
            resp = send_cmd(reader, cmd)
            end = time.perf_counter()
            local_latencies.append((end - start) * 1000.0)
            if resp is None or resp[0] == "error":
                with lock:
                    errors += 1
        try:
            sock.close()
        except Exception:
            pass
        with lock:
            all_latencies.extend(local_latencies)

    threads = []
    for c in range(clients):
        count = per_client + (1 if c < remainder else 0)
        t = threading.Thread(target=worker, args=(c, count), daemon=True)
        threads.append(t)

    start = time.perf_counter()
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    elapsed = time.perf_counter() - start
    return all_latencies, errors, elapsed


def run_test(
    name: str,
    host: str,
    port: int,
    clients: int,
    requests: int,
    payload: bytes,
    setup_fn: Callable[[RespReader], None],
    cmd_builder: Callable[[int, int], List[bytes]],
):
    print(f"\n=== running {name} ===", flush=True)
    setup_sock = socket.create_connection((host, port))
    setup_reader = RespReader(setup_sock)
    try:
        setup_fn(setup_reader)
    finally:
        setup_sock.close()

    latencies_ms, errors, elapsed = run_workers(
        host, port, clients, requests, cmd_builder
    )
    rps = requests / elapsed if elapsed > 0 else 0.0
    summary = summarize(latencies_ms)

    print(f"====== {name} ======")
    print(f"  {requests} requests completed in {elapsed:.2f} seconds")
    print(f"  {clients} parallel clients")
    print(f"  {len(payload)} bytes payload")
    print("  keep alive: 1")
    print("  host configuration \"save\": 3600 1 300 100 60 10000")
    print("  host configuration \"appendonly\": no")
    print("  multi-thread: no")
    if errors:
        print(f"  errors: {errors}")
    print()
    print("Latency by percentile distribution:")
    for pct, val in latency_percentiles(sorted(latencies_ms)):
        print(f"{pct:6.3f}% <= {val:.3f} milliseconds (cumulative count {int(round(pct/100.0*requests))})")
    print()
    print("Summary:")
    print(
        "  throughput summary: "
        f"{rps:.2f} requests per second"
    )
    print("  latency summary (msec):")
    print("          avg       min       p50       p95       p99       max")
    print(
        f"        {summary['avg']:.3f}     {summary['min']:.3f}     "
        f"{summary['p50']:.3f}     {summary['p95']:.3f}     "
        f"{summary['p99']:.3f}     {summary['max']:.3f}"
    )
    print()


def main():
    parser = argparse.ArgumentParser(description="muoncache benchmark (redis-benchmark style)")
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=6379)
    parser.add_argument("-c", "--clients", type=int, default=50)
    parser.add_argument("-n", "--requests", type=int, default=100000)
    parser.add_argument("-d", "--data-size", type=int, default=3)
    parser.add_argument("-t", "--tests", default="ping,set,get,incr,lpush,rpush,lpop,rpop,sadd,hset,zadd,xadd,lrange")
    parser.add_argument("--lrange-size", type=int, default=100)
    args = parser.parse_args()

    host = args.host
    port = args.port
    clients = args.clients
    requests = args.requests
    payload = b"x" * args.data_size

    print(
        f"muoncache benchmark: host={host} port={port} clients={clients} requests={requests} data={args.data_size}B",
        flush=True,
    )
    tests = [t.strip().lower() for t in args.tests.split(",") if t.strip()]

    def setup_noop(_reader: RespReader):
        return

    def setup_set_keys(reader: RespReader, key_prefix: bytes, value: bytes = None):
        data = payload if value is None else value
        for cid in range(clients):
            key = key_prefix + str(cid).encode()
            send_cmd(reader, [b"SET", key, data])

    def setup_list(reader: RespReader, key: bytes, count: int):
        send_cmd(reader, [b"DEL", key])
        for i in range(count):
            send_cmd(reader, [b"RPUSH", key, str(i).encode()])

    def setup_set(reader: RespReader, key: bytes):
        send_cmd(reader, [b"DEL", key])
        for i in range(10):
            send_cmd(reader, [b"SADD", key, str(i).encode()])

    def setup_hash(reader: RespReader, key: bytes):
        send_cmd(reader, [b"DEL", key])
        send_cmd(reader, [b"HSET", key, b"f", b"v"])

    def setup_zset(reader: RespReader, key: bytes):
        send_cmd(reader, [b"DEL", key])
        send_cmd(reader, [b"ZADD", key, b"1", b"a", b"2", b"b"])

    def setup_stream(reader: RespReader, key: bytes):
        send_cmd(reader, [b"DEL", key])
        send_cmd(reader, [b"XADD", key, b"*", b"f", b"v0"])

    for test in tests:
        if test == "ping":
            run_test(
                "PING",
                host,
                port,
                clients,
                requests,
                payload,
                setup_noop,
                lambda _cid, _i: [b"PING"],
            )
        elif test == "set":
            key_prefix = b"bench:set:"
            run_test(
                "SET",
                host,
                port,
                clients,
                requests,
                payload,
                setup_noop,
                lambda cid, i: [b"SET", key_prefix + str(cid).encode() + b":" + str(i).encode(), payload],
            )
        elif test == "get":
            key_prefix = b"bench:get:"
            def setup(reader: RespReader):
                setup_set_keys(reader, key_prefix)
            run_test(
                "GET",
                host,
                port,
                clients,
                requests,
                payload,
                setup,
                lambda cid, _i: [b"GET", key_prefix + str(cid).encode()],
            )
        elif test == "incr":
            key_prefix = b"bench:incr:"
            def setup(reader: RespReader):
                setup_set_keys(reader, key_prefix, b"0")
            run_test(
                "INCR",
                host,
                port,
                clients,
                requests,
                payload,
                setup,
                lambda cid, _i: [b"INCR", key_prefix + str(cid).encode()],
            )
        elif test == "lpush":
            key = b"bench:list:lpush"
            run_test(
                "LPUSH",
                host,
                port,
                clients,
                requests,
                payload,
                setup_noop,
                lambda _cid, _i: [b"LPUSH", key, payload],
            )
        elif test == "rpush":
            key = b"bench:list:rpush"
            run_test(
                "RPUSH",
                host,
                port,
                clients,
                requests,
                payload,
                setup_noop,
                lambda _cid, _i: [b"RPUSH", key, payload],
            )
        elif test == "lpop":
            key = b"bench:list:lpop"
            def setup(reader: RespReader):
                setup_list(reader, key, requests + 10)
            run_test(
                "LPOP",
                host,
                port,
                clients,
                requests,
                payload,
                setup,
                lambda _cid, _i: [b"LPOP", key],
            )
        elif test == "rpop":
            key = b"bench:list:rpop"
            def setup(reader: RespReader):
                setup_list(reader, key, requests + 10)
            run_test(
                "RPOP",
                host,
                port,
                clients,
                requests,
                payload,
                setup,
                lambda _cid, _i: [b"RPOP", key],
            )
        elif test == "sadd":
            key = b"bench:set"
            run_test(
                "SADD",
                host,
                port,
                clients,
                requests,
                payload,
                lambda reader: setup_set(reader, key),
                lambda cid, i: [b"SADD", key, str(cid).encode() + b":" + str(i).encode()],
            )
        elif test == "hset":
            key = b"bench:hash"
            run_test(
                "HSET",
                host,
                port,
                clients,
                requests,
                payload,
                lambda reader: setup_hash(reader, key),
                lambda cid, i: [b"HSET", key, str(cid).encode() + b":" + str(i).encode(), payload],
            )
        elif test == "zadd":
            key = b"bench:zset"
            run_test(
                "ZADD",
                host,
                port,
                clients,
                requests,
                payload,
                lambda reader: setup_zset(reader, key),
                lambda cid, i: [b"ZADD", key, b"1", str(cid).encode() + b":" + str(i).encode()],
            )
        elif test == "xadd":
            key = b"bench:stream"
            run_test(
                "XADD",
                host,
                port,
                clients,
                requests,
                payload,
                lambda reader: setup_stream(reader, key),
                lambda _cid, _i: [b"XADD", key, b"*", b"f", payload],
            )
        elif test == "lrange":
            key = b"bench:list:lrange"
            def setup(reader: RespReader):
                setup_list(reader, key, max(args.lrange_size, 1))
            run_test(
                f"LRANGE_{args.lrange_size} (first {args.lrange_size} elements)",
                host,
                port,
                clients,
                requests,
                payload,
                setup,
                lambda _cid, _i: [b"LRANGE", key, b"0", str(args.lrange_size - 1).encode()],
            )
        else:
            print(f"Skipping unknown test: {test}")


if __name__ == "__main__":
    main()
