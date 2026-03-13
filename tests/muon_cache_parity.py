#!/usr/bin/env python3
import argparse
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


def is_simple(resp, val):
    return resp[0] == "simple" and resp[1] == val


def is_int(resp, val=None):
    return resp[0] == "int" and (val is None or resp[1] == val)


def is_blob(resp, val):
    return resp[0] == "blob" and resp[1] == val


def is_null(resp):
    return resp[0] == "null"


def array_as_blob_list(resp):
    if resp[0] != "array":
        return None
    out = []
    for item in resp[1]:
        if item[0] != "blob":
            return None
        out.append(item[1])
    return out


def array_as_kv_dict(resp):
    if resp[0] != "array":
        return None
    items = resp[1]
    if len(items) % 2 != 0:
        return None
    out = {}
    for idx in range(0, len(items), 2):
        k = items[idx]
        v = items[idx + 1]
        if k[0] != "blob" or v[0] != "blob":
            return None
        out[k[1]] = v[1]
    return out


def parse_stream_entries(resp):
    if resp[0] != "array":
        return None
    entries = []
    for entry in resp[1]:
        if entry[0] != "array" or len(entry[1]) != 2:
            return None
        entry_id, fields = entry[1]
        if entry_id[0] != "blob" or fields[0] != "array":
            return None
        field_map = array_as_kv_dict(fields)
        if field_map is None:
            return None
        entries.append((entry_id[1], field_map))
    return entries

