//! Per-group command handlers extracted from `handle_command`.
//!
//! Each function handles one logical group of Redis commands and returns
//! `Some(RespValue)` when the command is handled, or `None` to signal
//! "not my group — keep searching".
//!
//! Groups:
//! - [`string_cmds`]  – GET/SET(NX)/MSET/MGET/GETSET/APPEND/INCR*/DECR*/STRLEN/DBSIZE
//! - [`hash_cmds`]    – HSET/HGET/HDEL/HGETALL/HLEN/HEXISTS/HINCRBY/HSETNX
//! - [`list_cmds`]    – LPUSH/RPUSH/LPOP/RPOP/LRANGE/LLEN/LINDEX/LSET/LINSERT/LREM/LPUSHX/RPUSHX/LTRIM
//! - [`set_cmds`]     – SADD/SREM/SMEMBERS/SISMEMBER/SCARD/SMOVE/SUNION/SINTER
//! - [`zset_cmds`]    – ZADD/ZRANGE/ZREM/ZCARD
//! - [`stream_cmds`]  – XADD/XRANGE/XREVRANGE/XLEN/XDEL
//! - [`key_cmds`]     – SET/DEL/EXISTS/EXPIRE/PEXPIRE/PERSIST/TTL/PTTL/TYPE/KEYS/SCAN/FLUSHDB

use std::sync::Arc;
use crate::muon_cache::resp::RespValue;
use crate::muon_cache::store::Db;
use crate::muon_cache::persist::Persist;
use super::{parse_i64, parse_u64, parse_f64, parse_set_args, now_ms};

type PersistState = Option<Persist>;

/// Inline equivalent of the `log_cmd!` macro used in `handle_command`.
#[inline]
fn log_cmd(
    persist_state: &Arc<PersistState>,
    db_index: usize,
    cmd: &str,
    args: &[Arc<[u8]>],
    skip_aof: bool,
) {
    if !skip_aof {
        if let Some(p) = persist_state.as_ref() {
            if p.aof_enabled() {
                let _ = p.log_command_nowait(db_index, cmd.as_bytes(), args);
            }
        }
    }
}

// ── String / scalar commands ─────────────────────────────────────────────────

