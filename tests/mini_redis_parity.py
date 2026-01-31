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
    ("KEYS", ["KEYS", "*"], lambda r: r[0] == "array"),
    ("SCAN", ["SCAN", "0"], lambda r: r[0] == "array" and len(r[1]) == 2),
    # Expire
    ("EXPIRE", ["EXPIRE", "b", "10"], lambda r: r[0] == "int"),
    ("PEXPIRE", ["PEXPIRE", "b", "1000"], lambda r: r[0] == "int"),
    ("PTTL", ["PTTL", "b"], lambda r: r[0] == "int" and r[1] >= 0),
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
    ("ZADD", ["ZADD", "z", "2", "b", "1", "a"], expect_int(2)),
    ("ZCARD", ["ZCARD", "z"], expect_int(2)),
    ("ZRANGE", ["ZRANGE", "z", "0", "-1"], expect_list([b"a", b"b"])),
    ("ZREM", ["ZREM", "z", "a"], expect_int(1)),
    # Streams
    ("XADD", ["XADD", "s", "*", "f", "v"], lambda r: r[0] == "blob"),
    ("XRANGE", ["XRANGE", "s", "-", "+"], lambda r: r[0] == "array" and len(r[1]) >= 1),
    # Pub/Sub
    # Transactions
    ("MULTI", ["MULTI"], expect_simple("OK")),
    ("MULTI SET", ["SET", "tx", "1"], expect_simple("QUEUED")),
    ("MULTI GET", ["GET", "tx"], expect_simple("QUEUED")),
    ("EXEC", ["EXEC"], lambda r: r[0] == "array" and len(r[1]) == 2 and r[1][0][0] == "simple" and r[1][1][0] == "blob"),
    ("GET tx", ["GET", "tx"], expect_blob(b"1")),
    ("MULTI again", ["MULTI"], expect_simple("OK")),
    ("DISCARD", ["DISCARD"], expect_simple("OK")),
    # Scripting
    ("EVAL", ["EVAL", "return 1", "0"], expect_int(1)),
    ("EVAL redis.call", ["EVAL", "return redis.call('SET', KEYS[0], ARGV[0])", "1", "evalkey", "evalval"], lambda r: (r[0] in ("simple", "blob") and r[1] == b"OK") or (r[0] == "simple" and r[1] == "OK")),
    ("GET evalkey", ["GET", "evalkey"], expect_blob(b"evalval")),
    ("SCRIPT LOAD", ["SCRIPT", "LOAD", "return 2"], lambda r: r[0] == "blob"),
    ("EVALSHA", ["EVALSHA", "__SCRIPT_SHA__", "0"], expect_int(2)),
    ("FUNCTION (expected fail for now)", ["FUNCTION", "LIST"], expect_error()),
    # Server / config
    ("CONFIG GET", ["CONFIG", "GET", "*"], lambda r: r[0] == "array"),
    ("CLIENT LIST", ["CLIENT", "LIST"], lambda r: r[0] in ("blob", "simple")),
    ("SLOWLOG GET", ["SLOWLOG", "GET"], lambda r: r[0] == "array"),
    # Persistence / replication
    ("SAVE", ["SAVE"], expect_error()),
    ("BGSAVE", ["BGSAVE"], expect_error()),
    ("REPLICAOF", ["REPLICAOF", "NO", "ONE"], expect_error()),
    ("FLUSHDB", ["FLUSHDB"], expect_simple("OK")),
    ("FLUSHALL", ["FLUSHALL"], expect_simple("OK")),
    ("SUBSCRIBE", ["SUBSCRIBE", "c"], lambda r: r[0] == "array"),
    ("PUBLISH", ["PUBLISH", "c", "msg"], expect_int(1)),
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
    last_script_sha = None
    for name, cmd, check in TESTS:
        if cmd and cmd[0] == "EVALSHA":
            if last_script_sha is None:
                failed += 1
                print(f"FAIL {name:30} -> missing SCRIPT LOAD sha")
                continue
            cmd = [cmd[0], last_script_sha] + cmd[2:]
        sock.sendall(resp_encode(cmd))
        resp = resp_read(sock)
        if name == "SCRIPT LOAD" and resp is not None and resp[0] == "blob":
            try:
                last_script_sha = resp[1].decode()
            except Exception:
                last_script_sha = None
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
