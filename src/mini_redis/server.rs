//! RESP3 server and command dispatcher.

use async_std::io::{self, BufReader};
use async_std::net::{TcpListener, TcpStream};
use async_std::sync::{Arc, Mutex};
use async_std::task;
use async_std::channel::Sender;
use std::ffi::c_void;

use crate::mini_redis::resp::{read_value, write_value, RespValue};
use crate::mini_redis::persist::Persist;
use crate::mini_redis::store::Db;
use crate::{
    JSContextImpl, JS_EVAL_RETVAL, JS_GetException, JS_GetGlobalObject, JS_IsBool, JS_IsNull,
    JS_IsNumber, JS_IsString, JS_IsUndefined, JS_NewArray, JS_NewCFunctionParams, JS_NewInt64,
    JS_NewObject, JS_NewString, JS_NewStringLen, JS_RegisterStdlibMinimal, JS_SetCFunctionTable,
    JS_SetContextOpaque, JS_SetPropertyStr, JS_SetPropertyUint32, JS_ToCString, JS_ToNumber,
};
use crate::{JSCFunctionDef, JSCFunctionDefEnum, JSCFunctionType, JSCStringBuf, JSValue};
use crate::{JS_Eval, JS_ThrowInternalError};

const SCRIPT_MEM_SIZE: usize = 256 * 1024;

#[derive(Clone)]
pub struct ServerConfig {
    pub bind: String,
    pub port: u16,
    pub databases: usize,
    pub persist_path: Option<String>,
    pub aof_enabled: bool,
}

pub struct ServerState {
    dbs: Vec<Db>,
    persist: Option<Persist>,
    pubsub: std::collections::HashMap<Vec<u8>, Vec<Sender<RespValue>>>,
    script_cache: std::collections::HashMap<String, String>,
}

impl ServerState {
    fn new(dbs: Vec<Db>, persist: Option<Persist>) -> Self {
        Self {
            dbs,
            persist,
            pubsub: std::collections::HashMap::new(),
            script_cache: std::collections::HashMap::new(),
        }
    }
}

pub async fn run(config: ServerConfig) -> io::Result<()> {
    let addr = format!("{}:{}", config.bind, config.port);
    let listener = TcpListener::bind(&addr).await?;
    let persist = init_persist(&config).await?;
    let mut dbs = Vec::with_capacity(config.databases);
    for _ in 0..config.databases {
        dbs.push(Db::new());
    }
    if let Some(p) = persist.as_ref() {
        let _ = p.load(&mut dbs).await;
    }
    let state = Arc::new(Mutex::new(ServerState::new(dbs, persist)));
    loop {
        let (stream, _) = listener.accept().await?;
        let state = state.clone();
        task::spawn(async move {
            let _ = handle_client(stream, state).await;
        });
    }
}

async fn handle_client(stream: TcpStream, state: Arc<Mutex<ServerState>>) -> io::Result<()> {
    let peer = stream.peer_addr().ok();
    let mut reader = BufReader::new(stream.clone());
    let mut current_db: usize = 0;
    let mut in_multi = false;
    let mut queued: Vec<(String, Vec<Vec<u8>>)> = Vec::new();
    let (pub_tx, pub_rx) = async_std::channel::unbounded::<RespValue>();
    let mut script_runtime: Option<ScriptRuntime> = None;
    loop {
        let val = read_value(&mut reader).await?;
        let val = match val {
            Some(v) => v,
            None => return Ok(()),
        };
        let args = match value_to_args(val) {
            Ok(a) => a,
            Err(err) => {
                write_value(&mut &stream, &RespValue::Error(err)).await?;
                continue;
            }
        };
        if args.is_empty() {
            write_value(&mut &stream, &RespValue::Error("ERR empty command".to_string())).await?;
            continue;
        }
        let cmd = to_upper_ascii(&args[0]);
        let resp = if cmd == "SUBSCRIBE" {
            handle_subscribe(&state, &pub_tx, &args[1..]).await
        } else if cmd == "PUBLISH" {
            handle_publish(&state, &args[1..]).await
        } else if in_multi && cmd != "EXEC" && cmd != "DISCARD" && cmd != "MULTI" {
            queued.push((cmd.clone(), args[1..].to_vec()));
            RespValue::Simple("QUEUED".to_string())
        } else if cmd == "MULTI" {
            if in_multi {
                RespValue::Error("ERR MULTI calls can not be nested".to_string())
            } else {
                in_multi = true;
                queued.clear();
                RespValue::Simple("OK".to_string())
            }
        } else if cmd == "DISCARD" {
            if !in_multi {
                RespValue::Error("ERR DISCARD without MULTI".to_string())
            } else {
                in_multi = false;
                queued.clear();
                RespValue::Simple("OK".to_string())
            }
        } else if cmd == "EXEC" {
            if !in_multi {
                RespValue::Error("ERR EXEC without MULTI".to_string())
            } else {
                in_multi = false;
                let mut results = Vec::with_capacity(queued.len());
                let mut guard = state.lock().await;
                for (qcmd, qargs) in queued.drain(..) {
                    let resp = handle_command(&mut guard, &mut current_db, &mut script_runtime, &qcmd, &qargs).await;
                    results.push(resp);
                }
                RespValue::Array(results)
            }
        } else {
            let mut guard = state.lock().await;
            handle_command(&mut guard, &mut current_db, &mut script_runtime, &cmd, &args[1..]).await
        };
        write_value(&mut &stream, &resp).await?;
        while let Ok(msg) = pub_rx.try_recv() {
            let _ = write_value(&mut &stream, &msg).await;
        }
        if cmd == "QUIT" {
            let _ = peer;
            return Ok(());
        }
    }
}

async fn handle_subscribe(
    state: &Arc<Mutex<ServerState>>,
    sender: &Sender<RespValue>,
    channels: &[Vec<u8>],
) -> RespValue {
    if channels.is_empty() {
        return RespValue::Error("ERR wrong number of arguments for 'SUBSCRIBE'".to_string());
    }
    let mut guard = state.lock().await;
    let mut count = 0;
    for channel in channels {
        let entry = guard.pubsub.entry(channel.clone()).or_default();
        entry.push(sender.clone());
        count += 1;
    }
    let channel = channels[0].clone();
    RespValue::Array(vec![
        RespValue::Blob(b"subscribe".to_vec()),
        RespValue::Blob(channel),
        RespValue::Integer(count),
    ])
}

async fn handle_publish(state: &Arc<Mutex<ServerState>>, args: &[Vec<u8>]) -> RespValue {
    if args.len() != 2 {
        return RespValue::Error("ERR wrong number of arguments for 'PUBLISH'".to_string());
    }
    let channel = args[0].clone();
    let message = args[1].clone();
    let mut receivers = Vec::new();
    {
        let guard = state.lock().await;
        if let Some(list) = guard.pubsub.get(&channel) {
            receivers.extend(list.iter().cloned());
        }
    }
    let mut delivered = 0;
    for tx in receivers {
        if tx
            .send(RespValue::Array(vec![
                RespValue::Blob(b"message".to_vec()),
                RespValue::Blob(channel.clone()),
                RespValue::Blob(message.clone()),
            ]))
            .await
            .is_ok()
        {
            delivered += 1;
        }
    }
    RespValue::Integer(delivered)
}