pub(super) fn string_cmds(
    db: &Db,
    cmd: &str,
    args: &[Arc<[u8]>],
    persist_state: &Arc<PersistState>,
    db_index: usize,
    skip_aof: bool,
) -> Option<RespValue> {
    let resp = match cmd {
        "DBSIZE" => RespValue::Integer(db.len() as i64),

        "GET" => match args.get(0) {
            Some(key) => match db.get_string(key.as_ref()) {
                Ok(Some(v)) => RespValue::Blob(v),
                Ok(None) => RespValue::Null,
                Err(_) => RespValue::StaticError(
                    "WRONGTYPE Operation against a key holding the wrong kind of value",
                ),
            },
            None => RespValue::StaticError("ERR wrong number of arguments for 'GET'"),
        },

        "SETNX" => match args.get(0).zip(args.get(1)) {
            Some((key, value)) => match db.set_nx(key.as_ref().to_vec(), Arc::clone(value)) {
                Ok(set) => {
                    if set {
                        log_cmd(persist_state, db_index, cmd, args, skip_aof);
                    }
                    RespValue::Integer(if set { 1 } else { 0 })
                }
                Err(_) => RespValue::StaticError(
                    "WRONGTYPE Operation against a key holding the wrong kind of value",
                ),
            },
            None => RespValue::StaticError("ERR wrong number of arguments for 'SETNX'"),
        },

        "MSET" => {
            if args.len() < 2 || args.len() % 2 != 0 {
                return Some(RespValue::StaticError(
                    "ERR wrong number of arguments for 'MSET'",
                ));
            }
            let mut idx = 0;
            while idx + 1 < args.len() {
                let key = args[idx].as_ref().to_vec();
                db.set_string(key, Arc::clone(&args[idx + 1]), None);
                idx += 2;
            }
            log_cmd(persist_state, db_index, cmd, args, skip_aof);
            RespValue::StaticSimple("OK")
        }

        "MGET" => {
            if args.is_empty() {
                return Some(RespValue::StaticError(
                    "ERR wrong number of arguments for 'MGET'",
                ));
            }
            let mut out = Vec::with_capacity(args.len());
            for key in args {
                match db.get_string(key.as_ref()) {
                    Ok(Some(v)) => out.push(RespValue::Blob(v)),
                    Ok(None) => out.push(RespValue::Null),
                    Err(_) => {
                        return Some(RespValue::StaticError(
                            "WRONGTYPE Operation against a key holding the wrong kind of value",
                        ))
                    }
                }
            }
            RespValue::Array(out)
        }

        "GETSET" => match args.get(0).zip(args.get(1)) {
            Some((key, value)) => {
                let prev = match db.get_string(key.as_ref()) {
                    Ok(val) => val,
                    Err(_) => {
                        return Some(RespValue::StaticError(
                            "WRONGTYPE Operation against a key holding the wrong kind of value",
                        ))
                    }
                };
                db.set_string(key.as_ref().to_vec(), Arc::clone(value), None);
                log_cmd(persist_state, db_index, cmd, args, skip_aof);
                match prev {
                    Some(v) => RespValue::Blob(v),
                    None => RespValue::Null,
                }
            }
            None => RespValue::StaticError("ERR wrong number of arguments for 'GETSET'"),
        },

        "APPEND" => match args.get(0).zip(args.get(1)) {
            Some((key, value)) => match db.append(key.as_ref().to_vec(), value.as_ref()) {
                Ok(len) => {
                    log_cmd(persist_state, db_index, cmd, args, skip_aof);
                    RespValue::Integer(len)
                }
                Err(_) => RespValue::StaticError(
                    "WRONGTYPE Operation against a key holding the wrong kind of value",
                ),
            },
            None => RespValue::StaticError("ERR wrong number of arguments for 'APPEND'"),
        },

        "INCR" => match args.get(0) {
            Some(key) => match db.incr_by(key.as_ref(), 1) {
                Ok(val) => {
                    log_cmd(persist_state, db_index, cmd, args, skip_aof);
                    RespValue::Integer(val)
                }
                Err(_) => {
                    RespValue::StaticError("ERR value is not an integer or out of range")
                }
            },
            None => RespValue::StaticError("ERR wrong number of arguments for 'INCR'"),
        },

        "INCRBY" => match args.get(0).zip(args.get(1)) {
            Some((key, delta)) => {
                let delta = parse_i64(delta.as_ref()).unwrap_or(0);
                match db.incr_by(key.as_ref(), delta) {
                    Ok(val) => {
                        log_cmd(persist_state, db_index, cmd, args, skip_aof);
                        RespValue::Integer(val)
                    }
                    Err(_) => {
                        RespValue::StaticError("ERR value is not an integer or out of range")
                    }
                }
            }
            None => RespValue::StaticError("ERR wrong number of arguments for 'INCRBY'"),
        },

        "DECR" => match args.get(0) {
            Some(key) => match db.incr_by(key.as_ref(), -1) {
                Ok(val) => {
                    log_cmd(persist_state, db_index, cmd, args, skip_aof);
                    RespValue::Integer(val)
                }
                Err(_) => {
                    RespValue::StaticError("ERR value is not an integer or out of range")
                }
            },
            None => RespValue::StaticError("ERR wrong number of arguments for 'DECR'"),
        },

        "DECRBY" => match args.get(0).zip(args.get(1)) {
            Some((key, delta)) => {
                let delta = parse_i64(delta.as_ref()).unwrap_or(0);
                match db.incr_by(key.as_ref(), -delta) {
                    Ok(val) => {
                        log_cmd(persist_state, db_index, cmd, args, skip_aof);
                        RespValue::Integer(val)
                    }
                    Err(_) => {
                        RespValue::StaticError("ERR value is not an integer or out of range")
                    }
                }
            }
            None => RespValue::StaticError("ERR wrong number of arguments for 'DECRBY'"),
        },

        "STRLEN" => match args.get(0) {
            Some(key) => match db.get_string(key.as_ref()) {
                Ok(Some(v)) => RespValue::Integer(v.len() as i64),
                Ok(None) => RespValue::Integer(0),
                Err(_) => RespValue::StaticError(
                    "WRONGTYPE Operation against a key holding the wrong kind of value",
                ),
            },
            None => RespValue::StaticError("ERR wrong number of arguments for 'STRLEN'"),
        },

        _ => return None,
    };
    Some(resp)
}

// ── Hash commands ────────────────────────────────────────────────────────────

