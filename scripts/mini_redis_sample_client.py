#!/usr/bin/env python3
import argparse
import socket
import time


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


def format_resp(resp):
    if resp is None:
        return "<no response>"
    kind, val = resp
    if kind == "blob":
        try:
            return f"{kind}:{val.decode()}"
        except Exception:
            return f"{kind}:{val!r}"
    return f"{kind}:{val}"


def main():
    parser = argparse.ArgumentParser(description="mini-redis sample client with JS scripting and perf timing")
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=6379)
    parser.add_argument("--perf", type=int, default=1000, help="Number of SET/GET pairs for perf timing")
    args = parser.parse_args()

    sock = socket.create_connection((args.host, args.port))

    print("PING ->", format_resp(send_cmd(sock, "PING")))

    print("SET user:1:name Alice ->", format_resp(send_cmd(sock, "SET", "user:1:name", "Alice")))
    print("GET user:1:name ->", format_resp(send_cmd(sock, "GET", "user:1:name")))

    print("HSET user:1 age 30 ->", format_resp(send_cmd(sock, "HSET", "user:1", "age", "30")))
    print("HGET user:1 age ->", format_resp(send_cmd(sock, "HGET", "user:1", "age")))

    script = """
redis.call('SET', KEYS[0], ARGV[0]);
return redis.call('GET', KEYS[0]);
""".strip()
    print("EVAL (JS) ->", format_resp(send_cmd(sock, "EVAL", script, 1, "js:key", "js:value")))

    print("GET js:key ->", format_resp(send_cmd(sock, "GET", "js:key")))

    script2 = """
return redis.pcall('NOPE');
""".strip()
    print("EVAL (pcall error) ->", format_resp(send_cmd(sock, "EVAL", script2, 0)))

    start = time.perf_counter()
    for i in range(args.perf):
        send_cmd(sock, "SET", f"perf:{i}", "v")
        send_cmd(sock, "GET", f"perf:{i}")
    elapsed = time.perf_counter() - start
    ops = args.perf * 2
    print(f"client perf: {ops} ops in {elapsed:.3f}s -> {ops / elapsed:.1f} ops/s")

    sock.close()


if __name__ == "__main__":
    main()
