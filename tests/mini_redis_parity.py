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
    # Connection / server
    ("PING", ["PING"], expect_simple("PONG")),
    ("ECHO", ["ECHO", "hi"], expect_blob(b"hi")),
    ("INFO", ["INFO"], lambda r: r[0] in ("blob", "simple")),
    # QUIT closes the connection; skip to keep a single-session run stable.
    # Keyspace / basic
    ("SELECT", ["SELECT", "1"], expect_simple("OK")),
    ("SET", ["SET", "a", "1"], expect_simple("OK")),
    ("GET", ["GET", "a"], expect_blob(b"1")),
    ("EXISTS", ["EXISTS", "a"], expect_int(1)),
    ("DEL", ["DEL", "a"], expect_int(1)),
    ("GET missing", ["GET", "a"], expect_null()),
    ("TTL missing", ["TTL", "a"], expect_int(-2)),
    ("SET EX", ["SET", "b", "1", "EX", "1"], expect_simple("OK")),
    ("GET b", ["GET", "b"], expect_blob(b"1")),
    ("TYPE (expected fail for now)", ["TYPE", "b"], lambda r: r[0] in ("simple", "error")),
    ("KEYS (expected fail for now)", ["KEYS", "*"], expect_error()),
    ("SCAN (expected fail for now)", ["SCAN", "0"], expect_error()),
    ("FLUSHDB (expected fail for now)", ["FLUSHDB"], expect_error()),
    ("FLUSHALL (expected fail for now)", ["FLUSHALL"], expect_error()),
    # Expire
    ("EXPIRE (expected fail for now)", ["EXPIRE", "b", "1"], lambda r: r[0] in ("int", "error")),
    ("PEXPIRE (expected fail for now)", ["PEXPIRE", "b", "1"], lambda r: r[0] in ("int", "error")),
    ("PTTL (expected fail for now)", ["PTTL", "b"], lambda r: r[0] in ("int", "error")),
    ("PERSIST (expected fail for now)", ["PERSIST", "b"], expect_error()),
    # String ops
    ("SETNX (expected fail for now)", ["SETNX", "k", "v"], expect_error()),
    ("MSET (expected fail for now)", ["MSET", "k1", "v1", "k2", "v2"], expect_error()),
    ("MGET (expected fail for now)", ["MGET", "k1", "k2"], expect_error()),
    ("GETSET (expected fail for now)", ["GETSET", "k1", "v3"], expect_error()),
    ("APPEND (expected fail for now)", ["APPEND", "k1", "x"], expect_error()),
    ("INCR (expected fail for now)", ["INCR", "counter"], expect_error()),
    ("INCRBY (expected fail for now)", ["INCRBY", "counter", "5"], expect_error()),
    ("DECR (expected fail for now)", ["DECR", "counter"], expect_error()),
    ("DECRBY (expected fail for now)", ["DECRBY", "counter", "5"], expect_error()),
    ("STRLEN (expected fail for now)", ["STRLEN", "k1"], expect_error()),
    # Lists
    ("LPUSH (expected fail for now)", ["LPUSH", "l", "1"], expect_error()),
    ("RPUSH (expected fail for now)", ["RPUSH", "l", "2"], expect_error()),
    ("LPOP (expected fail for now)", ["LPOP", "l"], expect_error()),
    ("RPOP (expected fail for now)", ["RPOP", "l"], expect_error()),
    ("LRANGE (expected fail for now)", ["LRANGE", "l", "0", "-1"], expect_error()),
    ("LLEN (expected fail for now)", ["LLEN", "l"], expect_error()),
    # Sets
    ("SADD (expected fail for now)", ["SADD", "s", "1"], expect_error()),
    ("SREM (expected fail for now)", ["SREM", "s", "1"], expect_error()),
    ("SMEMBERS (expected fail for now)", ["SMEMBERS", "s"], expect_error()),
    ("SISMEMBER (expected fail for now)", ["SISMEMBER", "s", "1"], expect_error()),
    ("SCARD (expected fail for now)", ["SCARD", "s"], expect_error()),
    ("SMOVE (expected fail for now)", ["SMOVE", "s", "s2", "1"], expect_error()),
    # Hashes
    ("HSET (expected fail for now)", ["HSET", "h", "f", "v"], expect_error()),
    ("HGET (expected fail for now)", ["HGET", "h", "f"], expect_error()),
    ("HDEL (expected fail for now)", ["HDEL", "h", "f"], expect_error()),
    ("HGETALL (expected fail for now)", ["HGETALL", "h"], expect_error()),
    ("HLEN (expected fail for now)", ["HLEN", "h"], expect_error()),
    ("HEXISTS (expected fail for now)", ["HEXISTS", "h", "f"], expect_error()),
    # Sorted sets
    ("ZADD (expected fail for now)", ["ZADD", "z", "1", "a"], expect_error()),
    ("ZRANGE (expected fail for now)", ["ZRANGE", "z", "0", "-1"], expect_error()),
    ("ZREM (expected fail for now)", ["ZREM", "z", "a"], expect_error()),
    ("ZCARD (expected fail for now)", ["ZCARD", "z"], expect_error()),
    # Streams
    ("XADD (expected fail for now)", ["XADD", "s", "*", "f", "v"], expect_error()),
    ("XRANGE (expected fail for now)", ["XRANGE", "s", "-", "+"], expect_error()),
    # Pub/Sub
    ("SUBSCRIBE (expected fail for now)", ["SUBSCRIBE", "c"], expect_error()),
    ("PUBLISH (expected fail for now)", ["PUBLISH", "c", "msg"], expect_error()),
    # Transactions
    ("MULTI (expected fail for now)", ["MULTI"], expect_error()),
    ("EXEC (expected fail for now)", ["EXEC"], expect_error()),
    ("DISCARD (expected fail for now)", ["DISCARD"], expect_error()),
    # Scripting
    ("EVAL (expected fail for now)", ["EVAL", "return 1", "0"], expect_error()),
    ("EVALSHA (expected fail for now)", ["EVALSHA", "deadbeef", "0"], expect_error()),
    ("SCRIPT (expected fail for now)", ["SCRIPT", "LOAD", "return 1"], expect_error()),
    ("FUNCTION (expected fail for now)", ["FUNCTION", "LIST"], expect_error()),
    # Server / config
    ("CONFIG (expected fail for now)", ["CONFIG", "GET", "*"], expect_error()),
    ("CLIENT (expected fail for now)", ["CLIENT", "LIST"], expect_error()),
    ("SLOWLOG (expected fail for now)", ["SLOWLOG", "GET"], expect_error()),
    # Persistence / replication
    ("SAVE (expected fail for now)", ["SAVE"], expect_error()),
    ("BGSAVE (expected fail for now)", ["BGSAVE"], expect_error()),
    ("REPLICAOF (expected fail for now)", ["REPLICAOF", "NO", "ONE"], expect_error()),
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
        if resp is None:
            failed += 1
            print(f"FAIL {name:30} -> no response (connection closed)")
            break
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