pub(super) fn hash_cmds(
    db: &Db,
    cmd: &str,
    args: &[Arc<[u8]>],
    persist_state: &Arc<PersistState>,
    db_index: usize,
    skip_aof: bool,
) -> Option<RespValue> {
    let resp = match cmd {
        "HSET" => {
            if args.len() < 3 || args.len() % 2 == 0 {
                return Some(RespValue::StaticError(
                    "ERR wrong number of arguments for 'HSET'",
                ));
            }
            let key = &args[0];
            let mut added = 0;
            let mut idx = 1;
            while idx + 1 < args.len() {
                match db.hash_set_ref(key.as_ref(), &args[idx], &args[idx + 1]) {
                    Ok(is_new) => {
                        if is_new {
                            added += 1;
                        }
                    }
                    Err(_) => {
                        return Some(RespValue::StaticError(
                            "WRONGTYPE Operation against a key holding the wrong kind of value",
                        ))
                    }
                }
                idx += 2;
            }
            log_cmd(persist_state, db_index, cmd, args, skip_aof);
            RespValue::Integer(added)
        }

        "HGET" => match (args.get(0), args.get(1)) {
            (Some(key), Some(field)) => match db.hash_get(key.as_ref(), field.as_ref()) {
                Ok(Some(v)) => RespValue::Blob(v),
                Ok(None) => RespValue::Null,
                Err(_) => RespValue::StaticError(
                    "WRONGTYPE Operation against a key holding the wrong kind of value",
                ),
            },
            _ => RespValue::StaticError("ERR wrong number of arguments for 'HGET'"),
        },

        "HDEL" => {
            if args.len() < 2 {
                return Some(RespValue::StaticError(
                    "ERR wrong number of arguments for 'HDEL'",
                ));
            }
            let key = &args[0];
            match db.hash_del(key.as_ref(), &args[1..]) {
                Ok(removed) => {
                    log_cmd(persist_state, db_index, cmd, args, skip_aof);
                    RespValue::Integer(removed)
                }
                Err(_) => RespValue::StaticError(
                    "WRONGTYPE Operation against a key holding the wrong kind of value",
                ),
            }
        }

        "HGETALL" => match args.get(0) {
            Some(key) => match db.hash_getall(key.as_ref()) {
                Ok(items) => {
                    let mut out = Vec::with_capacity(items.len() * 2);
                    for (field, value) in items {
                        out.push(RespValue::Blob(field));
                        out.push(RespValue::Blob(value));
                    }
                    RespValue::Array(out)
                }
                Err(_) => RespValue::StaticError(
                    "WRONGTYPE Operation against a key holding the wrong kind of value",
                ),
            },
            None => RespValue::StaticError("ERR wrong number of arguments for 'HGETALL'"),
        },

        "HLEN" => match args.get(0) {
            Some(key) => match db.hash_len(key.as_ref()) {
                Ok(len) => RespValue::Integer(len),
                Err(_) => RespValue::StaticError(
                    "WRONGTYPE Operation against a key holding the wrong kind of value",
                ),
            },
            None => RespValue::StaticError("ERR wrong number of arguments for 'HLEN'"),
        },

        "HEXISTS" => match (args.get(0), args.get(1)) {
            (Some(key), Some(field)) => match db.hash_exists(key.as_ref(), field.as_ref()) {
                Ok(exists) => RespValue::Integer(if exists { 1 } else { 0 }),
                Err(_) => RespValue::StaticError(
                    "WRONGTYPE Operation against a key holding the wrong kind of value",
                ),
            },
            _ => RespValue::StaticError("ERR wrong number of arguments for 'HEXISTS'"),
        },

        "HINCRBY" => match (args.get(0), args.get(1), args.get(2)) {
            (Some(key), Some(field), Some(delta)) => match parse_i64(delta.as_ref()) {
                Some(delta) => match db.hash_incr_by(key.as_ref(), field.as_ref(), delta) {
                    Ok(val) => {
                        log_cmd(persist_state, db_index, cmd, args, skip_aof);
                        RespValue::Integer(val)
                    }
                    Err(_) => RespValue::StaticError("ERR hash value is not an integer"),
                },
                None => RespValue::StaticError("ERR value is not an integer or out of range"),
            },
            _ => RespValue::StaticError("ERR wrong number of arguments for 'HINCRBY'"),
        },

        "HSETNX" => match (args.get(0), args.get(1), args.get(2)) {
            (Some(key), Some(field), Some(value)) => {
                match db.hash_set_nx(
                    key.as_ref(),
                    field.as_ref().into(),
                    value.as_ref().into(),
                ) {
                    Ok(inserted) => {
                        if inserted {
                            log_cmd(persist_state, db_index, cmd, args, skip_aof);
                        }
                        RespValue::Integer(if inserted { 1 } else { 0 })
                    }
                    Err(_) => RespValue::StaticError(
                        "WRONGTYPE Operation against a key holding the wrong kind of value",
                    ),
                }
            }
            _ => RespValue::StaticError("ERR wrong number of arguments for 'HSETNX'"),
        },

        _ => return None,
    };
    Some(resp)
}

// ── List commands ────────────────────────────────────────────────────────────