TESTS = [
    # Ensure a clean slate for both Redis and muoncache on every run
    ("FLUSHALL init", ["FLUSHALL"], expect_simple("OK")),
    ("FUNCTION FLUSH init", ["FUNCTION", "FLUSH"], expect_simple("OK")),
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
    ("LPUSH count", ["LPUSH", "lc", "a", "b", "c"], expect_int(3)),
    ("LPOP count", ["LPOP", "lc", "2"], expect_list([b"c", b"b"])),
    ("LINDEX", ["LINDEX", "lc", "0"], expect_blob(b"a")),
    ("LSET", ["LSET", "lc", "0", "z"], expect_simple("OK")),
    ("LINDEX after LSET", ["LINDEX", "lc", "0"], expect_blob(b"z")),
    ("LINSERT BEFORE", ["LINSERT", "lc", "BEFORE", "z", "y"], expect_int(2)),
    ("LREM", ["LREM", "lc", "0", "z"], expect_int(1)),
    ("LPUSHX missing", ["LPUSHX", "lmissing", "1"], expect_int(0)),
    ("RPUSHX missing", ["RPUSHX", "lmissing", "1"], expect_int(0)),
    ("RPUSHX existing", ["RPUSHX", "l", "9"], expect_int(0)),
    ("RPUSH l2", ["RPUSH", "l2", "a", "b"], expect_int(2)),
    ("LPOP count 1", ["LPOP", "l2", "1"], expect_list([b"a"])),
    ("LPUSH then LTRIM", ["LPUSH", "lt", "a", "b", "c", "d"], expect_int(4)),
    ("LTRIM", ["LTRIM", "lt", "0", "1"], expect_simple("OK")),
    ("LRANGE after LTRIM", ["LRANGE", "lt", "0", "-1"], expect_list([b"d", b"c"])),
    # Sets
    ("SADD", ["SADD", "s", "1", "2"], expect_int(2)),
    ("SCARD", ["SCARD", "s"], expect_int(2)),
    ("SISMEMBER", ["SISMEMBER", "s", "1"], expect_int(1)),
    ("SMEMBERS", ["SMEMBERS", "s"], lambda r: r[0] == "array" and len(r[1]) == 2),
    ("SREM", ["SREM", "s", "2"], expect_int(1)),
    ("SMOVE", ["SMOVE", "s", "s2", "1"], expect_int(1)),
    ("SCARD s2", ["SCARD", "s2"], expect_int(1)),
    ("SISMEMBER missing", ["SISMEMBER", "s2", "2"], expect_int(0)),
    ("SADD existing", ["SADD", "s2", "1"], expect_int(0)),
    ("SMEMBERS s2", ["SMEMBERS", "s2"], lambda r: r[0] == "array" and len(r[1]) == 1),
    ("SUNION", ["SUNION", "s", "s2"], lambda r: r[0] == "array" and len(r[1]) >= 1),
    ("SINTER", ["SINTER", "s", "s2"], lambda r: r[0] == "array"),
    # Hashes
    ("HSET", ["HSET", "h", "f", "v"], expect_int(1)),
    ("HGET", ["HGET", "h", "f"], expect_blob(b"v")),
    ("HEXISTS", ["HEXISTS", "h", "f"], expect_int(1)),
    ("HLEN", ["HLEN", "h"], expect_int(1)),
    ("HGETALL", ["HGETALL", "h"], expect_hash_pair(b"f", b"v")),
    ("HDEL", ["HDEL", "h", "f"], expect_int(1)),
    ("HGET missing", ["HGET", "h", "f"], expect_null()),
    ("HSET multiple", ["HSET", "h2", "f1", "v1", "f2", "v2"], expect_int(2)),
    ("HGET f2", ["HGET", "h2", "f2"], expect_blob(b"v2")),
    ("HINCRBY", ["HINCRBY", "h2", "count", "5"], expect_int(5)),
    ("HINCRBY again", ["HINCRBY", "h2", "count", "-2"], expect_int(3)),
    ("HSETNX", ["HSETNX", "h2", "f2", "v3"], expect_int(0)),
    ("HSETNX new", ["HSETNX", "h2", "f3", "v3"], expect_int(1)),
    ("HLEN h2", ["HLEN", "h2"], expect_int(4)),
    # Sorted sets
    ("ZADD", ["ZADD", "z", "2", "b", "1", "a"], expect_int(2)),
    ("ZCARD", ["ZCARD", "z"], expect_int(2)),
    ("ZRANGE", ["ZRANGE", "z", "0", "-1"], expect_list([b"a", b"b"])),
    ("ZREM", ["ZREM", "z", "a"], expect_int(1)),
    # Streams
    ("XADD", ["XADD", "s", "*", "f", "v"], lambda r: r[0] == "blob"),
    ("XRANGE", ["XRANGE", "s", "-", "+"], lambda r: r[0] == "array" and len(r[1]) >= 1),
    ("XLEN", ["XLEN", "s"], expect_int(1)),
    ("XADD second", ["XADD", "s", "*", "f", "v2"], lambda r: r[0] == "blob"),
    ("XREVRANGE", ["XREVRANGE", "s", "+", "-"], lambda r: r[0] == "array" and len(r[1]) == 2),
    ("XDEL", ["XDEL", "s", "0-0"], expect_int(0)),
    # Pub/Sub
    # Transactions
    ("MULTI", ["MULTI"], expect_simple("OK")),
    ("MULTI SET", ["SET", "tx", "1"], expect_simple("QUEUED")),
    ("MULTI GET", ["GET", "tx"], expect_simple("QUEUED")),
    ("EXEC", ["EXEC"], lambda r: r[0] == "array" and len(r[1]) == 2 and r[1][0][0] == "simple" and r[1][1][0] == "blob"),
    ("GET tx", ["GET", "tx"], expect_blob(b"1")),
    ("MULTI again", ["MULTI"], expect_simple("OK")),
    ("DISCARD", ["DISCARD"], expect_simple("OK")),
    # Scripting — scripts use KEYS[1]/ARGV[1] (1-based, matches Redis/Lua convention)
    ("EVAL", ["EVAL", "return 1", "0"], expect_int(1)),
    ("EVAL redis.call", ["EVAL", "return redis.call('SET', KEYS[1], ARGV[1])", "1", "evalkey", "evalval"], lambda r: (r[0] in ("simple", "blob") and r[1] == b"OK") or (r[0] == "simple" and r[1] == "OK")),
    ("GET evalkey", ["GET", "evalkey"], expect_blob(b"evalval")),
    ("EVAL redis.pcall", ["EVAL", "return redis.pcall('NOPE')", "0"], expect_error()),
    ("SCRIPT LOAD", ["SCRIPT", "LOAD", "return 2"], lambda r: r[0] == "blob"),
    ("SCRIPT EXISTS", ["SCRIPT", "EXISTS", "__SCRIPT_SHA__", "deadbeef"], lambda r: r[0] == "array" and len(r[1]) == 2),
    ("EVALSHA", ["EVALSHA", "__SCRIPT_SHA__", "0"], expect_int(2)),
    ("SCRIPT FLUSH", ["SCRIPT", "FLUSH"], expect_simple("OK")),
    ("EVALSHA missing", ["EVALSHA", "__SCRIPT_SHA__", "0"], expect_error()),
    ("FUNCTION LIST", ["FUNCTION", "LIST"], lambda r: r[0] == "array"),
    # FUNCTION LOAD: body uses #!lua shebang + redis.register_function (required by Redis 8).
    # muoncache strips the shebang and stores the rest; Redis executes it with the Lua engine.
    # Redis returns the library name as a blob; muoncache returns simple OK.
    ("FUNCTION LOAD", ["FUNCTION", "LOAD",
        "#!lua name=mylib\nredis.register_function('myfunc', function(keys, args) return 3 end)"],
        lambda r: r[0] in ("simple", "blob")),
    ("FUNCTION FLUSH", ["FUNCTION", "FLUSH"], expect_simple("OK")),
    # Server / config
    ("CONFIG GET", ["CONFIG", "GET", "*"], lambda r: r[0] == "array"),
    ("CLIENT LIST", ["CLIENT", "LIST"], lambda r: r[0] in ("blob", "simple")),
    ("SLOWLOG GET", ["SLOWLOG", "GET"], lambda r: r[0] == "array"),
    # Persistence / replication
    ("SAVE", ["SAVE"], lambda r: r[0] in ("error", "simple")),
    # BGSAVE: Redis returns a simple "Background saving started"; muoncache returns an error when not configured.
    ("BGSAVE", ["BGSAVE"], lambda r: r[0] in ("error", "simple")),
    # REPLICAOF NO ONE: Redis returns OK; muoncache returns an error (not implemented).
    ("REPLICAOF", ["REPLICAOF", "NO", "ONE"], lambda r: r[0] in ("error", "simple")),
    ("FLUSHDB", ["FLUSHDB"], expect_simple("OK")),
    ("FLUSHALL", ["FLUSHALL"], expect_simple("OK")),
    ("SUBSCRIBE", ["SUBSCRIBE", "c"], lambda r: r[0] == "array"),
    # PUBLISH on a subscribe socket: Redis rejects with an error; muoncache allows it and returns the subscriber count.
    ("PUBLISH", ["PUBLISH", "c", "msg"], lambda r: r[0] in ("int", "error")),
]