async fn handle_command(
    state: &mut ServerState,
    db_index: &mut usize,
    script: &mut Option<ScriptRuntime>,
    cmd: &str,
    args: &[Vec<u8>],
) -> RespValue {
    match cmd {
        "PING" => {
            if let Some(arg) = args.get(0) {
                RespValue::Blob(arg.clone())
            } else {
                RespValue::Simple("PONG".to_string())
            }
        }
        "ECHO" => match args.get(0) {
            Some(arg) => RespValue::Blob(arg.clone()),
            None => RespValue::Error("ERR wrong number of arguments for 'ECHO'".to_string()),
        },
        "SELECT" => match args.get(0).and_then(|v| parse_usize(v)) {
            Some(idx) if idx < state.dbs.len() => {
                *db_index = idx;
                RespValue::Simple("OK".to_string())
            }
            _ => RespValue::Error("ERR invalid DB index".to_string()),
        },
        "DBSIZE" => {
            let db = &mut state.dbs[*db_index];
            RespValue::Integer(db.len() as i64)
        }
        "GET" => match args.get(0) {
            Some(key) => {
                let db = &mut state.dbs[*db_index];
                match db.get_string(key) {
                    Ok(Some(v)) => RespValue::Blob(v),
                    Ok(None) => RespValue::Null,
                    Err(_) => RespValue::Error("WRONGTYPE Operation against a key holding the wrong kind of value".to_string()),
                }
            }
            None => RespValue::Error("ERR wrong number of arguments for 'GET'".to_string()),
        },
        "SETNX" => match args.get(0).zip(args.get(1)) {
            Some((key, value)) => {
                let db = &mut state.dbs[*db_index];
                match db.set_nx(key.clone(), value.clone()) {
                    Ok(set) => {
                        if set {
                            if let Some(p) = state.persist.as_ref() {
                                let _ = p.log_command(*db_index, &build_cmd(cmd, args)).await;
                            }
                        }
                        RespValue::Integer(if set { 1 } else { 0 })
                    }
                    Err(_) => RespValue::Error("WRONGTYPE Operation against a key holding the wrong kind of value".to_string()),
                }
            }
            None => RespValue::Error("ERR wrong number of arguments for 'SETNX'".to_string()),
        },
        "MSET" => {
            if args.len() < 2 || args.len() % 2 != 0 {
                return RespValue::Error("ERR wrong number of arguments for 'MSET'".to_string());
            }
            let db = &mut state.dbs[*db_index];
            let mut idx = 0;
            while idx + 1 < args.len() {
                let key = args[idx].clone();
                let value = args[idx + 1].clone();
                db.set_string(key, value, None);
                idx += 2;
            }
            if let Some(p) = state.persist.as_ref() {
                let _ = p.log_command(*db_index, &build_cmd(cmd, args)).await;
            }
            RespValue::Simple("OK".to_string())
        }
        "MGET" => {
            if args.is_empty() {
                return RespValue::Error("ERR wrong number of arguments for 'MGET'".to_string());
            }
            let db = &mut state.dbs[*db_index];
            let mut out = Vec::with_capacity(args.len());
            for key in args {
                match db.get_string(key) {
                    Ok(Some(v)) => out.push(RespValue::Blob(v)),
                    Ok(None) => out.push(RespValue::Null),
                    Err(_) => return RespValue::Error("WRONGTYPE Operation against a key holding the wrong kind of value".to_string()),
                }
            }
            RespValue::Array(out)
        }
        "GETSET" => match args.get(0).zip(args.get(1)) {
            Some((key, value)) => {
                let db = &mut state.dbs[*db_index];
                let prev = match db.get_string(key) {
                    Ok(val) => val,
                    Err(_) => {
                        return RespValue::Error("WRONGTYPE Operation against a key holding the wrong kind of value".to_string())
                    }
                };
                db.set_string(key.clone(), value.clone(), None);
                if let Some(p) = state.persist.as_ref() {
                    let _ = p.log_command(*db_index, &build_cmd(cmd, args)).await;
                }
                match prev {
                    Some(v) => RespValue::Blob(v),
                    None => RespValue::Null,
                }
            }
            None => RespValue::Error("ERR wrong number of arguments for 'GETSET'".to_string()),
        },
        "APPEND" => match args.get(0).zip(args.get(1)) {
            Some((key, value)) => {
                let db = &mut state.dbs[*db_index];
                match db.append(key.clone(), value) {
                    Ok(len) => {
                        if let Some(p) = state.persist.as_ref() {
                            let _ = p.log_command(*db_index, &build_cmd(cmd, args)).await;
                        }
                        RespValue::Integer(len)
                    }
                    Err(_) => RespValue::Error("WRONGTYPE Operation against a key holding the wrong kind of value".to_string()),
                }
            }
            None => RespValue::Error("ERR wrong number of arguments for 'APPEND'".to_string()),
        },
        "INCR" => match args.get(0) {
            Some(key) => {
                let db = &mut state.dbs[*db_index];
                match db.incr_by(key.clone(), 1) {
                    Ok(val) => {
                        if let Some(p) = state.persist.as_ref() {
                            let _ = p.log_command(*db_index, &build_cmd(cmd, args)).await;
                        }
                        RespValue::Integer(val)
                    }
                    Err(_) => RespValue::Error("ERR value is not an integer or out of range".to_string()),
                }
            }
            None => RespValue::Error("ERR wrong number of arguments for 'INCR'".to_string()),
        },
        "INCRBY" => match args.get(0).zip(args.get(1)) {
            Some((key, delta)) => {
                let delta = parse_i64(delta).unwrap_or(0);
                let db = &mut state.dbs[*db_index];
                match db.incr_by(key.clone(), delta) {
                    Ok(val) => {
                        if let Some(p) = state.persist.as_ref() {
                            let _ = p.log_command(*db_index, &build_cmd(cmd, args)).await;
                        }
                        RespValue::Integer(val)
                    }
                    Err(_) => RespValue::Error("ERR value is not an integer or out of range".to_string()),
                }
            }
            None => RespValue::Error("ERR wrong number of arguments for 'INCRBY'".to_string()),
        },
        "DECR" => match args.get(0) {
            Some(key) => {
                let db = &mut state.dbs[*db_index];
                match db.incr_by(key.clone(), -1) {
                    Ok(val) => {
                        if let Some(p) = state.persist.as_ref() {
                            let _ = p.log_command(*db_index, &build_cmd(cmd, args)).await;
                        }
                        RespValue::Integer(val)
                    }
                    Err(_) => RespValue::Error("ERR value is not an integer or out of range".to_string()),
                }
            }
            None => RespValue::Error("ERR wrong number of arguments for 'DECR'".to_string()),
        },
        "DECRBY" => match args.get(0).zip(args.get(1)) {
            Some((key, delta)) => {
                let delta = parse_i64(delta).unwrap_or(0);
                let db = &mut state.dbs[*db_index];
                match db.incr_by(key.clone(), -delta) {
                    Ok(val) => {
                        if let Some(p) = state.persist.as_ref() {
                            let _ = p.log_command(*db_index, &build_cmd(cmd, args)).await;
                        }
                        RespValue::Integer(val)
                    }
                    Err(_) => RespValue::Error("ERR value is not an integer or out of range".to_string()),
                }
            }
            None => RespValue::Error("ERR wrong number of arguments for 'DECRBY'".to_string()),
        },
        "STRLEN" => match args.get(0) {
            Some(key) => {
                let db = &mut state.dbs[*db_index];
                match db.get_string(key) {
                    Ok(Some(v)) => RespValue::Integer(v.len() as i64),
                    Ok(None) => RespValue::Integer(0),
                    Err(_) => RespValue::Error("WRONGTYPE Operation against a key holding the wrong kind of value".to_string()),
                }
            }
            None => RespValue::Error("ERR wrong number of arguments for 'STRLEN'".to_string()),
        },
        "HSET" => {
            if args.len() < 3 || args.len() % 2 == 0 {
                return RespValue::Error("ERR wrong number of arguments for 'HSET'".to_string());
            }
            let key = &args[0];
            let mut added = 0;
            let db = &mut state.dbs[*db_index];
            let mut idx = 1;
            while idx + 1 < args.len() {
                let field = args[idx].clone();
                let value = args[idx + 1].clone();
                match db.hash_set(key, field, value) {
                    Ok(is_new) => {
                        if is_new {
                            added += 1;
                        }
                    }
                    Err(_) => {
                        return RespValue::Error("WRONGTYPE Operation against a key holding the wrong kind of value".to_string());
                    }
                }
                idx += 2;
            }
            if let Some(p) = state.persist.as_ref() {
                let _ = p.log_command(*db_index, &build_cmd(cmd, args)).await;
            }
            RespValue::Integer(added)
        }
        "HGET" => match (args.get(0), args.get(1)) {
            (Some(key), Some(field)) => {
                let db = &mut state.dbs[*db_index];
                match db.hash_get(key, field) {
                    Ok(Some(v)) => RespValue::Blob(v),
                    Ok(None) => RespValue::Null,
                    Err(_) => RespValue::Error("WRONGTYPE Operation against a key holding the wrong kind of value".to_string()),
                }
            }
            _ => RespValue::Error("ERR wrong number of arguments for 'HGET'".to_string()),
        },
        "HDEL" => {
            if args.len() < 2 {
                return RespValue::Error("ERR wrong number of arguments for 'HDEL'".to_string());
            }
            let key = &args[0];
            let fields = &args[1..];
            let db = &mut state.dbs[*db_index];
            match db.hash_del(key, fields) {
                Ok(removed) => {
                    if let Some(p) = state.persist.as_ref() {
                        let _ = p.log_command(*db_index, &build_cmd(cmd, args)).await;
                    }
                    RespValue::Integer(removed)
                }
                Err(_) => RespValue::Error("WRONGTYPE Operation against a key holding the wrong kind of value".to_string()),
            }
        }
        "HGETALL" => match args.get(0) {
            Some(key) => {
                let db = &mut state.dbs[*db_index];
                match db.hash_getall(key) {
                    Ok(items) => {
                        let mut out = Vec::with_capacity(items.len() * 2);
                        for (field, value) in items {
                            out.push(RespValue::Blob(field));
                            out.push(RespValue::Blob(value));
                        }
                        RespValue::Array(out)
                    }
                    Err(_) => RespValue::Error("WRONGTYPE Operation against a key holding the wrong kind of value".to_string()),
                }
            }
            None => RespValue::Error("ERR wrong number of arguments for 'HGETALL'".to_string()),
        },
        "HLEN" => match args.get(0) {
            Some(key) => {
                let db = &mut state.dbs[*db_index];
                match db.hash_len(key) {
                    Ok(len) => RespValue::Integer(len),
                    Err(_) => RespValue::Error("WRONGTYPE Operation against a key holding the wrong kind of value".to_string()),
                }
            }
            None => RespValue::Error("ERR wrong number of arguments for 'HLEN'".to_string()),
        },
        "HEXISTS" => match (args.get(0), args.get(1)) {
            (Some(key), Some(field)) => {
                let db = &mut state.dbs[*db_index];
                match db.hash_exists(key, field) {
                    Ok(exists) => RespValue::Integer(if exists { 1 } else { 0 }),
                    Err(_) => RespValue::Error("WRONGTYPE Operation against a key holding the wrong kind of value".to_string()),
                }
            }
            _ => RespValue::Error("ERR wrong number of arguments for 'HEXISTS'".to_string()),
        },
        "LPUSH" | "RPUSH" => {
            if args.len() < 2 {
                return RespValue::Error(format!("ERR wrong number of arguments for '{}'", cmd));
            }
            let key = &args[0];
            let values = &args[1..];
            let db = &mut state.dbs[*db_index];
            match db.list_push(key, values, cmd == "LPUSH") {
                Ok(len) => {
                    if let Some(p) = state.persist.as_ref() {
                        let _ = p.log_command(*db_index, &build_cmd(cmd, args)).await;
                    }
                    RespValue::Integer(len)
                }
                Err(_) => RespValue::Error("WRONGTYPE Operation against a key holding the wrong kind of value".to_string()),
            }
        }
        "LPOP" | "RPOP" => match args.get(0) {
            Some(key) => {
                let db = &mut state.dbs[*db_index];
                match db.list_pop(key, cmd == "LPOP") {
                    Ok(Some(v)) => {
                        if let Some(p) = state.persist.as_ref() {
                            let _ = p.log_command(*db_index, &build_cmd(cmd, args)).await;
                        }
                        RespValue::Blob(v)
                    }
                    Ok(None) => RespValue::Null,
                    Err(_) => RespValue::Error("WRONGTYPE Operation against a key holding the wrong kind of value".to_string()),
                }
            }
            None => RespValue::Error(format!("ERR wrong number of arguments for '{}'", cmd)),
        },
        "LRANGE" => match (args.get(0), args.get(1), args.get(2)) {
            (Some(key), Some(start), Some(stop)) => {
                let start = parse_i64(start).unwrap_or(0);
                let stop = parse_i64(stop).unwrap_or(-1);
                let db = &mut state.dbs[*db_index];
                match db.list_range(key, start, stop) {
                    Ok(items) => RespValue::Array(items.into_iter().map(RespValue::Blob).collect()),
                    Err(_) => RespValue::Error("WRONGTYPE Operation against a key holding the wrong kind of value".to_string()),
                }
            }
            _ => RespValue::Error("ERR wrong number of arguments for 'LRANGE'".to_string()),
        },
        "LLEN" => match args.get(0) {
            Some(key) => {
                let db = &mut state.dbs[*db_index];
                match db.list_len(key) {
                    Ok(len) => RespValue::Integer(len),
                    Err(_) => RespValue::Error("WRONGTYPE Operation against a key holding the wrong kind of value".to_string()),
                }
            }
            None => RespValue::Error("ERR wrong number of arguments for 'LLEN'".to_string()),
        },
        "SADD" => {
            if args.len() < 2 {
                return RespValue::Error("ERR wrong number of arguments for 'SADD'".to_string());
            }
            let key = &args[0];
            let members = &args[1..];
            let db = &mut state.dbs[*db_index];
            match db.set_add(key, members) {
                Ok(added) => {
                    if let Some(p) = state.persist.as_ref() {
                        let _ = p.log_command(*db_index, &build_cmd(cmd, args)).await;
                    }
                    RespValue::Integer(added)
                }
                Err(_) => RespValue::Error("WRONGTYPE Operation against a key holding the wrong kind of value".to_string()),
            }
        }
        "SREM" => {
            if args.len() < 2 {
                return RespValue::Error("ERR wrong number of arguments for 'SREM'".to_string());
            }
            let key = &args[0];
            let members = &args[1..];
            let db = &mut state.dbs[*db_index];
            match db.set_remove(key, members) {
                Ok(removed) => {
                    if let Some(p) = state.persist.as_ref() {
                        let _ = p.log_command(*db_index, &build_cmd(cmd, args)).await;
                    }
                    RespValue::Integer(removed)
                }
                Err(_) => RespValue::Error("WRONGTYPE Operation against a key holding the wrong kind of value".to_string()),
            }
        }
        "SMEMBERS" => match args.get(0) {
            Some(key) => {
                let db = &mut state.dbs[*db_index];
                match db.set_members(key) {
                    Ok(members) => {
                        let mut out = Vec::with_capacity(members.len());
                        for member in members {
                            out.push(RespValue::Blob(member));
                        }
                        RespValue::Array(out)
                    }
                    Err(_) => RespValue::Error("WRONGTYPE Operation against a key holding the wrong kind of value".to_string()),
                }
            }
            None => RespValue::Error("ERR wrong number of arguments for 'SMEMBERS'".to_string()),
        },
        "SISMEMBER" => match (args.get(0), args.get(1)) {
            (Some(key), Some(member)) => {
                let db = &mut state.dbs[*db_index];
                match db.set_is_member(key, member) {
                    Ok(exists) => RespValue::Integer(if exists { 1 } else { 0 }),
                    Err(_) => RespValue::Error("WRONGTYPE Operation against a key holding the wrong kind of value".to_string()),
                }
            }
            _ => RespValue::Error("ERR wrong number of arguments for 'SISMEMBER'".to_string()),
        },
        "SCARD" => match args.get(0) {
            Some(key) => {
                let db = &mut state.dbs[*db_index];
                match db.set_card(key) {
                    Ok(len) => RespValue::Integer(len),
                    Err(_) => RespValue::Error("WRONGTYPE Operation against a key holding the wrong kind of value".to_string()),
                }
            }
            None => RespValue::Error("ERR wrong number of arguments for 'SCARD'".to_string()),
        },
        "SMOVE" => match (args.get(0), args.get(1), args.get(2)) {
            (Some(source), Some(dest), Some(member)) => {
                let db = &mut state.dbs[*db_index];
                match db.set_move(source, dest, member) {
                    Ok(moved) => {
                        if moved {
                            if let Some(p) = state.persist.as_ref() {
                                let _ = p.log_command(*db_index, &build_cmd(cmd, args)).await;
                            }
                        }
                        RespValue::Integer(if moved { 1 } else { 0 })
                    }
                    Err(_) => RespValue::Error("WRONGTYPE Operation against a key holding the wrong kind of value".to_string()),
                }
            }
            _ => RespValue::Error("ERR wrong number of arguments for 'SMOVE'".to_string()),
        },
        "ZADD" => {
            if args.len() < 3 || (args.len() - 1) % 2 != 0 {
                return RespValue::Error("ERR wrong number of arguments for 'ZADD'".to_string());
            }
            let key = &args[0];
            let db = &mut state.dbs[*db_index];
            let mut added = 0;
            let mut idx = 1;
            while idx + 1 < args.len() {
                let score = parse_f64(&args[idx]).unwrap_or(0.0);
                let member = args[idx + 1].clone();
                match db.zadd(key, score, member) {
                    Ok(is_new) => {
                        if is_new {
                            added += 1;
                        }
                    }
                    Err(_) => {
                        return RespValue::Error("WRONGTYPE Operation against a key holding the wrong kind of value".to_string());
                    }
                }
                idx += 2;
            }
            if let Some(p) = state.persist.as_ref() {
                let _ = p.log_command(*db_index, &build_cmd(cmd, args)).await;
            }
            RespValue::Integer(added)
        }
        "ZRANGE" => match (args.get(0), args.get(1), args.get(2)) {
            (Some(key), Some(start), Some(stop)) => {
                let start = parse_i64(start).unwrap_or(0);
                let stop = parse_i64(stop).unwrap_or(-1);
                let db = &mut state.dbs[*db_index];
                match db.zrange(key, start, stop) {
                    Ok(items) => RespValue::Array(items.into_iter().map(RespValue::Blob).collect()),
                    Err(_) => RespValue::Error("WRONGTYPE Operation against a key holding the wrong kind of value".to_string()),
                }
            }
            _ => RespValue::Error("ERR wrong number of arguments for 'ZRANGE'".to_string()),
        },
        "ZREM" => {
            if args.len() < 2 {
                return RespValue::Error("ERR wrong number of arguments for 'ZREM'".to_string());
            }
            let key = &args[0];
            let members = &args[1..];
            let db = &mut state.dbs[*db_index];
            match db.zrem(key, members) {
                Ok(removed) => {
                    if removed > 0 {
                        if let Some(p) = state.persist.as_ref() {
                            let _ = p.log_command(*db_index, &build_cmd(cmd, args)).await;
                        }
                    }
                    RespValue::Integer(removed)
                }
                Err(_) => RespValue::Error("WRONGTYPE Operation against a key holding the wrong kind of value".to_string()),
            }
        }
        "ZCARD" => match args.get(0) {
            Some(key) => {
                let db = &mut state.dbs[*db_index];
                match db.zcard(key) {
                    Ok(len) => RespValue::Integer(len),
                    Err(_) => RespValue::Error("WRONGTYPE Operation against a key holding the wrong kind of value".to_string()),
                }
            }
            None => RespValue::Error("ERR wrong number of arguments for 'ZCARD'".to_string()),
        },
        "XADD" => {
            if args.len() < 4 || args.len() % 2 != 0 {
                return RespValue::Error("ERR wrong number of arguments for 'XADD'".to_string());
            }
            let key = &args[0];
            let id = match std::str::from_utf8(&args[1]) {
                Ok(v) => v,
                Err(_) => return RespValue::Error("ERR invalid stream ID".to_string()),
            };
            let mut fields = Vec::new();
            let mut idx = 2;
            while idx + 1 < args.len() {
                fields.push((args[idx].clone(), args[idx + 1].clone()));
                idx += 2;
            }
            let db = &mut state.dbs[*db_index];
            match db.stream_add(key, id, fields) {
                Ok(new_id) => {
                    if let Some(p) = state.persist.as_ref() {
                        let _ = p.log_command(*db_index, &build_cmd(cmd, args)).await;
                    }
                    RespValue::Blob(new_id.into_bytes())
                }
                Err(_) => RespValue::Error("ERR invalid stream ID".to_string()),
            }
        }
        "XRANGE" => match (args.get(0), args.get(1), args.get(2)) {
            (Some(key), Some(start), Some(end)) => {
                let start = std::str::from_utf8(start).unwrap_or("-");
                let end = std::str::from_utf8(end).unwrap_or("+");
                let db = &mut state.dbs[*db_index];
                match db.stream_range(key, start, end) {
                    Ok(items) => {
                        let mut out = Vec::with_capacity(items.len());
                        for (id, fields) in items {
                            let mut field_array = Vec::with_capacity(fields.len() * 2);
                            for (field, value) in fields {
                                field_array.push(RespValue::Blob(field));
                                field_array.push(RespValue::Blob(value));
                            }
                            out.push(RespValue::Array(vec![
                                RespValue::Blob(id.into_bytes()),
                                RespValue::Array(field_array),
                            ]));
                        }
                        RespValue::Array(out)
                    }
                    Err(_) => RespValue::Error("WRONGTYPE Operation against a key holding the wrong kind of value".to_string()),
                }
            }
            _ => RespValue::Error("ERR wrong number of arguments for 'XRANGE'".to_string()),
        },
        "SET" => match parse_set_args(args) {
            Ok((key, value, expire_ms)) => {
                let db = &mut state.dbs[*db_index];
                let expire_at = expire_ms.map(|ms| now_ms().saturating_add(ms));
                db.set_string(key, value, expire_at);
                if let Some(p) = state.persist.as_ref() {
                    let _ = p.log_command(*db_index, &build_cmd(cmd, args)).await;
                }
                RespValue::Simple("OK".to_string())
            }
            Err(e) => RespValue::Error(e),
        },
        "DEL" => {
            let mut removed = 0;
            let db = &mut state.dbs[*db_index];
            for key in args {
                if db.remove(key) {
                    removed += 1;
                }
            }
            if let Some(p) = state.persist.as_ref() {
                let _ = p.log_command(*db_index, &build_cmd(cmd, args)).await;
            }
            RespValue::Integer(removed)
        }
        "EXISTS" => {
            let mut count = 0;
            let db = &mut state.dbs[*db_index];
            for key in args {
                if db.exists(key) {
                    count += 1;
                }
            }
            RespValue::Integer(count)
        }
        "EXPIRE" => match (args.get(0), args.get(1)) {
            (Some(key), Some(sec)) => {
                let db = &mut state.dbs[*db_index];
                let ms = parse_u64(sec).unwrap_or(0).saturating_mul(1000);
                let ok = db.set_expire_ms(key, ms);
                if ok {
                    if let Some(p) = state.persist.as_ref() {
                        let _ = p.log_command(*db_index, &build_cmd(cmd, args)).await;
                    }
                }
                RespValue::Integer(if ok { 1 } else { 0 })
            }
            _ => RespValue::Error("ERR wrong number of arguments for 'EXPIRE'".to_string()),
        },
        "PEXPIRE" => match (args.get(0), args.get(1)) {
            (Some(key), Some(ms)) => {
                let db = &mut state.dbs[*db_index];
                let ms = parse_u64(ms).unwrap_or(0);
                let ok = db.set_expire_ms(key, ms);
                if ok {
                    if let Some(p) = state.persist.as_ref() {
                        let _ = p.log_command(*db_index, &build_cmd(cmd, args)).await;
                    }
                }
                RespValue::Integer(if ok { 1 } else { 0 })
            }
            _ => RespValue::Error("ERR wrong number of arguments for 'PEXPIRE'".to_string()),
        },
        "PERSIST" => match args.get(0) {
            Some(key) => {
                let db = &mut state.dbs[*db_index];
                let removed = db.persist(key);
                if removed == 1 {
                    if let Some(p) = state.persist.as_ref() {
                        let _ = p.log_command(*db_index, &build_cmd(cmd, args)).await;
                    }
                }
                RespValue::Integer(removed)
            }
            None => RespValue::Error("ERR wrong number of arguments for 'PERSIST'".to_string()),
        },
        "TTL" => match args.get(0) {
            Some(key) => {
                let db = &mut state.dbs[*db_index];
                match db.ttl_ms(key) {
                    Some(ms) if ms >= 0 => RespValue::Integer((ms / 1000) as i64),
                    Some(ms) => RespValue::Integer(ms),
                    None => RespValue::Integer(-2),
                }
            }
            None => RespValue::Error("ERR wrong number of arguments for 'TTL'".to_string()),
        },
        "PTTL" => match args.get(0) {
            Some(key) => {
                let db = &mut state.dbs[*db_index];
                match db.ttl_ms(key) {
                    Some(ms) => RespValue::Integer(ms),
                    None => RespValue::Integer(-2),
                }
            }
            None => RespValue::Error("ERR wrong number of arguments for 'PTTL'".to_string()),
        },
        "TYPE" => match args.get(0) {
            Some(key) => {
                let db = &mut state.dbs[*db_index];
                match db.value_type(key) {
                    Some(t) => RespValue::Simple(t.to_string()),
                    None => RespValue::Simple("none".to_string()),
                }
            }
            None => RespValue::Error("ERR wrong number of arguments for 'TYPE'".to_string()),
        },
        "KEYS" => match args.get(0) {
            Some(pattern) => {
                let db = &mut state.dbs[*db_index];
                let keys = db.keys_matching(pattern);
                RespValue::Array(keys.into_iter().map(RespValue::Blob).collect())
            }
            None => RespValue::Error("ERR wrong number of arguments for 'KEYS'".to_string()),
        },
        "SCAN" => match args.get(0) {
            Some(cursor) => {
                let cursor_str = match core::str::from_utf8(cursor) {
                    Ok(s) => s,
                    Err(_) => return RespValue::Error("ERR invalid cursor".to_string()),
                };
                let mut cursor_val = match cursor_str.parse::<usize>() {
                    Ok(v) => v,
                    Err(_) => return RespValue::Error("ERR invalid cursor".to_string()),
                };
                let mut pattern = b"*".to_vec();
                let mut count = 10usize;
                let mut i = 1usize;
                while i < args.len() {
                    let opt = args[i].to_ascii_uppercase();
                    if opt == b"MATCH" {
                        if i + 1 >= args.len() {
                            return RespValue::Error("ERR syntax error".to_string());
                        }
                        pattern = args[i + 1].clone();
                        i += 2;
                        continue;
                    }
                    if opt == b"COUNT" {
                        if i + 1 >= args.len() {
                            return RespValue::Error("ERR syntax error".to_string());
                        }
                        let cnt_str = match core::str::from_utf8(&args[i + 1]) {
                            Ok(s) => s,
                            Err(_) => return RespValue::Error("ERR invalid COUNT".to_string()),
                        };
                        count = match cnt_str.parse::<usize>() {
                            Ok(v) => v,
                            Err(_) => return RespValue::Error("ERR invalid COUNT".to_string()),
                        };
                        i += 2;
                        continue;
                    }
                    return RespValue::Error("ERR syntax error".to_string());
                }
                let db = &mut state.dbs[*db_index];
                let keys = db.keys_matching(&pattern);
                if cursor_val > keys.len() {
                    cursor_val = keys.len();
                }
                let end = (cursor_val + count).min(keys.len());
                let batch = keys[cursor_val..end].to_vec();
                let next_cursor = if end >= keys.len() { 0 } else { end };
                let mut out = Vec::with_capacity(2);
                out.push(RespValue::Blob(next_cursor.to_string().into_bytes()));
                out.push(RespValue::Array(batch.into_iter().map(RespValue::Blob).collect()));
                RespValue::Array(out)
            }
            None => RespValue::Error("ERR wrong number of arguments for 'SCAN'".to_string()),
        },
        "FLUSHDB" => {
            if !args.is_empty() {
                return RespValue::Error("ERR wrong number of arguments for 'FLUSHDB'".to_string());
            }
            let db = &mut state.dbs[*db_index];
            db.flush();
            if let Some(p) = state.persist.as_ref() {
                let _ = p.log_command(*db_index, &build_cmd(cmd, args)).await;
            }
            RespValue::Simple("OK".to_string())
        }
        "FLUSHALL" => {
            if !args.is_empty() {
                return RespValue::Error("ERR wrong number of arguments for 'FLUSHALL'".to_string());
            }
            for db in state.dbs.iter_mut() {
                db.flush();
            }
            if let Some(p) = state.persist.as_ref() {
                let _ = p.log_command(0, &build_cmd(cmd, args)).await;
            }
            RespValue::Simple("OK".to_string())
        }
        "INFO" => RespValue::Blob(b"mini-redis:1\r\n".to_vec()),
        "EVAL" => {
            if script.is_none() {
                *script = Some(ScriptRuntime::new());
            }
            eval_script(state, db_index, script.as_mut().unwrap(), args, true)
        }
        "EVALSHA" => {
            if script.is_none() {
                *script = Some(ScriptRuntime::new());
            }
            eval_script_sha(state, db_index, script.as_mut().unwrap(), args)
        }
        "SCRIPT" => handle_script_command(state, args),
        "CONFIG" => {
            if args.len() >= 1 && to_upper_ascii(&args[0]) == "GET" {
                RespValue::Array(Vec::new())
            } else {
                RespValue::Error("ERR syntax error".to_string())
            }
        }
        "FUNCTION" => {
            if args.is_empty() {
                return RespValue::Error("ERR wrong number of arguments for 'FUNCTION'".to_string());
            }
            let sub = to_upper_ascii(&args[0]);
            match sub.as_str() {
                "LIST" => RespValue::Array(Vec::new()),
                "FLUSH" => {
                    state.script_cache.clear();
                    RespValue::Simple("OK".to_string())
                }
                "LOAD" => {
                    if args.len() != 2 {
                        return RespValue::Error("ERR wrong number of arguments for 'FUNCTION LOAD'".to_string());
                    }
                    let script = match std::str::from_utf8(&args[1]) {
                        Ok(s) => s,
                        Err(_) => return RespValue::Error("ERR invalid function".to_string()),
                    };
                    let sha = sha1_hex(script.as_bytes());
                    state.script_cache.insert(sha, script.to_string());
                    RespValue::Simple("OK".to_string())
                }
                _ => RespValue::Error("ERR unknown subcommand for FUNCTION".to_string()),
            }
        }
        "CLIENT" => {
            if args.len() >= 1 && to_upper_ascii(&args[0]) == "LIST" {
                RespValue::Blob(b"id=1 addr=127.0.0.1:0".to_vec())
            } else {
                RespValue::Error("ERR syntax error".to_string())
            }
        }
        "SLOWLOG" => {
            if args.len() >= 1 && to_upper_ascii(&args[0]) == "GET" {
                RespValue::Array(Vec::new())
            } else {
                RespValue::Error("ERR syntax error".to_string())
            }
        }
        "SAVE" => RespValue::Error("ERR persistence not implemented".to_string()),
        "BGSAVE" => RespValue::Error("ERR persistence not implemented".to_string()),
        "REPLICAOF" => RespValue::Error("ERR replication not implemented".to_string()),
        "QUIT" => RespValue::Simple("OK".to_string()),
        _ => RespValue::Error(format!("ERR unknown command '{}'", cmd)),
    }
}