pub(super) fn list_cmds(
    db: &Db,
    cmd: &str,
    args: &[Arc<[u8]>],
    persist_state: &Arc<PersistState>,
    db_index: usize,
    skip_aof: bool,
) -> Option<RespValue> {
    let resp = match cmd {
        "LPUSH" | "RPUSH" => {
            if args.len() < 2 {
                return Some(RespValue::Error(format!(
                    "ERR wrong number of arguments for '{}'",
                    cmd
                )));
            }
            let key = &args[0];
            match db.list_push(key.as_ref(), &args[1..], cmd == "LPUSH") {
                Ok(len) => {
                    log_cmd(persist_state, db_index, cmd, args, skip_aof);
                    RespValue::Integer(len)
                }
                Err(_) => RespValue::StaticError(
                    "WRONGTYPE Operation against a key holding the wrong kind of value",
                ),
            }
        }

        "LPOP" | "RPOP" => {
            if args.is_empty() || args.len() > 2 {
                return Some(RespValue::Error(format!(
                    "ERR wrong number of arguments for '{}'",
                    cmd
                )));
            }
            let key = &args[0];
            let count = if args.len() == 2 {
                let n = parse_i64(args[1].as_ref()).unwrap_or(0);
                if n <= 0 {
                    return Some(RespValue::StaticError(
                        "ERR value is not an integer or out of range",
                    ));
                }
                Some(n as usize)
            } else {
                None
            };
            if let Some(count) = count {
                let mut out = Vec::new();
                for _ in 0..count {
                    match db.list_pop(key.as_ref(), cmd == "LPOP") {
                        Ok(Some(v)) => out.push(RespValue::Blob(v)),
                        Ok(None) => break,
                        Err(_) => {
                            return Some(RespValue::Error(
                                "WRONGTYPE Operation against a key holding the wrong kind of value"
                                    .to_string(),
                            ))
                        }
                    }
                }
                if out.is_empty() {
                    RespValue::Null
                } else {
                    log_cmd(persist_state, db_index, cmd, args, skip_aof);
                    RespValue::Array(out)
                }
            } else {
                match db.list_pop(key.as_ref(), cmd == "LPOP") {
                    Ok(Some(v)) => {
                        log_cmd(persist_state, db_index, cmd, args, skip_aof);
                        RespValue::Blob(v)
                    }
                    Ok(None) => RespValue::Null,
                    Err(_) => RespValue::Error(
                        "WRONGTYPE Operation against a key holding the wrong kind of value"
                            .to_string(),
                    ),
                }
            }
        }

        "LRANGE" => match (args.get(0), args.get(1), args.get(2)) {
            (Some(key), Some(start), Some(stop)) => {
                let start = parse_i64(start.as_ref()).unwrap_or(0);
                let stop = parse_i64(stop.as_ref()).unwrap_or(-1);
                match db.list_range(key.as_ref(), start, stop) {
                    Ok(items) => {
                        RespValue::Array(items.into_iter().map(RespValue::Blob).collect())
                    }
                    Err(_) => RespValue::StaticError(
                        "WRONGTYPE Operation against a key holding the wrong kind of value",
                    ),
                }
            }
            _ => RespValue::StaticError("ERR wrong number of arguments for 'LRANGE'"),
        },

        "LLEN" => match args.get(0) {
            Some(key) => match db.list_len(key.as_ref()) {
                Ok(len) => RespValue::Integer(len),
                Err(_) => RespValue::StaticError(
                    "WRONGTYPE Operation against a key holding the wrong kind of value",
                ),
            },
            None => RespValue::StaticError("ERR wrong number of arguments for 'LLEN'"),
        },

        "LINDEX" => match (args.get(0), args.get(1)) {
            (Some(key), Some(index)) => {
                let idx = match parse_i64(index.as_ref()) {
                    Some(v) => v,
                    None => {
                        return Some(RespValue::StaticError(
                            "ERR value is not an integer or out of range",
                        ))
                    }
                };
                match db.list_index(key.as_ref(), idx) {
                    Ok(Some(v)) => RespValue::Blob(v),
                    Ok(None) => RespValue::Null,
                    Err(_) => RespValue::StaticError(
                        "WRONGTYPE Operation against a key holding the wrong kind of value",
                    ),
                }
            }
            _ => RespValue::StaticError("ERR wrong number of arguments for 'LINDEX'"),
        },

        "LSET" => match (args.get(0), args.get(1), args.get(2)) {
            (Some(key), Some(index), Some(value)) => {
                let idx = match parse_i64(index.as_ref()) {
                    Some(v) => v,
                    None => {
                        return Some(RespValue::StaticError(
                            "ERR value is not an integer or out of range",
                        ))
                    }
                };
                match db.list_set(key.as_ref(), idx, value.as_ref()) {
                    Ok(()) => {
                        log_cmd(persist_state, db_index, cmd, args, skip_aof);
                        RespValue::StaticSimple("OK")
                    }
                    Err(_) => RespValue::StaticError(
                        "WRONGTYPE Operation against a key holding the wrong kind of value",
                    ),
                }
            }
            _ => RespValue::StaticError("ERR wrong number of arguments for 'LSET'"),
        },

        "LINSERT" => match (args.get(0), args.get(1), args.get(2), args.get(3)) {
            (Some(key), Some(pos), Some(pivot), Some(value)) => {
                let before = match pos.as_ref().to_ascii_uppercase().as_slice() {
                    b"BEFORE" => true,
                    b"AFTER" => false,
                    _ => return Some(RespValue::StaticError("ERR syntax error")),
                };
                match db.list_insert(key.as_ref(), before, pivot.as_ref(), value.as_ref()) {
                    Ok(len) => {
                        if len > 0 {
                            log_cmd(persist_state, db_index, cmd, args, skip_aof);
                        }
                        RespValue::Integer(len)
                    }
                    Err(_) => RespValue::StaticError(
                        "WRONGTYPE Operation against a key holding the wrong kind of value",
                    ),
                }
            }
            _ => RespValue::StaticError("ERR wrong number of arguments for 'LINSERT'"),
        },

        "LREM" => match (args.get(0), args.get(1), args.get(2)) {
            (Some(key), Some(count), Some(value)) => {
                let cnt = match parse_i64(count.as_ref()) {
                    Some(v) => v,
                    None => {
                        return Some(RespValue::StaticError(
                            "ERR value is not an integer or out of range",
                        ))
                    }
                };
                match db.list_rem(key.as_ref(), cnt, value.as_ref()) {
                    Ok(removed) => {
                        if removed > 0 {
                            log_cmd(persist_state, db_index, cmd, args, skip_aof);
                        }
                        RespValue::Integer(removed)
                    }
                    Err(_) => RespValue::StaticError(
                        "WRONGTYPE Operation against a key holding the wrong kind of value",
                    ),
                }
            }
            _ => RespValue::StaticError("ERR wrong number of arguments for 'LREM'"),
        },

        "LPUSHX" | "RPUSHX" => {
            if args.len() < 2 {
                return Some(RespValue::Error(format!(
                    "ERR wrong number of arguments for '{}'",
                    cmd
                )));
            }
            let key = &args[0];
            let left = cmd == "LPUSHX";
            match db.list_push_x(key.as_ref(), &args[1..], left) {
                Ok(len) => {
                    if len > 0 {
                        log_cmd(persist_state, db_index, cmd, args, skip_aof);
                    }
                    RespValue::Integer(len)
                }
                Err(_) => RespValue::StaticError(
                    "WRONGTYPE Operation against a key holding the wrong kind of value",
                ),
            }
        }

        "LTRIM" => match (args.get(0), args.get(1), args.get(2)) {
            (Some(key), Some(start), Some(stop)) => {
                let start = parse_i64(start.as_ref()).unwrap_or(0);
                let stop = parse_i64(stop.as_ref()).unwrap_or(-1);
                match db.list_trim(key.as_ref(), start, stop) {
                    Ok(()) => {
                        log_cmd(persist_state, db_index, cmd, args, skip_aof);
                        RespValue::StaticSimple("OK")
                    }
                    Err(_) => RespValue::StaticError(
                        "WRONGTYPE Operation against a key holding the wrong kind of value",
                    ),
                }
            }
            _ => RespValue::StaticError("ERR wrong number of arguments for 'LTRIM'"),
        },

        _ => return None,
    };
    Some(resp)
}