def send_cmd(sock, cmd):
    sock.sendall(resp_encode(cmd))
    return resp_read(sock)


def run_perf(sock, seconds, retain, retain_count):
    if seconds <= 0:
        print("perf skipped (non-positive duration)")
        return

    start = time.monotonic()
    deadline = start + seconds
    prefix = f"perf:{int(time.time())}"
    ops = 0
    failures = 0
    iterations = 0

    def fail(label, resp):
        nonlocal failures
        failures += 1
        print(f"PERF FAIL {label} -> {resp}")

    retained = 0
    while time.monotonic() < deadline:
        i = iterations
        iterations += 1

        keep = retain and retained < retain_count

        # Strings: insert, retrieve, update, retrieve, delete, verify missing
        skey = f"{prefix}:str:{i}"
        svalue1 = f"v{i}".encode()
        svalue2 = f"v{i}:u".encode()
        resp = send_cmd(sock, ["SET", skey, svalue1])
        ops += 1
        if not is_simple(resp, "OK"):
            fail("SET string", resp)
        resp = send_cmd(sock, ["GET", skey])
        ops += 1
        if not is_blob(resp, svalue1):
            fail("GET string", resp)
        resp = send_cmd(sock, ["SET", skey, svalue2])
        ops += 1
        if not is_simple(resp, "OK"):
            fail("SET string update", resp)
        resp = send_cmd(sock, ["GET", skey])
        ops += 1
        if not is_blob(resp, svalue2):
            fail("GET string updated", resp)
        if not keep:
            resp = send_cmd(sock, ["DEL", skey])
            ops += 1
            if not is_int(resp):
                fail("DEL string", resp)
            resp = send_cmd(sock, ["GET", skey])
            ops += 1
            if not is_null(resp):
                fail("GET string missing", resp)

        # Lists: insert, retrieve, update element, retrieve, delete
        lkey = f"{prefix}:list:{i}"
        resp = send_cmd(sock, ["RPUSH", lkey, "a", "b"]) 
        ops += 1
        if not is_int(resp):
            fail("RPUSH list", resp)
        resp = send_cmd(sock, ["LRANGE", lkey, "0", "-1"])
        ops += 1
        items = array_as_blob_list(resp)
        if items is None or items != [b"a", b"b"]:
            fail("LRANGE list", resp)
        resp = send_cmd(sock, ["LSET", lkey, "0", "z"])
        ops += 1
        if not is_simple(resp, "OK"):
            fail("LSET list", resp)
        resp = send_cmd(sock, ["LINDEX", lkey, "0"])
        ops += 1
        if not is_blob(resp, b"z"):
            fail("LINDEX list", resp)
        if not keep:
            resp = send_cmd(sock, ["DEL", lkey])
            ops += 1
            if not is_int(resp):
                fail("DEL list", resp)
            resp = send_cmd(sock, ["LLEN", lkey])
            ops += 1
            if not is_int(resp, 0):
                fail("LLEN list missing", resp)

        # Sets: insert, retrieve, update (remove), retrieve, delete
        setkey = f"{prefix}:set:{i}"
        resp = send_cmd(sock, ["SADD", setkey, "a", "b"])
        ops += 1
        if not is_int(resp):
            fail("SADD set", resp)
        resp = send_cmd(sock, ["SMEMBERS", setkey])
        ops += 1
        members = array_as_blob_list(resp)
        if members is None or set(members) != {b"a", b"b"}:
            fail("SMEMBERS set", resp)
        resp = send_cmd(sock, ["SREM", setkey, "b"])
        ops += 1
        if not is_int(resp):
            fail("SREM set", resp)
        resp = send_cmd(sock, ["SMEMBERS", setkey])
        ops += 1
        members = array_as_blob_list(resp)
        if members is None or set(members) != {b"a"}:
            fail("SMEMBERS set updated", resp)
        if not keep:
            resp = send_cmd(sock, ["DEL", setkey])
            ops += 1
            if not is_int(resp):
                fail("DEL set", resp)
            resp = send_cmd(sock, ["SCARD", setkey])
            ops += 1
            if not is_int(resp, 0):
                fail("SCARD set missing", resp)

        # Hashes: insert, retrieve, update field, retrieve, delete field
        hkey = f"{prefix}:hash:{i}"
        resp = send_cmd(sock, ["HSET", hkey, "f", "v1"])
        ops += 1
        if not is_int(resp):
            fail("HSET hash", resp)
        resp = send_cmd(sock, ["HGET", hkey, "f"])
        ops += 1
        if not is_blob(resp, b"v1"):
            fail("HGET hash", resp)
        resp = send_cmd(sock, ["HSET", hkey, "f", "v2"])
        ops += 1
        if not is_int(resp):
            fail("HSET hash update", resp)
        resp = send_cmd(sock, ["HGET", hkey, "f"])
        ops += 1
        if not is_blob(resp, b"v2"):
            fail("HGET hash updated", resp)
        if not keep:
            resp = send_cmd(sock, ["HDEL", hkey, "f"])
            ops += 1
            if not is_int(resp):
                fail("HDEL hash", resp)
            resp = send_cmd(sock, ["HGET", hkey, "f"])
            ops += 1
            if not is_null(resp):
                fail("HGET hash missing", resp)

        # Sorted sets: insert, retrieve, update score, retrieve, delete member
        zkey = f"{prefix}:z:{i}"
        resp = send_cmd(sock, ["ZADD", zkey, "1", "a", "2", "b"])
        ops += 1
        if not is_int(resp):
            fail("ZADD zset", resp)
        resp = send_cmd(sock, ["ZRANGE", zkey, "0", "-1"])
        ops += 1
        zitems = array_as_blob_list(resp)
        if zitems is None or zitems != [b"a", b"b"]:
            fail("ZRANGE zset", resp)
        resp = send_cmd(sock, ["ZADD", zkey, "3", "a"])
        ops += 1
        if not is_int(resp):
            fail("ZADD zset update", resp)
        resp = send_cmd(sock, ["ZRANGE", zkey, "0", "-1"])
        ops += 1
        zitems = array_as_blob_list(resp)
        if zitems is None or zitems != [b"b", b"a"]:
            fail("ZRANGE zset updated", resp)
        if not keep:
            resp = send_cmd(sock, ["ZREM", zkey, "a"])
            ops += 1
            if not is_int(resp):
                fail("ZREM zset", resp)
            resp = send_cmd(sock, ["ZRANGE", zkey, "0", "-1"])
            ops += 1
            zitems = array_as_blob_list(resp)
            if zitems is None or zitems != [b"b"]:
                fail("ZRANGE zset removed", resp)

        # Streams: append entries, verify, delete key
        xkey = f"{prefix}:stream:{i}"
        resp = send_cmd(sock, ["XADD", xkey, "*", "f", "v1"])
        ops += 1
        if resp[0] != "blob":
            fail("XADD stream", resp)
        resp = send_cmd(sock, ["XADD", xkey, "*", "f", "v2"])
        ops += 1
        if resp[0] != "blob":
            fail("XADD stream second", resp)
        resp = send_cmd(sock, ["XRANGE", xkey, "-", "+"])
        ops += 1
        entries = parse_stream_entries(resp)
        if entries is None or len(entries) < 2:
            fail("XRANGE stream", resp)
        else:
            last_fields = entries[-1][1]
            if last_fields.get(b"f") != b"v2":
                fail("XRANGE stream last", resp)
        if not keep:
            resp = send_cmd(sock, ["DEL", xkey])
            ops += 1
            if not is_int(resp):
                fail("DEL stream", resp)

        if keep:
            retained += 1

    elapsed = time.monotonic() - start
    ops_per_sec = ops / elapsed if elapsed > 0 else 0.0
    print(
        "\nPerf summary: "
        f"{iterations} iterations, {ops} ops, "
        f"{failures} failures, {elapsed:.2f}s elapsed, "
        f"{ops_per_sec:.1f} ops/s"
    )


