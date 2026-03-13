#!/usr/bin/env python3
import sqlite3
import sys
from typing import Optional

TABLES = [
    "kv",
    "list_items",
    "set_items",
    "hash_items",
    "zset_items",
    "stream_entries",
    "aof_log",
]


def bytes_or_none(val: Optional[bytes]) -> str:
    if val is None:
        return "<null>"
    try:
        return val.decode("utf-8")
    except Exception:
        return val.hex()


def main() -> int:
    if len(sys.argv) < 2:
        print("Usage: scripts/read_muon_cache_db.py <path-to-db>")
        return 2
    path = sys.argv[1]
    conn = sqlite3.connect(path)
    cur = conn.cursor()

    print("\n== persisted db counts ==")
    for table in TABLES:
        cur.execute(
            "SELECT name FROM sqlite_master WHERE type='table' AND name=?",
            (table,),
        )
        if cur.fetchone() is None:
            continue
        cur.execute(f"SELECT COUNT(*) FROM {table}")
        count = cur.fetchone()[0]
        print(f"{table}: {count}")

    # Detailed dump for debugging (kept for future use)
    # for table in TABLES:
    #     cur.execute(
    #         "SELECT name FROM sqlite_master WHERE type='table' AND name=?",
    #         (table,),
    #     )
    #     if cur.fetchone() is None:
    #         continue
    #
    #     print(f"\n== {table} ==")
    #     if table == "kv":
    #         cur.execute("SELECT db, key, type, value, expires_at_ms FROM kv ORDER BY db, key")
    #         for db, key, typ, value, exp in cur.fetchall():
    #             print(
    #                 f"db={db} key={bytes_or_none(key)} type={typ} value={bytes_or_none(value)} expires_at_ms={exp}"
    #             )
    #     elif table == "list_items":
    #         cur.execute("SELECT db, key, idx, value FROM list_items ORDER BY db, key, idx")
    #         for db, key, idx, value in cur.fetchall():
    #             print(f"db={db} key={bytes_or_none(key)} idx={idx} value={bytes_or_none(value)}")
    #     elif table == "set_items":
    #         cur.execute("SELECT db, key, value FROM set_items ORDER BY db, key, value")
    #         for db, key, value in cur.fetchall():
    #             print(f"db={db} key={bytes_or_none(key)} value={bytes_or_none(value)}")
    #     elif table == "hash_items":
    #         cur.execute("SELECT db, key, field, value FROM hash_items ORDER BY db, key, field")
    #         for db, key, field, value in cur.fetchall():
    #             print(
    #                 f"db={db} key={bytes_or_none(key)} field={bytes_or_none(field)} value={bytes_or_none(value)}"
    #             )
    #     elif table == "zset_items":
    #         cur.execute("SELECT db, key, member, score FROM zset_items ORDER BY db, key, score")
    #         for db, key, member, score in cur.fetchall():
    #             print(
    #                 f"db={db} key={bytes_or_none(key)} member={bytes_or_none(member)} score={score}"
    #             )
    #     elif table == "stream_entries":
    #         cur.execute(
    #             "SELECT db, key, entry_id, field_idx, field, value FROM stream_entries ORDER BY db, key, entry_id, field_idx"
    #         )
    #         for db, key, entry_id, field_idx, field, value in cur.fetchall():
    #             print(
    #                 "db={} key={} entry_id={} field_idx={} field={} value={}".format(
    #                     db,
    #                     bytes_or_none(key),
    #                     bytes_or_none(entry_id),
    #                     field_idx,
    #                     bytes_or_none(field),
    #                     bytes_or_none(value),
    #                 )
    #             )
    #     elif table == "aof_log":
    #         cur.execute("SELECT id, db, cmd FROM aof_log ORDER BY id")
    #         for row_id, db, cmd in cur.fetchall():
    #             print(f"id={row_id} db={db} cmd={bytes_or_none(cmd)}")

    conn.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