// ── Set commands ─────────────────────────────────────────────────────────────

pub(super) fn set_cmds(
    db: &Db,
    cmd: &str,
    args: &[Arc<[u8]>],
    persist_state: &Arc<PersistState>,
    db_index: usize,
    skip_aof: bool,
) -> Option<RespValue> {
    let resp = match cmd {
        "SADD" => {
            if args.len() < 2 {
                return Some(RespValue::StaticError(
                    "ERR wrong number of arguments for 'SADD'",
                ));
            }
            let key = &args[0];
            match db.set_add(key.as_ref(), &args[1..]) {
                Ok(added) => {
                    log_cmd(persist_state, db_index, cmd, args, skip_aof);
                    RespValue::Integer(added)
                }
                Err(_) => RespValue::StaticError(
                    "WRONGTYPE Operation against a key holding the wrong kind of value",
                ),
            }
        }

        "SREM" => {
            if args.len() < 2 {
                return Some(RespValue::StaticError(
                    "ERR wrong number of arguments for 'SREM'",
                ));
            }
            let key = &args[0];
            match db.set_remove(key.as_ref(), &args[1..]) {
                Ok(removed) => {
                    log_cmd(persist_state, db_index, cmd, args, skip_aof);
                    RespValue::Integer(removed)
                }
                Err(_) => RespValue::StaticError(
                    "WRONGTYPE Operation against a key holding the wrong kind of value",
                ),
            }
        }

        "SMEMBERS" => match args.get(0) {
            Some(key) => match db.set_members(key.as_ref()) {
                Ok(members) => {
                    let mut out = Vec::with_capacity(members.len());
                    for member in members {
                        out.push(RespValue::Blob(member));
                    }
                    RespValue::Array(out)
                }
                Err(_) => RespValue::StaticError(
                    "WRONGTYPE Operation against a key holding the wrong kind of value",
                ),
            },
            None => RespValue::StaticError("ERR wrong number of arguments for 'SMEMBERS'"),
        },

        "SISMEMBER" => match (args.get(0), args.get(1)) {
            (Some(key), Some(member)) => match db.set_is_member(key.as_ref(), member.as_ref()) {
                Ok(exists) => RespValue::Integer(if exists { 1 } else { 0 }),
                Err(_) => RespValue::StaticError(
                    "WRONGTYPE Operation against a key holding the wrong kind of value",
                ),
            },
            _ => RespValue::StaticError("ERR wrong number of arguments for 'SISMEMBER'"),
        },

        "SCARD" => match args.get(0) {
            Some(key) => match db.set_card(key.as_ref()) {
                Ok(len) => RespValue::Integer(len),
                Err(_) => RespValue::StaticError(
                    "WRONGTYPE Operation against a key holding the wrong kind of value",
                ),
            },
            None => RespValue::StaticError("ERR wrong number of arguments for 'SCARD'"),
        },

        "SMOVE" => match (args.get(0), args.get(1), args.get(2)) {
            (Some(source), Some(dest), Some(member)) => {
                match db.set_move(source.as_ref(), dest.as_ref(), member.as_ref()) {
                    Ok(moved) => {
                        if moved {
                            log_cmd(persist_state, db_index, cmd, args, skip_aof);
                        }
                        RespValue::Integer(if moved { 1 } else { 0 })
                    }
                    Err(_) => RespValue::StaticError(
                        "WRONGTYPE Operation against a key holding the wrong kind of value",
                    ),
                }
            }
            _ => RespValue::StaticError("ERR wrong number of arguments for 'SMOVE'"),
        },

        "SUNION" => {
            if args.is_empty() {
                return Some(RespValue::StaticError(
                    "ERR wrong number of arguments for 'SUNION'",
                ));
            }
            let key_refs: Vec<&[u8]> = args.iter().map(|a| a.as_ref()).collect();
            match db.set_union(&key_refs) {
                Ok(members) => {
                    let out: Vec<RespValue> =
                        members.into_iter().map(|m| RespValue::Blob(m)).collect();
                    RespValue::Array(out)
                }
                Err(_) => RespValue::StaticError(
                    "WRONGTYPE Operation against a key holding the wrong kind of value",
                ),
            }
        }

        "SINTER" => {
            if args.is_empty() {
                return Some(RespValue::StaticError(
                    "ERR wrong number of arguments for 'SINTER'",
                ));
            }
            let key_refs: Vec<&[u8]> = args.iter().map(|a| a.as_ref()).collect();
            match db.set_inter(&key_refs) {
                Ok(members) => {
                    let out: Vec<RespValue> =
                        members.into_iter().map(|m| RespValue::Blob(m)).collect();
                    RespValue::Array(out)
                }
                Err(_) => RespValue::StaticError(
                    "WRONGTYPE Operation against a key holding the wrong kind of value",
                ),
            }
        }

        _ => return None,
    };
    Some(resp)
}