fn eval_script(
    state: &mut ServerState,
    db_index: &mut usize,
    script_runtime: &mut ScriptRuntime,
    args: &[Vec<u8>],
    cache: bool,
) -> RespValue {
    if args.len() < 2 {
        return RespValue::Error("ERR wrong number of arguments for 'EVAL'".to_string());
    }
    let script = match std::str::from_utf8(&args[0]) {
        Ok(s) => s,
        Err(_) => return RespValue::Error("ERR invalid script".to_string()),
    };
    if cache {
        let sha = sha1_hex(script.as_bytes());
        state.script_cache.insert(sha, script.to_string());
    }
    let numkeys = match parse_usize(&args[1]) {
        Some(n) => n,
        None => return RespValue::Error("ERR invalid number of keys".to_string()),
    };
    if args.len() < 2 + numkeys {
        return RespValue::Error("ERR invalid number of keys".to_string());
    }
    let keys = &args[2..2 + numkeys];
    let argv = &args[2 + numkeys..];
    script_runtime.set_keys_argv(keys, argv);
    let mut exec = ScriptExec {
        state: state as *mut ServerState,
        db_index: db_index as *mut usize,
    };
    let ctx = &mut script_runtime.ctx;
    JS_SetContextOpaque(ctx, &mut exec as *mut ScriptExec as *mut c_void);
    let wrapped = format!("function __redis_script__(){{\n{}\n}}\n__redis_script__()", script);
    let result = JS_Eval(ctx, &wrapped, "<eval>", JS_EVAL_RETVAL);
    JS_SetContextOpaque(ctx, std::ptr::null_mut());
    if result.is_exception() {
        let exc = JS_GetException(ctx);
        let msg = js_value_to_string(ctx, exc);
        return RespValue::Error(msg);
    }
    js_to_resp(ctx, result)
}