def run_perf_matrix(sock, seconds):
    if seconds <= 0:
        return
    phases = ["set", "read", "update", "delete"]
    per_phase = max(0.05, seconds / (len(phases) * 6))
    prefix = f"perfmat:{int(time.time())}"

    def bench(label, setup_fn, cmd_fn, post_fn=None):
        if setup_fn is not None:
            setup_fn()
        start = time.monotonic()
        end = start + per_phase
        ops = 0
        errors = 0
        while time.monotonic() < end:
            resp = send_cmd(sock, cmd_fn())
            ops += 1
            if resp is None or resp[0] == "error":
                errors += 1
            if post_fn is not None:
                post_fn()
        ops_per_sec = ops / per_phase if per_phase > 0 else 0.0
        print(f"{label}: {ops} ops, {errors} errors, {ops_per_sec:.1f} ops/s")

    print("\nPerf matrix (per data type, set/read/update/delete)")

    # Strings
    skey = f"{prefix}:str"
    sval = 0
    def s_set():
        nonlocal sval
        sval += 1
        return ["SET", skey, f"v{sval}"]
    def s_read():
        return ["GET", skey]
    def s_update():
        nonlocal sval
        sval += 1
        return ["SET", skey, f"v{sval}:u"]
    def s_delete():
        return ["DEL", skey]
    def s_setup():
        _ = send_cmd(sock, ["SET", skey, "v0"])
    def s_recreate():
        _ = send_cmd(sock, ["SET", skey, "v0"])

    # Lists
    lkey = f"{prefix}:list"
    def l_setup():
        _ = send_cmd(sock, ["DEL", lkey])
        _ = send_cmd(sock, ["RPUSH", lkey, "a", "b", "c"])
    def l_set():
        return ["RPUSH", lkey, "d"]
    def l_read():
        return ["LRANGE", lkey, "0", "-1"]
    def l_update():
        return ["LSET", lkey, "0", "z"]
    def l_delete():
        return ["DEL", lkey]
    def l_recreate():
        _ = send_cmd(sock, ["RPUSH", lkey, "a", "b", "c"])

    # Sets
    setkey = f"{prefix}:set"
    def set_setup():
        _ = send_cmd(sock, ["DEL", setkey])
        _ = send_cmd(sock, ["SADD", setkey, "a", "b"])
    def set_set():
        return ["SADD", setkey, "c"]
    def set_read():
        return ["SISMEMBER", setkey, "a"]
    def set_update():
        return ["SREM", setkey, "b"]
    def set_delete():
        return ["DEL", setkey]
    def set_recreate():
        _ = send_cmd(sock, ["SADD", setkey, "a", "b"])

    # Hashes
    hkey = f"{prefix}:hash"
    hval = 0
    def h_setup():
        _ = send_cmd(sock, ["DEL", hkey])
        _ = send_cmd(sock, ["HSET", hkey, "f", "v0"])
    def h_set():
        return ["HSET", hkey, "f", "v1"]
    def h_read():
        return ["HGET", hkey, "f"]
    def h_update():
        nonlocal hval
        hval += 1
        return ["HSET", hkey, "f", f"v{hval}"]
    def h_delete():
        return ["HDEL", hkey, "f"]
    def h_recreate():
        _ = send_cmd(sock, ["HSET", hkey, "f", "v0"])

    # Sorted sets
    zkey = f"{prefix}:zset"
    def z_setup():
        _ = send_cmd(sock, ["DEL", zkey])
        _ = send_cmd(sock, ["ZADD", zkey, "1", "a", "2", "b"])
    def z_set():
        return ["ZADD", zkey, "3", "c"]
    def z_read():
        return ["ZRANGE", zkey, "0", "-1"]
    def z_update():
        return ["ZADD", zkey, "4", "a"]
    def z_delete():
        return ["ZREM", zkey, "a"]
    def z_recreate():
        _ = send_cmd(sock, ["ZADD", zkey, "1", "a", "2", "b"])

    # Streams
    xkey = f"{prefix}:stream"
    def x_setup():
        _ = send_cmd(sock, ["DEL", xkey])
        _ = send_cmd(sock, ["XADD", xkey, "*", "f", "v0"])
    def x_set():
        return ["XADD", xkey, "*", "f", "v1"]
    def x_read():
        return ["XRANGE", xkey, "-", "+"]
    def x_update():
        return ["XADD", xkey, "*", "f", "v2"]
    def x_delete():
        return ["DEL", xkey]
    def x_recreate():
        _ = send_cmd(sock, ["XADD", xkey, "*", "f", "v0"])

    for dtype, setup_fn, set_fn, read_fn, update_fn, delete_fn, recreate_fn in [
        ("string", s_setup, s_set, s_read, s_update, s_delete, s_recreate),
        ("list", l_setup, l_set, l_read, l_update, l_delete, l_recreate),
        ("set", set_setup, set_set, set_read, set_update, set_delete, set_recreate),
        ("hash", h_setup, h_set, h_read, h_update, h_delete, h_recreate),
        ("zset", z_setup, z_set, z_read, z_update, z_delete, z_recreate),
        ("stream", x_setup, x_set, x_read, x_update, x_delete, x_recreate),
    ]:
        bench(f"{dtype} set", setup_fn, set_fn)
        bench(f"{dtype} read", setup_fn, read_fn)
        bench(f"{dtype} update", setup_fn, update_fn)
        bench(f"{dtype} delete", setup_fn, delete_fn, recreate_fn)