// ── Sorted-set commands ──────────────────────────────────────────────────────

pub(super) fn zset_cmds(
    db: &Db,
    cmd: &str,
    args: &[Arc<[u8]>],
    persist_state: &Arc<PersistState>,
    db_index: usize,
    skip_aof: bool,
) -> Option<RespValue> {
    let resp = match cmd {
        "ZADD" => {
            if args.len() < 3 || (args.len() - 1) % 2 != 0 {
                return Some(RespValue::StaticError(
                    "ERR wrong number of arguments for 'ZADD'",
                ));
            }
            let key = &args[0];
            let mut added = 0;
            let mut idx = 1;
            while idx + 1 < args.len() {
                let score = parse_f64(args[idx].as_ref()).unwrap_or(0.0);
                let member = args[idx + 1].as_ref().to_vec();
                match db.zadd(key.as_ref(), score, member) {
                    Ok(is_new) => {
                        if is_new {
                            added += 1;
                        }
                    }
                    Err(_) => {
                        return Some(RespValue::StaticError(
                            "WRONGTYPE Operation against a key holding the wrong kind of value",
                        ))
                    }
                }
                idx += 2;
            }
            log_cmd(persist_state, db_index, cmd, args, skip_aof);
            RespValue::Integer(added)
        }

        "ZRANGE" => match (args.get(0), args.get(1), args.get(2)) {
            (Some(key), Some(start), Some(stop)) => {
                let start = parse_i64(start.as_ref()).unwrap_or(0);
                let stop = parse_i64(stop.as_ref()).unwrap_or(-1);
                match db.zrange(key.as_ref(), start, stop) {
                    Ok(items) => RespValue::Array(
                        items
                            .into_iter()
                            .map(|v| RespValue::Blob(v.into()))
                            .collect(),
                    ),
                    Err(_) => RespValue::StaticError(
                        "WRONGTYPE Operation against a key holding the wrong kind of value",
                    ),
                }
            }
            _ => RespValue::StaticError("ERR wrong number of arguments for 'ZRANGE'"),
        },

        "ZREM" => {
            if args.len() < 2 {
                return Some(RespValue::StaticError(
                    "ERR wrong number of arguments for 'ZREM'",
                ));
            }
            let key = &args[0];
            let members: Vec<Vec<u8>> = args[1..].iter().map(|m| m.as_ref().to_vec()).collect();
            match db.zrem(key.as_ref(), &members) {
                Ok(removed) => {
                    if removed > 0 {
                        log_cmd(persist_state, db_index, cmd, args, skip_aof);
                    }
                    RespValue::Integer(removed)
                }
                Err(_) => RespValue::StaticError(
                    "WRONGTYPE Operation against a key holding the wrong kind of value",
                ),
            }
        }

        "ZCARD" => match args.get(0) {
            Some(key) => match db.zcard(key.as_ref()) {
                Ok(len) => RespValue::Integer(len),
                Err(_) => RespValue::StaticError(
                    "WRONGTYPE Operation against a key holding the wrong kind of value",
                ),
            },
            None => RespValue::StaticError("ERR wrong number of arguments for 'ZCARD'"),
        },

        _ => return None,
    };
    Some(resp)
}

// ── Stream commands ──────────────────────────────────────────────────────────

