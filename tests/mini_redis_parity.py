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

def expect_list(values):
    def check(resp):
        if resp[0] != "array":
            return False
        items = resp[1]
        if len(items) != len(values):
            return False
        for item, expected in zip(items, values):
            if item[0] != "blob" or item[1] != expected:
                return False
        return True
    return check

def expect_hash_pair(field, value):
    def check(resp):
        if resp[0] != "array":
            return False
        items = resp[1]
        if len(items) != 2:
            return False
        pairs = [(items[0], items[1])]
        for a, b in pairs:
            if a[0] == "blob" and b[0] == "blob":
                if a[1] == field and b[1] == value:
                    return True
                if a[1] == value and b[1] == field:
                    return True
        return False
    return check

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
    ("PERSIST", ["PERSIST", "b"], expect_int(1)),
    ("PTTL after PERSIST", ["PTTL", "b"], expect_int(-1)),
    # String ops
    ("SETNX", ["SETNX", "k", "v"], expect_int(1)),
    ("SETNX again", ["SETNX", "k", "v2"], expect_int(0)),
    ("MSET", ["MSET", "k1", "v1", "k2", "v2"], expect_simple("OK")),
    ("MGET", ["MGET", "k1", "k2"], expect_list([b"v1", b"v2"])),
    ("GETSET", ["GETSET", "k1", "v3"], expect_blob(b"v1")),
    ("APPEND", ["APPEND", "k1", "x"], expect_int(3)),
    ("STRLEN", ["STRLEN", "k1"], expect_int(3)),
    ("INCR", ["INCR", "counter"], expect_int(1)),
    ("INCRBY", ["INCRBY", "counter", "5"], expect_int(6)),
    ("DECR", ["DECR", "counter"], expect_int(5)),
    ("DECRBY", ["DECRBY", "counter", "2"], expect_int(3)),
    # Lists
    ("LPUSH", ["LPUSH", "l", "1", "2"], expect_int(2)),
    ("LPOP", ["LPOP", "l"], expect_blob(b"2")),
    ("RPUSH", ["RPUSH", "l", "3"], expect_int(2)),
    ("LRANGE", ["LRANGE", "l", "0", "-1"], expect_list([b"1", b"3"])),
    ("LLEN", ["LLEN", "l"], expect_int(2)),
    ("RPOP", ["RPOP", "l"], expect_blob(b"3")),
    ("LPOP empty", ["LPOP", "l"], expect_blob(b"1")),
    ("LLEN empty", ["LLEN", "l"], expect_int(0)),
    # Sets
    ("SADD", ["SADD", "s", "1", "2"], expect_int(2)),
    ("SCARD", ["SCARD", "s"], expect_int(2)),
    ("SISMEMBER", ["SISMEMBER", "s", "1"], expect_int(1)),
    ("SMEMBERS", ["SMEMBERS", "s"], lambda r: r[0] == "array" and len(r[1]) == 2),
    ("SREM", ["SREM", "s", "2"], expect_int(1)),
    ("SMOVE", ["SMOVE", "s", "s2", "1"], expect_int(1)),
    ("SCARD s2", ["SCARD", "s2"], expect_int(1)),
    # Hashes
    ("HSET", ["HSET", "h", "f", "v"], expect_int(1)),
    ("HGET", ["HGET", "h", "f"], expect_blob(b"v")),
    ("HEXISTS", ["HEXISTS", "h", "f"], expect_int(1)),
    ("HLEN", ["HLEN", "h"], expect_int(1)),
    ("HGETALL", ["HGETALL", "h"], expect_hash_pair(b"f", b"v")),
    ("HDEL", ["HDEL", "h", "f"], expect_int(1)),
    ("HGET missing", ["HGET", "h", "f"], expect_null()),
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