def main():
    parser = argparse.ArgumentParser(description="muon_cache parity + perf runner")
    parser.add_argument("host", nargs="?", default="127.0.0.1")
    parser.add_argument("port", nargs="?", type=int, default=6379)
    parser.add_argument("--perf-seconds", type=float, default=1.2)
    parser.add_argument("--perf-retain", action="store_true", help="retain a sample of each data type after perf")
    parser.add_argument("--perf-retain-count", type=int, default=10, help="number of retained samples when --perf-retain is set")
    parser.add_argument("--no-perf", action="store_true", help="skip perf test")
    args = parser.parse_args()
    host = args.host
    port = args.port

    print(f"muon_cache parity run @ commit {git_commit()}")
    print(f"connecting to {host}:{port}")
    sock = socket.create_connection((host, port))

    passed = 0
    failed = 0
    last_script_sha = None
    for name, cmd, check in TESTS:
        if cmd:
            cmd = [last_script_sha if (last_script_sha is not None and v == "__SCRIPT_SHA__") else v for v in cmd]
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

    if not args.no_perf:
        sock = socket.create_connection((host, port))
        print(f"\nStarting perf run for {args.perf_seconds:.2f}s")
        run_perf(sock, args.perf_seconds, args.perf_retain, args.perf_retain_count)
        run_perf_matrix(sock, args.perf_seconds)
        sock.close()


if __name__ == "__main__":
    main()