pub(super) fn stream_cmds(
    db: &Db,
    cmd: &str,
    args: &[Arc<[u8]>],
    persist_state: &Arc<PersistState>,
    db_index: usize,
    skip_aof: bool,
) -> Option<RespValue> {
    let resp = match cmd {
        "XADD" => {
            if args.len() < 4 || args.len() % 2 != 0 {
                return Some(RespValue::StaticError(
                    "ERR wrong number of arguments for 'XADD'",
                ));
            }
            let key = &args[0];
            let id = match std::str::from_utf8(args[1].as_ref()) {
                Ok(v) => v,
                Err(_) => return Some(RespValue::StaticError("ERR invalid stream ID")),
            };
            let mut fields = Vec::new();
            let mut idx = 2;
            while idx + 1 < args.len() {
                fields.push((
                    args[idx].as_ref().to_vec(),
                    args[idx + 1].as_ref().to_vec(),
                ));
                idx += 2;
            }
            match db.stream_add(key.as_ref(), id, fields) {
                Ok(new_id) => {
                    log_cmd(persist_state, db_index, cmd, args, skip_aof);
                    RespValue::Blob(new_id.into_bytes().into())
                }
                Err(_) => RespValue::StaticError("ERR invalid stream ID"),
            }
        }

        "XRANGE" => match (args.get(0), args.get(1), args.get(2)) {
            (Some(key), Some(start), Some(end)) => {
                let start = std::str::from_utf8(start.as_ref()).unwrap_or("-");
                let end = std::str::from_utf8(end.as_ref()).unwrap_or("+");
                match db.stream_range(key.as_ref(), start, end) {
                    Ok(items) => {
                        let mut out = Vec::with_capacity(items.len());
                        for (id, fields) in items {
                            let mut field_array = Vec::with_capacity(fields.len() * 2);
                            for (field, value) in fields {
                                field_array.push(RespValue::Blob(field.into()));
                                field_array.push(RespValue::Blob(value.into()));
                            }
                            out.push(RespValue::Array(vec![
                                RespValue::Blob(id.into_bytes().into()),
                                RespValue::Array(field_array),
                            ]));
                        }
                        RespValue::Array(out)
                    }
                    Err(_) => RespValue::StaticError(
                        "WRONGTYPE Operation against a key holding the wrong kind of value",
                    ),
                }
            }
            _ => RespValue::StaticError("ERR wrong number of arguments for 'XRANGE'"),
        },

        "XREVRANGE" => match (args.get(0), args.get(1), args.get(2)) {
            (Some(key), Some(end_arg), Some(start_arg)) => {
                let start_str = std::str::from_utf8(start_arg.as_ref()).unwrap_or("-");
                let end_str = std::str::from_utf8(end_arg.as_ref()).unwrap_or("+");
                match db.stream_rev_range(key.as_ref(), end_str, start_str) {
                    Ok(items) => {
                        let mut out = Vec::with_capacity(items.len());
                        for (id, fields) in items {
                            let mut field_array = Vec::with_capacity(fields.len() * 2);
                            for (field, value) in fields {
                                field_array.push(RespValue::Blob(field.into()));
                                field_array.push(RespValue::Blob(value.into()));
                            }
                            out.push(RespValue::Array(vec![
                                RespValue::Blob(id.into_bytes().into()),
                                RespValue::Array(field_array),
                            ]));
                        }
                        RespValue::Array(out)
                    }
                    Err(_) => RespValue::StaticError(
                        "WRONGTYPE Operation against a key holding the wrong kind of value",
                    ),
                }
            }
            _ => RespValue::StaticError("ERR wrong number of arguments for 'XREVRANGE'"),
        },

        "XLEN" => match args.get(0) {
            Some(key) => match db.stream_len(key.as_ref()) {
                Ok(len) => RespValue::Integer(len),
                Err(_) => RespValue::StaticError(
                    "WRONGTYPE Operation against a key holding the wrong kind of value",
                ),
            },
            None => RespValue::StaticError("ERR wrong number of arguments for 'XLEN'"),
        },

        "XDEL" => {
            if args.len() < 2 {
                return Some(RespValue::StaticError(
                    "ERR wrong number of arguments for 'XDEL'",
                ));
            }
            let key = &args[0];
            let ids: Vec<&str> = args[1..]
                .iter()
                .filter_map(|a| std::str::from_utf8(a.as_ref()).ok())
                .collect();
            match db.stream_del(key.as_ref(), &ids) {
                Ok(deleted) => {
                    log_cmd(persist_state, db_index, cmd, args, skip_aof);
                    RespValue::Integer(deleted)
                }
                Err(_) => RespValue::StaticError(
                    "WRONGTYPE Operation against a key holding the wrong kind of value",
                ),
            }
        }

        _ => return None,
    };
    Some(resp)
}

// ── Key / generic commands ───────────────────────────────────────────────────