fn eval_script_sha(
    state: &mut ServerState,
    db_index: &mut usize,
    script_runtime: &mut ScriptRuntime,
    args: &[Vec<u8>],
) -> RespValue {
    if args.len() < 2 {
        return RespValue::Error("ERR wrong number of arguments for 'EVALSHA'".to_string());
    }
    let sha = match std::str::from_utf8(&args[0]) {
        Ok(s) => s,
        Err(_) => return RespValue::Error("ERR invalid script".to_string()),
    };
    let script = match state.script_cache.get(sha) {
        Some(s) => s.clone(),
        None => return RespValue::Error("NOSCRIPT No matching script. Please use EVAL.".to_string()),
    };
    let mut new_args = Vec::with_capacity(args.len());
    new_args.push(script.into_bytes());
    new_args.extend_from_slice(&args[1..]);
    eval_script(state, db_index, script_runtime, &new_args, false)
}

fn handle_script_command(state: &mut ServerState, args: &[Vec<u8>]) -> RespValue {
    if args.len() < 1 {
        return RespValue::Error("ERR wrong number of arguments for 'SCRIPT'".to_string());
    }
    let sub = to_upper_ascii(&args[0]);
    match sub.as_str() {
        "LOAD" => {
            if args.len() != 2 {
                return RespValue::Error("ERR wrong number of arguments for 'SCRIPT LOAD'".to_string());
            }
            let script = match std::str::from_utf8(&args[1]) {
                Ok(s) => s,
                Err(_) => return RespValue::Error("ERR invalid script".to_string()),
            };
            let sha = sha1_hex(script.as_bytes());
            state.script_cache.insert(sha.clone(), script.to_string());
            RespValue::Blob(sha.into_bytes())
        }
        "EXISTS" => {
            if args.len() < 2 {
                return RespValue::Error("ERR wrong number of arguments for 'SCRIPT EXISTS'".to_string());
            }
            let mut out = Vec::with_capacity(args.len() - 1);
            for sha in &args[1..] {
                let s = match std::str::from_utf8(sha) {
                    Ok(v) => v,
                    Err(_) => "",
                };
                let exists = state.script_cache.contains_key(s);
                out.push(RespValue::Integer(if exists { 1 } else { 0 }));
            }
            RespValue::Array(out)
        }
        "FLUSH" => {
            state.script_cache.clear();
            RespValue::Simple("OK".to_string())
        }
        _ => RespValue::Error("ERR unknown subcommand for SCRIPT".to_string()),
    }
}

