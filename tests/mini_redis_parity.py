#!/usr/bin/env python3
import socket
import sys
import time
import subprocess


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
        _ = sock.recv(2)  # CRLF
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


def git_commit():
    try:
        return subprocess.check_output(["git", "rev-parse", "--short", "HEAD"]).decode().strip()
    except Exception:
        return "unknown"


def expect_simple(val):
    return lambda resp: resp[0] == "simple" and resp[1] == val


def expect_int(val):
    return lambda resp: resp[0] == "int" and resp[1] == val


def expect_blob(val):
    return lambda resp: resp[0] == "blob" and resp[1] == val


def expect_null():
    return lambda resp: resp[0] == "null"


def expect_error():
    return lambda resp: resp[0] == "error"


TESTS = [
    ("PING", ["PING"], expect_simple("PONG")),
    ("ECHO", ["ECHO", "hi"], expect_blob(b"hi")),
    ("SELECT", ["SELECT", "1"], expect_simple("OK")),
    ("SET", ["SET", "a", "1"], expect_simple("OK")),
    ("GET", ["GET", "a"], expect_blob(b"1")),
    ("EXISTS", ["EXISTS", "a"], expect_int(1)),
    ("DEL", ["DEL", "a"], expect_int(1)),
    ("GET missing", ["GET", "a"], expect_null()),
    ("TTL missing", ["TTL", "a"], expect_int(-2)),
    ("SET EX", ["SET", "b", "1", "EX", "1"], expect_simple("OK")),
    ("GET b", ["GET", "b"], expect_blob(b"1")),
    ("SADD (expected fail for now)", ["SADD", "s", "1"], expect_error()),
    ("HSET (expected fail for now)", ["HSET", "h", "f", "v"], expect_error()),
    ("LPUSH (expected fail for now)", ["LPUSH", "l", "1"], expect_error()),
    ("ZADD (expected fail for now)", ["ZADD", "z", "1", "a"], expect_error()),
    ("EVAL (expected fail for now)", ["EVAL", "return 1", "0"], expect_error()),
]


def main():
    host = "127.0.0.1"
    port = 6379
    if len(sys.argv) >= 2:
        host = sys.argv[1]
    if len(sys.argv) >= 3:
        port = int(sys.argv[2])

    print(f"mini_redis parity run @ commit {git_commit()}")
    print(f"connecting to {host}:{port}")
    sock = socket.create_connection((host, port))

    passed = 0
    failed = 0
    for name, cmd, check in TESTS:
        sock.sendall(resp_encode(cmd))
        resp = resp_read(sock)
        ok = check(resp)
        if ok:
            passed += 1
            status = "PASS"
        else:
            failed += 1
            status = "FAIL"
        print(f"{status:4} {name:30} -> {resp}")

    print(f"\nSummary: {passed} passed, {failed} failed, total {passed+failed}")
    sock.close()


if __name__ == "__main__":
    main()