pub(super) fn key_cmds(
    db: &Db,
    cmd: &str,
    args: &[Arc<[u8]>],
    persist_state: &Arc<PersistState>,
    db_index: usize,
    skip_aof: bool,
) -> Option<RespValue> {
    let resp = match cmd {
        "SET" => {
            // Hot path: plain SET key value (redis-benchmark default).
            if args.len() == 2 {
                db.set_string_ref(args[0].as_ref(), Arc::clone(&args[1]), None);
                log_cmd(persist_state, db_index, cmd, args, skip_aof);
                RespValue::StaticSimple("OK")
            } else {
                match parse_set_args(args) {
                    Ok((key, value, expire_ms)) => {
                        let expire_at = expire_ms.map(|ms| now_ms().saturating_add(ms));
                        db.set_string_ref(key.as_ref(), value, expire_at);
                        log_cmd(persist_state, db_index, cmd, args, skip_aof);
                        RespValue::StaticSimple("OK")
                    }
                    Err(e) => RespValue::Error(e),
                }
            }
        }

        "DEL" => {
            let mut removed = 0;
            for key in args {
                if db.remove(key.as_ref()) {
                    removed += 1;
                }
            }
            log_cmd(persist_state, db_index, cmd, args, skip_aof);
            RespValue::Integer(removed)
        }

        "EXISTS" => {
            let mut count = 0;
            for key in args {
                if db.exists(key.as_ref()) {
                    count += 1;
                }
            }
            RespValue::Integer(count)
        }

        "EXPIRE" => match (args.get(0), args.get(1)) {
            (Some(key), Some(sec)) => {
                let ms = parse_u64(sec.as_ref()).unwrap_or(0).saturating_mul(1000);
                let ok = db.set_expire_ms(key.as_ref(), ms);
                if ok {
                    log_cmd(persist_state, db_index, cmd, args, skip_aof);
                }
                RespValue::Integer(if ok { 1 } else { 0 })
            }
            _ => RespValue::StaticError("ERR wrong number of arguments for 'EXPIRE'"),
        },

        "PEXPIRE" => match (args.get(0), args.get(1)) {
            (Some(key), Some(ms)) => {
                let ms = parse_u64(ms.as_ref()).unwrap_or(0);
                let ok = db.set_expire_ms(key.as_ref(), ms);
                if ok {
                    log_cmd(persist_state, db_index, cmd, args, skip_aof);
                }
                RespValue::Integer(if ok { 1 } else { 0 })
            }
            _ => RespValue::StaticError("ERR wrong number of arguments for 'PEXPIRE'"),
        },

        "PERSIST" => match args.get(0) {
            Some(key) => {
                let removed = db.persist(key.as_ref());
                if removed == 1 {
                    log_cmd(persist_state, db_index, cmd, args, skip_aof);
                }
                RespValue::Integer(removed)
            }
            None => RespValue::StaticError("ERR wrong number of arguments for 'PERSIST'"),
        },

        "TTL" => match args.get(0) {
            Some(key) => match db.ttl_ms(key.as_ref()) {
                Some(ms) if ms >= 0 => RespValue::Integer((ms / 1000) as i64),
                Some(ms) => RespValue::Integer(ms),
                None => RespValue::Integer(-2),
            },
            None => RespValue::StaticError("ERR wrong number of arguments for 'TTL'"),
        },

        "PTTL" => match args.get(0) {
            Some(key) => match db.ttl_ms(key.as_ref()) {
                Some(ms) => RespValue::Integer(ms),
                None => RespValue::Integer(-2),
            },
            None => RespValue::StaticError("ERR wrong number of arguments for 'PTTL'"),
        },

        "TYPE" => match args.get(0) {
            Some(key) => match db.value_type(key.as_ref()) {
                Some(t) => RespValue::Simple(t.to_string()),
                None => RespValue::Simple("none".to_string()),
            },
            None => RespValue::StaticError("ERR wrong number of arguments for 'TYPE'"),
        },

        "KEYS" => match args.get(0) {
            Some(pattern) => {
                let keys = db.keys_matching(pattern.as_ref());
                RespValue::Array(keys.into_iter().map(|v| RespValue::Blob(v.into())).collect())
            }
            None => RespValue::StaticError("ERR wrong number of arguments for 'KEYS'"),
        },

        "SCAN" => match args.get(0) {
            Some(cursor) => {
                let cursor_str = match core::str::from_utf8(cursor.as_ref()) {
                    Ok(s) => s,
                    Err(_) => return Some(RespValue::StaticError("ERR invalid cursor")),
                };
                let mut cursor_val = match cursor_str.parse::<usize>() {
                    Ok(v) => v,
                    Err(_) => return Some(RespValue::StaticError("ERR invalid cursor")),
                };
                let mut pattern = b"*".to_vec();
                let mut count = 10usize;
                let mut i = 1usize;
                while i < args.len() {
                    let opt = args[i].as_ref().to_ascii_uppercase();
                    if opt == b"MATCH" {
                        if i + 1 >= args.len() {
                            return Some(RespValue::StaticError("ERR syntax error"));
                        }
                        pattern = args[i + 1].as_ref().to_vec();
                        i += 2;
                        continue;
                    }
                    if opt == b"COUNT" {
                        if i + 1 >= args.len() {
                            return Some(RespValue::StaticError("ERR syntax error"));
                        }
                        let cnt_str = match core::str::from_utf8(args[i + 1].as_ref()) {
                            Ok(s) => s,
                            Err(_) => return Some(RespValue::StaticError("ERR invalid COUNT")),
                        };
                        count = match cnt_str.parse::<usize>() {
                            Ok(v) => v,
                            Err(_) => {
                                return Some(RespValue::StaticError("ERR invalid COUNT"))
                            }
                        };
                        i += 2;
                        continue;
                    }
                    return Some(RespValue::StaticError("ERR syntax error"));
                }
                let keys = db.keys_matching(&pattern);
                if cursor_val > keys.len() {
                    cursor_val = keys.len();
                }
                let end = (cursor_val + count).min(keys.len());
                let batch = keys[cursor_val..end].to_vec();
                let next_cursor = if end >= keys.len() { 0 } else { end };
                let mut out = Vec::with_capacity(2);
                out.push(RespValue::Blob(
                    next_cursor.to_string().into_bytes().into(),
                ));
                out.push(RespValue::Array(
                    batch.into_iter().map(|v| RespValue::Blob(v.into())).collect(),
                ));
                RespValue::Array(out)
            }
            None => RespValue::StaticError("ERR wrong number of arguments for 'SCAN'"),
        },

        "FLUSHDB" => {
            if !args.is_empty() {
                return Some(RespValue::StaticError(
                    "ERR wrong number of arguments for 'FLUSHDB'",
                ));
            }
            db.flush();
            log_cmd(persist_state, db_index, cmd, args, skip_aof);
            RespValue::StaticSimple("OK")
        }

        _ => return None,
    };
    Some(resp)
}