fn sha1_hex(input: &[u8]) -> String {
    let mut h0: u32 = 0x67452301;
    let mut h1: u32 = 0xEFCDAB89;
    let mut h2: u32 = 0x98BADCFE;
    let mut h3: u32 = 0x10325476;
    let mut h4: u32 = 0xC3D2E1F0;
    let mut msg = input.to_vec();
    let bit_len = (msg.len() as u64) * 8;
    msg.push(0x80);
    while (msg.len() % 64) != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 80];
        for (i, word) in w.iter_mut().take(16).enumerate() {
            let start = i * 4;
            *word = u32::from_be_bytes([chunk[start], chunk[start + 1], chunk[start + 2], chunk[start + 3]]);
        }
        for i in 16..80 {
            let v = w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16];
            w[i] = v.rotate_left(1);
        }
        let mut a = h0;
        let mut b = h1;
        let mut c = h2;
        let mut d = h3;
        let mut e = h4;
        for i in 0..80 {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(w[i]);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        h0 = h0.wrapping_add(a);
        h1 = h1.wrapping_add(b);
        h2 = h2.wrapping_add(c);
        h3 = h3.wrapping_add(d);
        h4 = h4.wrapping_add(e);
    }
    format!("{:08x}{:08x}{:08x}{:08x}{:08x}", h0, h1, h2, h3, h4)
}

struct ScriptRuntime {
    mem: Vec<u8>,
    ctx: JSContextImpl,
    cfuncs: Vec<JSCFunctionDef>,
}

unsafe impl Send for ScriptRuntime {}

impl ScriptRuntime {
    fn new() -> Self {
        let mut mem = vec![0u8; SCRIPT_MEM_SIZE];
        let mut ctx = crate::JS_NewContext(&mut mem);
        let _ = JS_RegisterStdlibMinimal(&mut ctx);
        let cfuncs = vec![
            JSCFunctionDef {
                func: JSCFunctionType { generic_magic: Some(redis_call) },
                name: JSValue::UNDEFINED,
                def_type: JSCFunctionDefEnum::GenericMagic as u8,
                arg_count: 1,
                magic: 0,
            },
            JSCFunctionDef {
                func: JSCFunctionType { generic_magic: Some(redis_call) },
                name: JSValue::UNDEFINED,
                def_type: JSCFunctionDefEnum::GenericMagic as u8,
                arg_count: 1,
                magic: 1,
            },
        ];
        JS_SetCFunctionTable(&mut ctx, &cfuncs);
        let redis_obj = JS_NewObject(&mut ctx);
        let call_fn = JS_NewCFunctionParams(&mut ctx, 0, JSValue::UNDEFINED);
        let pcall_fn = JS_NewCFunctionParams(&mut ctx, 1, JSValue::UNDEFINED);
        let _ = JS_SetPropertyStr(&mut ctx, redis_obj, "call", call_fn);
        let _ = JS_SetPropertyStr(&mut ctx, redis_obj, "pcall", pcall_fn);
        let global = JS_GetGlobalObject(&mut ctx);
        let _ = JS_SetPropertyStr(&mut ctx, global, "redis", redis_obj);
        Self { mem, ctx, cfuncs }
    }

    fn set_keys_argv(&mut self, keys: &[Vec<u8>], argv: &[Vec<u8>]) {
        let global = JS_GetGlobalObject(&mut self.ctx);
        let keys_arr = JS_NewArray(&mut self.ctx, keys.len() as i32);
        for (idx, key) in keys.iter().enumerate() {
            let v = JS_NewStringLen(&mut self.ctx, key);
            let _ = JS_SetPropertyUint32(&mut self.ctx, keys_arr, idx as u32, v);
        }
        let argv_arr = JS_NewArray(&mut self.ctx, argv.len() as i32);
        for (idx, arg) in argv.iter().enumerate() {
            let v = JS_NewStringLen(&mut self.ctx, arg);
            let _ = JS_SetPropertyUint32(&mut self.ctx, argv_arr, idx as u32, v);
        }
        let _ = JS_SetPropertyStr(&mut self.ctx, global, "KEYS", keys_arr);
        let _ = JS_SetPropertyStr(&mut self.ctx, global, "ARGV", argv_arr);
    }
}

struct ScriptExec {
    state: *mut ServerState,
    db_index: *mut usize,
}

fn redis_call(
    ctx: *mut crate::JSContext,
    _this_val: *mut JSValue,
    argc: i32,
    argv: *mut JSValue,
    magic: i32,
) -> JSValue {
    if argc < 1 {
        unsafe {
            let ctx = &mut *(ctx as *mut JSContextImpl);
            return JS_ThrowInternalError(ctx, "redis.call requires at least one argument");
        }
    }
    unsafe {
        let ctx = &mut *(ctx as *mut JSContextImpl);
        let opaque = ctx.opaque() as *mut ScriptExec;
        if opaque.is_null() {
            return JS_ThrowInternalError(ctx, "redis.call missing context");
        }
        let exec = &mut *opaque;
        let mut args = Vec::with_capacity(argc as usize);
        for i in 0..argc {
            let val = *argv.add(i as usize);
            let s = js_value_to_string(ctx, val);
            args.push(s.into_bytes());
        }
        let cmd = to_upper_ascii(&args[0]);
        let state = &mut *exec.state;
        let db_index = &mut *exec.db_index;
        let mut script = None;
        let resp = async_std::task::block_on(handle_command(state, db_index, &mut script, &cmd, &args[1..]));
        resp_to_js(ctx, resp, magic == 1)
    }
}

fn resp_to_js(ctx: &mut JSContextImpl, resp: RespValue, is_pcall: bool) -> JSValue {
    match resp {
        RespValue::Simple(s) => JS_NewString(ctx, &s),
        RespValue::Blob(b) => JS_NewStringLen(ctx, &b),
        RespValue::Integer(n) => JS_NewInt64(ctx, n),
        RespValue::Null => JSValue::NULL,
        RespValue::Array(items) => {
            let arr = JS_NewArray(ctx, items.len() as i32);
            for (idx, item) in items.into_iter().enumerate() {
                let v = resp_to_js(ctx, item, is_pcall);
                let _ = JS_SetPropertyUint32(ctx, arr, idx as u32, v);
            }
            arr
        }
        RespValue::Error(msg) => {
            if is_pcall {
                let obj = JS_NewObject(ctx);
                let err_val = JS_NewString(ctx, &msg);
                let _ = JS_SetPropertyStr(ctx, obj, "err", err_val);
                obj
            } else {
                JS_ThrowInternalError(ctx, &msg)
            }
        }
    }
}

fn js_to_resp(ctx: &mut JSContextImpl, val: JSValue) -> RespValue {
    if JS_IsNull(ctx, val) != 0 || JS_IsUndefined(ctx, val) != 0 {
        return RespValue::Null;
    }
    if JS_IsBool(ctx, val) != 0 {
        let num = JS_ToNumber(ctx, val).unwrap_or(0.0);
        return RespValue::Integer(if num != 0.0 { 1 } else { 0 });
    }
    if JS_IsNumber(ctx, val) != 0 {
        let num = JS_ToNumber(ctx, val).unwrap_or(0.0);
        if (num.fract() - 0.0).abs() < f64::EPSILON {
            return RespValue::Integer(num as i64);
        }
        return RespValue::Blob(num.to_string().into_bytes());
    }
    if JS_IsString(ctx, val) != 0 {
        let s = js_value_to_string(ctx, val);
        return RespValue::Blob(s.into_bytes());
    }
    if ctx.object_class_id(val) == Some(crate::JSObjectClassEnum::Array as u32) {
        let len_val = crate::JS_GetPropertyStr(ctx, val, "length");
        let len = crate::JS_ToUint32(ctx, len_val).unwrap_or(0);
        let mut out = Vec::with_capacity(len as usize);
        for i in 0..len {
            let item = crate::JS_GetPropertyUint32(ctx, val, i);
            out.push(js_to_resp(ctx, item));
        }
        return RespValue::Array(out);
    }
    let err = crate::JS_GetPropertyStr(ctx, val, "err");
    if JS_IsUndefined(ctx, err) == 0 && JS_IsNull(ctx, err) == 0 {
        let msg = js_value_to_string(ctx, err);
        return RespValue::Error(msg);
    }
    let ok = crate::JS_GetPropertyStr(ctx, val, "ok");
    if JS_IsUndefined(ctx, ok) == 0 && JS_IsNull(ctx, ok) == 0 {
        let msg = js_value_to_string(ctx, ok);
        return RespValue::Simple(msg);
    }
    let s = js_value_to_string(ctx, val);
    RespValue::Blob(s.into_bytes())
}

fn js_value_to_string(ctx: &mut JSContextImpl, val: JSValue) -> String {
    let mut buf = JSCStringBuf { buf: [0u8; 5] };
    JS_ToCString(ctx, val, &mut buf).to_string()
}

fn value_to_args(val: RespValue) -> Result<Vec<Vec<u8>>, String> {
    match val {
        RespValue::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                match item {
                    RespValue::Blob(b) => out.push(b),
                    RespValue::Simple(s) => out.push(s.into_bytes()),
                    RespValue::Integer(n) => out.push(n.to_string().into_bytes()),
                    _ => return Err("ERR invalid array item".to_string()),
                }
            }
            Ok(out)
        }
        RespValue::Simple(s) => Ok(vec![s.into_bytes()]),
        RespValue::Blob(b) => Ok(vec![b]),
        _ => Err("ERR invalid request".to_string()),
    }
}

fn parse_set_args(args: &[Vec<u8>]) -> Result<(Vec<u8>, Vec<u8>, Option<u64>), String> {
    if args.len() < 2 {
        return Err("ERR wrong number of arguments for 'SET'".to_string());
    }
    let key = args[0].clone();
    let value = args[1].clone();
    let mut expire_ms = None;
    let mut idx = 2;
    while idx < args.len() {
        let opt = to_upper_ascii(&args[idx]);
        if opt == "EX" {
            idx += 1;
            let sec = args.get(idx).ok_or_else(|| "ERR syntax error".to_string())?;
            expire_ms = Some(parse_u64(sec).unwrap_or(0).saturating_mul(1000));
        } else if opt == "PX" {
            idx += 1;
            let ms = args.get(idx).ok_or_else(|| "ERR syntax error".to_string())?;
            expire_ms = Some(parse_u64(ms).unwrap_or(0));
        } else {
            return Err("ERR syntax error".to_string());
        }
        idx += 1;
    }
    Ok((key, value, expire_ms))
}

fn parse_usize(input: &[u8]) -> Option<usize> {
    core::str::from_utf8(input).ok()?.parse::<usize>().ok()
}

fn parse_u64(input: &[u8]) -> Option<u64> {
    core::str::from_utf8(input).ok()?.parse::<u64>().ok()
}

fn parse_i64(input: &[u8]) -> Option<i64> {
    core::str::from_utf8(input).ok()?.parse::<i64>().ok()
}

fn parse_f64(input: &[u8]) -> Option<f64> {
    core::str::from_utf8(input).ok()?.parse::<f64>().ok()
}

fn to_upper_ascii(input: &[u8]) -> String {
    input.iter().map(|b| b.to_ascii_uppercase() as char).collect()
}

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn build_cmd(cmd: &str, args: &[Vec<u8>]) -> Vec<Vec<u8>> {
    let mut out = Vec::with_capacity(args.len() + 1);
    out.push(cmd.as_bytes().to_vec());
    for arg in args {
        out.push(arg.clone());
    }
    out
}

async fn init_persist(config: &ServerConfig) -> io::Result<Option<Persist>> {
    if let Some(path) = config.persist_path.as_ref() {
        #[cfg(feature = "mini-redis-libsql")]
        {
            let p = crate::mini_redis::persist::LibsqlPersist::open(path, config.aof_enabled).await?;
            return Ok(Some(Persist::Libsql(p)));
        }
        #[cfg(not(feature = "mini-redis-libsql"))]
        {
            let _ = path;
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "persist requested but mini-redis-libsql feature is not enabled",
            ));
        }
    }
    Ok(None)
}
