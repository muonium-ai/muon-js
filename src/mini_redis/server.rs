//! RESP3 server and command dispatcher.

use async_std::io::{self, BufReader, BufWriter, WriteExt};
use async_std::net::{TcpListener, TcpStream};
use async_std::sync::{Arc, Mutex};
use async_std::task;
use async_std::channel::{Sender, bounded};
use std::ffi::c_void;
use std::sync::OnceLock;
use std::time::Instant;

use crate::mini_redis::resp::{read_value, write_array_of_blobs_buf, write_value_buf, RespValue};
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

const DEFAULT_SCRIPT_MEM_SIZE: usize = 4 * 1024 * 1024;
const DEFAULT_SCRIPT_RESET_THRESHOLD_PCT: u8 = 90;

#[derive(Clone, Debug)]
pub struct ScriptRuntimeConfig {
    pub mem_size: usize,
    pub reset_threshold_pct: u8,
}

impl Default for ScriptRuntimeConfig {
    fn default() -> Self {
        Self {
            mem_size: DEFAULT_SCRIPT_MEM_SIZE,
            reset_threshold_pct: DEFAULT_SCRIPT_RESET_THRESHOLD_PCT,
        }
    }
}

#[derive(Clone)]
pub struct ServerConfig {
    pub bind: String,
    pub port: u16,
    pub databases: usize,
    pub persist_path: Option<String>,
    pub aof_enabled: bool,
    pub script_runtime: ScriptRuntimeConfig,
}

pub struct ServerState {
    dbs: Vec<Db>,
    script_runtime: ScriptRuntimeConfig,
}

type PubSubState = std::collections::HashMap<Arc<[u8]>, Vec<Sender<RespValue>>>;
type ScriptCacheState = std::collections::HashMap<String, String>;
type PersistState = Option<Persist>;

impl ServerState {
    fn new(dbs: Vec<Db>, script_runtime: ScriptRuntimeConfig) -> Self {
        Self {
            dbs,
            script_runtime,
        }
    }
}

fn timing_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("MINI_REDIS_TIMINGS")
            .ok()
            .map(|v| {
                let v = v.to_ascii_lowercase();
                v == "1" || v == "true" || v == "yes" || v == "on"
            })
            .unwrap_or(false)
    })
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
    let state = Arc::new(Mutex::new(ServerState::new(dbs, config.script_runtime)));
    let persist_state = Arc::new(Mutex::new(persist));
    let pubsub_state = Arc::new(Mutex::new(PubSubState::new()));
    let script_cache_state = Arc::new(Mutex::new(ScriptCacheState::new()));
    let (shutdown_tx, shutdown_rx) = bounded::<()>(1);
    let shutdown_state = state.clone();
    let shutdown_persist_state = persist_state.clone();
    let shutdown_path = config.persist_path.clone();
    if let Err(err) = ctrlc::set_handler(move || {
        let _ = shutdown_tx.try_send(());
    }) {
        eprintln!("mini-redis: failed to install ctrl+c handler: {}", err);
    }
    task::spawn(async move {
        let _ = shutdown_rx.recv().await;
        graceful_shutdown(shutdown_state, shutdown_persist_state, shutdown_path).await;
        std::process::exit(0);
    });
    loop {
        let (stream, _) = listener.accept().await?;
        let state = state.clone();
        let persist_state = persist_state.clone();
        let pubsub_state = pubsub_state.clone();
        let script_cache_state = script_cache_state.clone();
        task::spawn(async move {
            let _ = handle_client(stream, state, persist_state, pubsub_state, script_cache_state).await;
        });
    }
}

async fn graceful_shutdown(
    state: Arc<Mutex<ServerState>>,
    persist_state: Arc<Mutex<PersistState>>,
    persist_path: Option<String>,
) {
    let mut guard = state.lock().await;
    eprintln!("mini-redis: shutdown requested");
    if let Some(db) = guard.dbs.get_mut(0) {
        let items = db.snapshot_items();
        let mut counts = (0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
        for (_, value, _) in items.iter() {
            match value {
                crate::mini_redis::store::Value::String(_) => counts.0 += 1,
                crate::mini_redis::store::Value::List(_) => counts.1 += 1,
                crate::mini_redis::store::Value::Set(_) => counts.2 += 1,
                crate::mini_redis::store::Value::Hash(_) => counts.3 += 1,
                crate::mini_redis::store::Value::ZSet(_) => counts.4 += 1,
                crate::mini_redis::store::Value::Stream(_) => counts.5 += 1,
            }
        }
        eprintln!("mini-redis: db0 keys={}", items.len());
        eprintln!(
            "mini-redis: db0 counts: string={} list={} set={} hash={} zset={} stream={}",
            counts.0, counts.1, counts.2, counts.3, counts.4, counts.5
        );
    }
    let _path_msg = persist_path.as_deref().unwrap_or("<unknown>");
    let persist_guard = persist_state.lock().await;
    if persist_guard.is_none() {
        eprintln!("mini-redis: persistence not configured; skipping snapshot");
        return;
    }
    if let Some(persist) = persist_guard.as_ref() {
        if let Err(err) = persist.snapshot(&mut guard.dbs).await {
            eprintln!("mini-redis: persistence failed: {}", err);
        }
    }
}

async fn handle_client(
    stream: TcpStream,
    state: Arc<Mutex<ServerState>>,
    persist_state: Arc<Mutex<PersistState>>,
    pubsub_state: Arc<Mutex<PubSubState>>,
    script_cache_state: Arc<Mutex<ScriptCacheState>>,
) -> io::Result<()> {
    let peer = stream.peer_addr().ok();
    let mut reader = BufReader::new(stream.clone());
    let mut writer = BufWriter::new(stream);
    let mut resp_buf: Vec<u8> = Vec::with_capacity(1024);
    let mut current_db: usize = 0;
    let mut in_multi = false;
    let mut queued: Vec<(String, Vec<Arc<[u8]>>)> = Vec::new();
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
                write_value_buf(&mut writer, &RespValue::Error(err), &mut resp_buf).await?;
                continue;
            }
        };
        if args.is_empty() {
            write_value_buf(
                &mut writer,
                &RespValue::Error("ERR empty command".to_string()),
                &mut resp_buf,
            )
            .await?;
            continue;
        }
        let cmd = match parse_command(args[0].as_ref()) {
            Some(cmd) => cmd,
            None => {
                let unknown = String::from_utf8_lossy(args[0].as_ref());
                write_value_buf(
                    &mut writer,
                    &RespValue::Error(format!("ERR unknown command '{}'", unknown)),
                    &mut resp_buf,
                )
                .await?;
                continue;
            }
        };
        let timing = if timing_enabled() {
            Some(Instant::now())
        } else {
            None
        };
        let resp = if cmd == "SUBSCRIBE" {
            FastResponse::Value(handle_subscribe(&pubsub_state, &pub_tx, &args[1..]).await)
        } else if cmd == "PUBLISH" {
            FastResponse::Value(handle_publish(&pubsub_state, &args[1..]).await)
        } else if cmd == "LRANGE" && !in_multi {
            match handle_lrange_fast(&state, &mut current_db, &args[1..]).await {
                Ok(items) => FastResponse::BlobArray(items),
                Err(err) => FastResponse::Value(err),
            }
        } else if in_multi && cmd != "EXEC" && cmd != "DISCARD" && cmd != "MULTI" {
            queued.push((cmd.to_string(), args[1..].to_vec()));
            FastResponse::Value(RespValue::Simple("QUEUED".to_string()))
        } else if cmd == "MULTI" {
            if in_multi {
                FastResponse::Value(RespValue::Error("ERR MULTI calls can not be nested".to_string()))
            } else {
                in_multi = true;
                queued.clear();
                FastResponse::Value(RespValue::Simple("OK".to_string()))
            }
        } else if cmd == "DISCARD" {
            if !in_multi {
                FastResponse::Value(RespValue::Error("ERR DISCARD without MULTI".to_string()))
            } else {
                in_multi = false;
                queued.clear();
                FastResponse::Value(RespValue::Simple("OK".to_string()))
            }
        } else if cmd == "EXEC" {
            if !in_multi {
                FastResponse::Value(RespValue::Error("ERR EXEC without MULTI".to_string()))
            } else {
                in_multi = false;
                let mut results = Vec::with_capacity(queued.len());
                let mut guard = state.lock().await;
                for (qcmd, qargs) in queued.drain(..) {
                    let resp = handle_command(
                        &mut guard,
                        &persist_state,
                        &script_cache_state,
                        &mut current_db,
                        &mut script_runtime,
                        &qcmd,
                        &qargs,
                    );
                    results.push(resp);
                }
                FastResponse::Value(RespValue::Array(results))
            }
        } else {
            let mut guard = state.lock().await;
            let resp = handle_command(
                &mut guard,
                &persist_state,
                &script_cache_state,
                &mut current_db,
                &mut script_runtime,
                cmd,
                &args[1..],
            );
            FastResponse::Value(resp)
        };
        if let Some(start) = timing {
            let elapsed_us = start.elapsed().as_micros();
            let arg_count = args.len().saturating_sub(1);
            eprintln!(
                "mini-redis: cmd={} args={} elapsed_us={}",
                cmd,
                arg_count,
                elapsed_us
            );
        }
        match resp {
            FastResponse::Value(value) => {
                write_value_buf(&mut writer, &value, &mut resp_buf).await?;
            }
            FastResponse::BlobArray(items) => {
                write_array_of_blobs_buf(&mut writer, &items, &mut resp_buf).await?;
            }
        }
        while let Ok(msg) = pub_rx.try_recv() {
            let _ = write_value_buf(&mut writer, &msg, &mut resp_buf).await;
        }
        writer.flush().await?;
        if cmd == "QUIT" {
            let _ = peer;
            return Ok(());
        }
    }
}

enum FastResponse {
    Value(RespValue),
    BlobArray(Vec<Arc<[u8]>>),
}

async fn handle_lrange_fast(
    state: &Arc<Mutex<ServerState>>,
    db_index: &mut usize,
    args: &[Arc<[u8]>],
) -> Result<Vec<Arc<[u8]>>, RespValue> {
    match (args.get(0), args.get(1), args.get(2)) {
        (Some(key), Some(start), Some(stop)) => {
            let start = parse_i64(start.as_ref()).unwrap_or(0);
            let stop = parse_i64(stop.as_ref()).unwrap_or(-1);
            let mut guard = state.lock().await;
            let db = &mut guard.dbs[*db_index];
            match db.list_range(key.as_ref(), start, stop) {
                Ok(items) => Ok(items),
                Err(_) => Err(RespValue::Error(
                    "WRONGTYPE Operation against a key holding the wrong kind of value".to_string(),
                )),
            }
        }
        _ => Err(RespValue::Error("ERR wrong number of arguments for 'LRANGE'".to_string())),
    }
}

async fn handle_subscribe(
    pubsub_state: &Arc<Mutex<PubSubState>>,
    sender: &Sender<RespValue>,
    channels: &[Arc<[u8]>],
) -> RespValue {
    if channels.is_empty() {
        return RespValue::Error("ERR wrong number of arguments for 'SUBSCRIBE'".to_string());
    }
    let mut guard = pubsub_state.lock().await;
    let mut count = 0;
    for channel in channels {
        let entry = guard.entry(channel.clone()).or_default();
        entry.push(sender.clone());
        count += 1;
    }
    let channel = channels[0].clone();
    RespValue::Array(vec![
        RespValue::Blob(b"subscribe".to_vec().into()),
        RespValue::Blob(channel.into()),
        RespValue::Integer(count),
    ])
}

async fn handle_publish(pubsub_state: &Arc<Mutex<PubSubState>>, args: &[Arc<[u8]>]) -> RespValue {
    if args.len() != 2 {
        return RespValue::Error("ERR wrong number of arguments for 'PUBLISH'".to_string());
    }
    let channel = args[0].clone();
    let message = args[1].clone();
    let mut receivers = Vec::new();
    {
        let guard = pubsub_state.lock().await;
        if let Some(list) = guard.get(&channel) {
            receivers.extend(list.iter().cloned());
        }
    }
    let mut delivered = 0;
    for tx in receivers {
        if tx
            .send(RespValue::Array(vec![
                RespValue::Blob(b"message".to_vec().into()),
                RespValue::Blob(channel.clone().into()),
                RespValue::Blob(message.clone().into()),
            ]))
            .await
            .is_ok()
        {
            delivered += 1;
        }
    }
    RespValue::Integer(delivered)
}

fn handle_command(
    state: &mut ServerState,
    persist_state: &Arc<Mutex<PersistState>>,
    script_cache_state: &Arc<Mutex<ScriptCacheState>>,
    db_index: &mut usize,
    script: &mut Option<ScriptRuntime>,
    cmd: &str,
    args: &[Arc<[u8]>],
) -> RespValue {
    macro_rules! log_cmd {
        ($state:expr, $db:expr, $cmd:expr, $args:expr) => {
            {
                let _ = &$state;
                let _ = async_std::task::block_on(async {
                    let persist_guard = persist_state.lock().await;
                    if let Some(p) = persist_guard.as_ref() {
                        if p.aof_enabled() {
                            let _ = p.log_command($db, $cmd, $args).await;
                        }
                    }
                });
            }
        };
    }
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
        "SELECT" => match args.get(0).and_then(|v| parse_usize(v.as_ref())) {
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
                match db.get_string(key.as_ref()) {
                    Ok(Some(v)) => RespValue::Blob(v.into()),
                    Ok(None) => RespValue::Null,
                    Err(_) => RespValue::Error("WRONGTYPE Operation against a key holding the wrong kind of value".to_string()),
                }
            }
            None => RespValue::Error("ERR wrong number of arguments for 'GET'".to_string()),
        },
        "SETNX" => match args.get(0).zip(args.get(1)) {
            Some((key, value)) => {
                let db = &mut state.dbs[*db_index];
                match db.set_nx(key.as_ref().to_vec(), value.as_ref().to_vec()) {
                    Ok(set) => {
                        if set {
                            log_cmd!(state, *db_index, cmd, args);
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
                let key = args[idx].as_ref().to_vec();
                let value = args[idx + 1].as_ref().to_vec();
                db.set_string(key, value, None);
                idx += 2;
            }
            log_cmd!(state, *db_index, cmd, args);
            RespValue::Simple("OK".to_string())
        }
        "MGET" => {
            if args.is_empty() {
                return RespValue::Error("ERR wrong number of arguments for 'MGET'".to_string());
            }
            let db = &mut state.dbs[*db_index];
            let mut out = Vec::with_capacity(args.len());
            for key in args {
                match db.get_string(key.as_ref()) {
                    Ok(Some(v)) => out.push(RespValue::Blob(v.into())),
                    Ok(None) => out.push(RespValue::Null),
                    Err(_) => return RespValue::Error("WRONGTYPE Operation against a key holding the wrong kind of value".to_string()),
                }
            }
            RespValue::Array(out)
        }
        "GETSET" => match args.get(0).zip(args.get(1)) {
            Some((key, value)) => {
                let db = &mut state.dbs[*db_index];
                let prev = match db.get_string(key.as_ref()) {
                    Ok(val) => val,
                    Err(_) => {
                        return RespValue::Error("WRONGTYPE Operation against a key holding the wrong kind of value".to_string())
                    }
                };
                db.set_string(key.as_ref().to_vec(), value.as_ref().to_vec(), None);
                log_cmd!(state, *db_index, cmd, args);
                match prev {
                    Some(v) => RespValue::Blob(v.into()),
                    None => RespValue::Null,
                }
            }
            None => RespValue::Error("ERR wrong number of arguments for 'GETSET'".to_string()),
        },
        "APPEND" => match args.get(0).zip(args.get(1)) {
            Some((key, value)) => {
                let db = &mut state.dbs[*db_index];
                match db.append(key.as_ref().to_vec(), value.as_ref()) {
                    Ok(len) => {
                        log_cmd!(state, *db_index, cmd, args);
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
                match db.incr_by(key.as_ref(), 1) {
                    Ok(val) => {
                        log_cmd!(state, *db_index, cmd, args);
                        RespValue::Integer(val)
                    }
                    Err(_) => RespValue::Error("ERR value is not an integer or out of range".to_string()),
                }
            }
            None => RespValue::Error("ERR wrong number of arguments for 'INCR'".to_string()),
        },
        "INCRBY" => match args.get(0).zip(args.get(1)) {
            Some((key, delta)) => {
                let delta = parse_i64(delta.as_ref()).unwrap_or(0);
                let db = &mut state.dbs[*db_index];
                match db.incr_by(key.as_ref(), delta) {
                    Ok(val) => {
                        log_cmd!(state, *db_index, cmd, args);
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
                match db.incr_by(key.as_ref(), -1) {
                    Ok(val) => {
                        log_cmd!(state, *db_index, cmd, args);
                        RespValue::Integer(val)
                    }
                    Err(_) => RespValue::Error("ERR value is not an integer or out of range".to_string()),
                }
            }
            None => RespValue::Error("ERR wrong number of arguments for 'DECR'".to_string()),
        },
        "DECRBY" => match args.get(0).zip(args.get(1)) {
            Some((key, delta)) => {
                let delta = parse_i64(delta.as_ref()).unwrap_or(0);
                let db = &mut state.dbs[*db_index];
                match db.incr_by(key.as_ref(), -delta) {
                    Ok(val) => {
                        log_cmd!(state, *db_index, cmd, args);
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
                match db.get_string(key.as_ref()) {
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
                match db.hash_set(key.as_ref(), field, value) {
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
            log_cmd!(state, *db_index, cmd, args);
            RespValue::Integer(added)
        }
        "HGET" => match (args.get(0), args.get(1)) {
            (Some(key), Some(field)) => {
                let db = &mut state.dbs[*db_index];
                match db.hash_get(key.as_ref(), field.as_ref()) {
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
            let db = &mut state.dbs[*db_index];
            match db.hash_del(key.as_ref(), &args[1..]) {
                Ok(removed) => {
                    log_cmd!(state, *db_index, cmd, args);
                    RespValue::Integer(removed)
                }
                Err(_) => RespValue::Error("WRONGTYPE Operation against a key holding the wrong kind of value".to_string()),
            }
        }
        "HGETALL" => match args.get(0) {
            Some(key) => {
                let db = &mut state.dbs[*db_index];
                match db.hash_getall(key.as_ref()) {
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
                match db.hash_len(key.as_ref()) {
                    Ok(len) => RespValue::Integer(len),
                    Err(_) => RespValue::Error("WRONGTYPE Operation against a key holding the wrong kind of value".to_string()),
                }
            }
            None => RespValue::Error("ERR wrong number of arguments for 'HLEN'".to_string()),
        },
        "HEXISTS" => match (args.get(0), args.get(1)) {
            (Some(key), Some(field)) => {
                let db = &mut state.dbs[*db_index];
                match db.hash_exists(key.as_ref(), field.as_ref()) {
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
            let db = &mut state.dbs[*db_index];
            match db.list_push(key.as_ref(), &args[1..], cmd == "LPUSH") {
                Ok(len) => {
                    log_cmd!(state, *db_index, cmd, args);
                    RespValue::Integer(len)
                }
                Err(_) => RespValue::Error("WRONGTYPE Operation against a key holding the wrong kind of value".to_string()),
            }
        }
        "LPOP" | "RPOP" => {
            if args.is_empty() || args.len() > 2 {
                return RespValue::Error(format!("ERR wrong number of arguments for '{}'", cmd));
            }
            let key = &args[0];
            let count = if args.len() == 2 {
                let n = parse_i64(args[1].as_ref()).unwrap_or(0);
                if n <= 0 {
                    return RespValue::Error("ERR value is not an integer or out of range".to_string());
                }
                Some(n as usize)
            } else {
                None
            };
            let db = &mut state.dbs[*db_index];
            if let Some(count) = count {
                let mut out = Vec::new();
                for _ in 0..count {
                    match db.list_pop(key.as_ref(), cmd == "LPOP") {
                        Ok(Some(v)) => out.push(RespValue::Blob(v)),
                        Ok(None) => break,
                        Err(_) => {
                            return RespValue::Error(
                                "WRONGTYPE Operation against a key holding the wrong kind of value"
                                    .to_string(),
                            )
                        }
                    }
                }
                if out.is_empty() {
                    RespValue::Null
                } else {
                    log_cmd!(state, *db_index, cmd, args);
                    RespValue::Array(out)
                }
            } else {
                match db.list_pop(key.as_ref(), cmd == "LPOP") {
                    Ok(Some(v)) => {
                        log_cmd!(state, *db_index, cmd, args);
                        RespValue::Blob(v)
                    }
                    Ok(None) => RespValue::Null,
                    Err(_) => RespValue::Error(
                        "WRONGTYPE Operation against a key holding the wrong kind of value".to_string(),
                    ),
                }
            }
        }
        "LRANGE" => match (args.get(0), args.get(1), args.get(2)) {
            (Some(key), Some(start), Some(stop)) => {
                let start = parse_i64(start.as_ref()).unwrap_or(0);
                let stop = parse_i64(stop.as_ref()).unwrap_or(-1);
                let db = &mut state.dbs[*db_index];
                match db.list_range(key.as_ref(), start, stop) {
                    Ok(items) => RespValue::Array(items.into_iter().map(RespValue::Blob).collect()),
                    Err(_) => RespValue::Error("WRONGTYPE Operation against a key holding the wrong kind of value".to_string()),
                }
            }
            _ => RespValue::Error("ERR wrong number of arguments for 'LRANGE'".to_string()),
        },
        "LLEN" => match args.get(0) {
            Some(key) => {
                let db = &mut state.dbs[*db_index];
                match db.list_len(key.as_ref()) {
                    Ok(len) => RespValue::Integer(len),
                    Err(_) => RespValue::Error("WRONGTYPE Operation against a key holding the wrong kind of value".to_string()),
                }
            }
            None => RespValue::Error("ERR wrong number of arguments for 'LLEN'".to_string()),
        },
        "LINDEX" => match (args.get(0), args.get(1)) {
            (Some(key), Some(index)) => {
                let idx = match parse_i64(index.as_ref()) {
                    Some(v) => v,
                    None => return RespValue::Error("ERR value is not an integer or out of range".to_string()),
                };
                let db = &mut state.dbs[*db_index];
                match db.list_index(key.as_ref(), idx) {
                    Ok(Some(v)) => RespValue::Blob(v),
                    Ok(None) => RespValue::Null,
                    Err(_) => RespValue::Error("WRONGTYPE Operation against a key holding the wrong kind of value".to_string()),
                }
            }
            _ => RespValue::Error("ERR wrong number of arguments for 'LINDEX'".to_string()),
        },
        "LSET" => match (args.get(0), args.get(1), args.get(2)) {
            (Some(key), Some(index), Some(value)) => {
                let idx = match parse_i64(index.as_ref()) {
                    Some(v) => v,
                    None => return RespValue::Error("ERR value is not an integer or out of range".to_string()),
                };
                let db = &mut state.dbs[*db_index];
                match db.list_set(key.as_ref(), idx, value.as_ref()) {
                    Ok(()) => {
                        log_cmd!(state, *db_index, cmd, args);
                        RespValue::Simple("OK".to_string())
                    }
                    Err(_) => RespValue::Error("WRONGTYPE Operation against a key holding the wrong kind of value".to_string()),
                }
            }
            _ => RespValue::Error("ERR wrong number of arguments for 'LSET'".to_string()),
        },
        "LINSERT" => match (args.get(0), args.get(1), args.get(2), args.get(3)) {
            (Some(key), Some(pos), Some(pivot), Some(value)) => {
                let before = match pos.as_ref().to_ascii_uppercase().as_slice() {
                    b"BEFORE" => true,
                    b"AFTER" => false,
                    _ => return RespValue::Error("ERR syntax error".to_string()),
                };
                let db = &mut state.dbs[*db_index];
                match db.list_insert(key.as_ref(), before, pivot.as_ref(), value.as_ref()) {
                    Ok(len) => {
                        if len > 0 {
                            log_cmd!(state, *db_index, cmd, args);
                        }
                        RespValue::Integer(len)
                    }
                    Err(_) => RespValue::Error("WRONGTYPE Operation against a key holding the wrong kind of value".to_string()),
                }
            }
            _ => RespValue::Error("ERR wrong number of arguments for 'LINSERT'".to_string()),
        },
        "LREM" => match (args.get(0), args.get(1), args.get(2)) {
            (Some(key), Some(count), Some(value)) => {
                let cnt = match parse_i64(count.as_ref()) {
                    Some(v) => v,
                    None => return RespValue::Error("ERR value is not an integer or out of range".to_string()),
                };
                let db = &mut state.dbs[*db_index];
                match db.list_rem(key.as_ref(), cnt, value.as_ref()) {
                    Ok(removed) => {
                        if removed > 0 {
                            log_cmd!(state, *db_index, cmd, args);
                        }
                        RespValue::Integer(removed)
                    }
                    Err(_) => RespValue::Error("WRONGTYPE Operation against a key holding the wrong kind of value".to_string()),
                }
            }
            _ => RespValue::Error("ERR wrong number of arguments for 'LREM'".to_string()),
        },
        "SADD" => {
            if args.len() < 2 {
                return RespValue::Error("ERR wrong number of arguments for 'SADD'".to_string());
            }
            let key = &args[0];
            let db = &mut state.dbs[*db_index];
            match db.set_add(key.as_ref(), &args[1..]) {
                Ok(added) => {
                    log_cmd!(state, *db_index, cmd, args);
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
            let db = &mut state.dbs[*db_index];
            match db.set_remove(key.as_ref(), &args[1..]) {
                Ok(removed) => {
                    log_cmd!(state, *db_index, cmd, args);
                    RespValue::Integer(removed)
                }
                Err(_) => RespValue::Error("WRONGTYPE Operation against a key holding the wrong kind of value".to_string()),
            }
        }
        "SMEMBERS" => match args.get(0) {
            Some(key) => {
                let db = &mut state.dbs[*db_index];
                match db.set_members(key.as_ref()) {
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
                match db.set_is_member(key.as_ref(), member.as_ref()) {
                    Ok(exists) => RespValue::Integer(if exists { 1 } else { 0 }),
                    Err(_) => RespValue::Error("WRONGTYPE Operation against a key holding the wrong kind of value".to_string()),
                }
            }
            _ => RespValue::Error("ERR wrong number of arguments for 'SISMEMBER'".to_string()),
        },
        "SCARD" => match args.get(0) {
            Some(key) => {
                let db = &mut state.dbs[*db_index];
                match db.set_card(key.as_ref()) {
                    Ok(len) => RespValue::Integer(len),
                    Err(_) => RespValue::Error("WRONGTYPE Operation against a key holding the wrong kind of value".to_string()),
                }
            }
            None => RespValue::Error("ERR wrong number of arguments for 'SCARD'".to_string()),
        },
        "SMOVE" => match (args.get(0), args.get(1), args.get(2)) {
            (Some(source), Some(dest), Some(member)) => {
                let db = &mut state.dbs[*db_index];
                match db.set_move(source.as_ref(), dest.as_ref(), member.as_ref()) {
                    Ok(moved) => {
                        if moved {
                            log_cmd!(state, *db_index, cmd, args);
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
                let score = parse_f64(args[idx].as_ref()).unwrap_or(0.0);
                let member = args[idx + 1].as_ref().to_vec();
                match db.zadd(key.as_ref(), score, member) {
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
            log_cmd!(state, *db_index, cmd, args);
            RespValue::Integer(added)
        }
        "ZRANGE" => match (args.get(0), args.get(1), args.get(2)) {
            (Some(key), Some(start), Some(stop)) => {
                let start = parse_i64(start.as_ref()).unwrap_or(0);
                let stop = parse_i64(stop.as_ref()).unwrap_or(-1);
                let db = &mut state.dbs[*db_index];
                match db.zrange(key.as_ref(), start, stop) {
                    Ok(items) => RespValue::Array(items.into_iter().map(|v| RespValue::Blob(v.into())).collect()),
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
            let members: Vec<Vec<u8>> = args[1..].iter().map(|m| m.as_ref().to_vec()).collect();
            let db = &mut state.dbs[*db_index];
            match db.zrem(key.as_ref(), &members) {
                Ok(removed) => {
                    if removed > 0 {
                            log_cmd!(state, *db_index, cmd, args);
                    }
                    RespValue::Integer(removed)
                }
                Err(_) => RespValue::Error("WRONGTYPE Operation against a key holding the wrong kind of value".to_string()),
            }
        }
        "ZCARD" => match args.get(0) {
            Some(key) => {
                let db = &mut state.dbs[*db_index];
                match db.zcard(key.as_ref()) {
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
            let id = match std::str::from_utf8(args[1].as_ref()) {
                Ok(v) => v,
                Err(_) => return RespValue::Error("ERR invalid stream ID".to_string()),
            };
            let mut fields = Vec::new();
            let mut idx = 2;
            while idx + 1 < args.len() {
                fields.push((args[idx].as_ref().to_vec(), args[idx + 1].as_ref().to_vec()));
                idx += 2;
            }
            let db = &mut state.dbs[*db_index];
            match db.stream_add(key.as_ref(), id, fields) {
                Ok(new_id) => {
                        log_cmd!(state, *db_index, cmd, args);
                    RespValue::Blob(new_id.into_bytes().into())
                }
                Err(_) => RespValue::Error("ERR invalid stream ID".to_string()),
            }
        }
        "XRANGE" => match (args.get(0), args.get(1), args.get(2)) {
            (Some(key), Some(start), Some(end)) => {
                let start = std::str::from_utf8(start.as_ref()).unwrap_or("-");
                let end = std::str::from_utf8(end.as_ref()).unwrap_or("+");
                let db = &mut state.dbs[*db_index];
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
                    Err(_) => RespValue::Error("WRONGTYPE Operation against a key holding the wrong kind of value".to_string()),
                }
            }
            _ => RespValue::Error("ERR wrong number of arguments for 'XRANGE'".to_string()),
        },
        "SET" => match parse_set_args(args) {
            Ok((key, value, expire_ms)) => {
                let db = &mut state.dbs[*db_index];
                let expire_at = expire_ms.map(|ms| now_ms().saturating_add(ms));
                db.set_string(key.as_ref().to_vec(), value.as_ref().to_vec(), expire_at);
                log_cmd!(state, *db_index, cmd, args);
                RespValue::Simple("OK".to_string())
            }
            Err(e) => RespValue::Error(e),
        },
        "DEL" => {
            let mut removed = 0;
            let db = &mut state.dbs[*db_index];
            for key in args {
                if db.remove(key.as_ref()) {
                    removed += 1;
                }
            }
            log_cmd!(state, *db_index, cmd, args);
            RespValue::Integer(removed)
        }
        "EXISTS" => {
            let mut count = 0;
            let db = &mut state.dbs[*db_index];
            for key in args {
                if db.exists(key.as_ref()) {
                    count += 1;
                }
            }
            RespValue::Integer(count)
        }
        "EXPIRE" => match (args.get(0), args.get(1)) {
            (Some(key), Some(sec)) => {
                let db = &mut state.dbs[*db_index];
                let ms = parse_u64(sec.as_ref()).unwrap_or(0).saturating_mul(1000);
                let ok = db.set_expire_ms(key.as_ref(), ms);
                if ok {
                    log_cmd!(state, *db_index, cmd, args);
                }
                RespValue::Integer(if ok { 1 } else { 0 })
            }
            _ => RespValue::Error("ERR wrong number of arguments for 'EXPIRE'".to_string()),
        },
        "PEXPIRE" => match (args.get(0), args.get(1)) {
            (Some(key), Some(ms)) => {
                let db = &mut state.dbs[*db_index];
                let ms = parse_u64(ms.as_ref()).unwrap_or(0);
                let ok = db.set_expire_ms(key.as_ref(), ms);
                if ok {
                    log_cmd!(state, *db_index, cmd, args);
                }
                RespValue::Integer(if ok { 1 } else { 0 })
            }
            _ => RespValue::Error("ERR wrong number of arguments for 'PEXPIRE'".to_string()),
        },
        "PERSIST" => match args.get(0) {
            Some(key) => {
                let db = &mut state.dbs[*db_index];
                let removed = db.persist(key.as_ref());
                if removed == 1 {
                    log_cmd!(state, *db_index, cmd, args);
                }
                RespValue::Integer(removed)
            }
            None => RespValue::Error("ERR wrong number of arguments for 'PERSIST'".to_string()),
        },
        "TTL" => match args.get(0) {
            Some(key) => {
                let db = &mut state.dbs[*db_index];
                match db.ttl_ms(key.as_ref()) {
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
                match db.ttl_ms(key.as_ref()) {
                    Some(ms) => RespValue::Integer(ms),
                    None => RespValue::Integer(-2),
                }
            }
            None => RespValue::Error("ERR wrong number of arguments for 'PTTL'".to_string()),
        },
        "TYPE" => match args.get(0) {
            Some(key) => {
                let db = &mut state.dbs[*db_index];
                match db.value_type(key.as_ref()) {
                    Some(t) => RespValue::Simple(t.to_string()),
                    None => RespValue::Simple("none".to_string()),
                }
            }
            None => RespValue::Error("ERR wrong number of arguments for 'TYPE'".to_string()),
        },
        "KEYS" => match args.get(0) {
            Some(pattern) => {
                let db = &mut state.dbs[*db_index];
                let keys = db.keys_matching(pattern.as_ref());
                RespValue::Array(keys.into_iter().map(|v| RespValue::Blob(v.into())).collect())
            }
            None => RespValue::Error("ERR wrong number of arguments for 'KEYS'".to_string()),
        },
        "SCAN" => match args.get(0) {
            Some(cursor) => {
                let cursor_str = match core::str::from_utf8(cursor.as_ref()) {
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
                    let opt = args[i].as_ref().to_ascii_uppercase();
                    if opt == b"MATCH" {
                        if i + 1 >= args.len() {
                            return RespValue::Error("ERR syntax error".to_string());
                        }
                        pattern = args[i + 1].as_ref().to_vec();
                        i += 2;
                        continue;
                    }
                    if opt == b"COUNT" {
                        if i + 1 >= args.len() {
                            return RespValue::Error("ERR syntax error".to_string());
                        }
                        let cnt_str = match core::str::from_utf8(args[i + 1].as_ref()) {
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
                out.push(RespValue::Blob(next_cursor.to_string().into_bytes().into()));
                out.push(RespValue::Array(batch.into_iter().map(|v| RespValue::Blob(v.into())).collect()));
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
            log_cmd!(state, *db_index, cmd, args);
            RespValue::Simple("OK".to_string())
        }
        "FLUSHALL" => {
            if !args.is_empty() {
                return RespValue::Error("ERR wrong number of arguments for 'FLUSHALL'".to_string());
            }
            for db in state.dbs.iter_mut() {
                db.flush();
            }
            log_cmd!(state, 0, cmd, args);
            RespValue::Simple("OK".to_string())
        }
        "INFO" => RespValue::Blob(b"mini-redis:1\r\n".to_vec().into()),
        "EVAL" => {
            if script.is_none() {
                *script = Some(ScriptRuntime::new(&state.script_runtime));
            }
            eval_script(
                state,
                persist_state,
                script_cache_state,
                db_index,
                script.as_mut().unwrap(),
                args,
                true,
            )
        }
        "EVALSHA" => {
            if script.is_none() {
                *script = Some(ScriptRuntime::new(&state.script_runtime));
            }
            eval_script_sha(
                state,
                persist_state,
                script_cache_state,
                db_index,
                script.as_mut().unwrap(),
                args,
            )
        }
        "SCRIPT" => handle_script_command(script_cache_state, args),
        "CONFIG" => {
            if args.len() >= 1 && to_upper_ascii(args[0].as_ref()) == "GET" {
                RespValue::Array(Vec::new())
            } else {
                RespValue::Error("ERR syntax error".to_string())
            }
        }
        "FUNCTION" => {
            if args.is_empty() {
                return RespValue::Error("ERR wrong number of arguments for 'FUNCTION'".to_string());
            }
            let sub = to_upper_ascii(args[0].as_ref());
            match sub.as_ref() {
                "LIST" => RespValue::Array(Vec::new()),
                "FLUSH" => {
                    let mut cache = async_std::task::block_on(script_cache_state.lock());
                    cache.clear();
                    RespValue::Simple("OK".to_string())
                }
                "LOAD" => {
                    if args.len() != 2 {
                        return RespValue::Error("ERR wrong number of arguments for 'FUNCTION LOAD'".to_string());
                    }
                    let script = match std::str::from_utf8(args[1].as_ref()) {
                        Ok(s) => s,
                        Err(_) => return RespValue::Error("ERR invalid function".to_string()),
                    };
                    let sha = sha1_hex(script.as_bytes());
                    let mut cache = async_std::task::block_on(script_cache_state.lock());
                    cache.insert(sha, script.to_string());
                    RespValue::Simple("OK".to_string())
                }
                _ => RespValue::Error("ERR unknown subcommand for FUNCTION".to_string()),
            }
        }
        "CLIENT" => {
            if args.len() >= 1 && to_upper_ascii(args[0].as_ref()) == "LIST" {
                RespValue::Blob(b"id=1 addr=127.0.0.1:0".to_vec().into())
            } else {
                RespValue::Error("ERR syntax error".to_string())
            }
        }
        "SLOWLOG" => {
            if args.len() >= 1 && to_upper_ascii(args[0].as_ref()) == "GET" {
                RespValue::Array(Vec::new())
            } else {
                RespValue::Error("ERR syntax error".to_string())
            }
        }
        "SAVE" => {
            match async_std::task::block_on(async {
                let persist_guard = persist_state.lock().await;
                if let Some(p) = persist_guard.as_ref() {
                    p.snapshot(&mut state.dbs).await.map(|_| ())
                }
                else {
                    Err(io::Error::new(io::ErrorKind::NotFound, "persistence not configured"))
                }
            }) {
                Ok(()) => RespValue::Simple("OK".to_string()),
                Err(err) if err.kind() == io::ErrorKind::NotFound => {
                    RespValue::Error("ERR persistence not configured".to_string())
                }
                Err(err) => RespValue::Error(format!("ERR persistence failed: {}", err)),
            }
        }
        "BGSAVE" => {
            match async_std::task::block_on(async {
                let persist_guard = persist_state.lock().await;
                if let Some(p) = persist_guard.as_ref() {
                    p.snapshot(&mut state.dbs).await.map(|_| ())
                }
                else {
                    Err(io::Error::new(io::ErrorKind::NotFound, "persistence not configured"))
                }
            }) {
                Ok(()) => RespValue::Simple("OK".to_string()),
                Err(err) if err.kind() == io::ErrorKind::NotFound => {
                    RespValue::Error("ERR persistence not configured".to_string())
                }
                Err(err) => RespValue::Error(format!("ERR persistence failed: {}", err)),
            }
        }
        "REPLICAOF" => RespValue::Error("ERR replication not implemented".to_string()),
        "QUIT" => RespValue::Simple("OK".to_string()),
        _ => RespValue::Error(format!("ERR unknown command '{}'", cmd)),
    }
}

fn eval_script(
    state: &mut ServerState,
    persist_state: &Arc<Mutex<PersistState>>,
    script_cache_state: &Arc<Mutex<ScriptCacheState>>,
    db_index: &mut usize,
    script_runtime: &mut ScriptRuntime,
    args: &[Arc<[u8]>],
    cache: bool,
) -> RespValue {
    if args.len() < 2 {
        return RespValue::Error("ERR wrong number of arguments for 'EVAL'".to_string());
    }
    let script = match std::str::from_utf8(args[0].as_ref()) {
        Ok(s) => s,
        Err(_) => return RespValue::Error("ERR invalid script".to_string()),
    };
    if cache {
        let sha = sha1_hex(script.as_bytes());
        let mut cache = async_std::task::block_on(script_cache_state.lock());
        cache.insert(sha, script.to_string());
    }
    eval_script_source(
        state,
        persist_state,
        script_cache_state,
        db_index,
        script_runtime,
        script,
        &args[1..],
    )
}

fn eval_script_source(
    state: &mut ServerState,
    persist_state: &Arc<Mutex<PersistState>>,
    script_cache_state: &Arc<Mutex<ScriptCacheState>>,
    db_index: &mut usize,
    script_runtime: &mut ScriptRuntime,
    script: &str,
    args: &[Arc<[u8]>],
) -> RespValue {
    let wrapped = wrap_eval_script(script);
    eval_wrapped_script(
        state,
        persist_state,
        script_cache_state,
        db_index,
        script_runtime,
        &wrapped,
        args,
    )
}

fn eval_wrapped_script(
    state: &mut ServerState,
    persist_state: &Arc<Mutex<PersistState>>,
    script_cache_state: &Arc<Mutex<ScriptCacheState>>,
    db_index: &mut usize,
    script_runtime: &mut ScriptRuntime,
    wrapped_script: &str,
    args: &[Arc<[u8]>],
) -> RespValue {
    let numkeys = match args.first().and_then(|n| parse_usize(n.as_ref())) {
        Some(n) => n,
        None => return RespValue::Error("ERR invalid number of keys".to_string()),
    };
    if args.len() < 1 + numkeys {
        return RespValue::Error("ERR invalid number of keys".to_string());
    }
    let keys = &args[1..1 + numkeys];
    let argv = &args[1 + numkeys..];
    script_runtime.maybe_reset();
    script_runtime.set_keys_argv(keys, argv);
    let mut exec = ScriptExec {
        state: state as *mut ServerState,
        db_index: db_index as *mut usize,
        persist_state: persist_state as *const Arc<Mutex<PersistState>>,
        script_cache_state: script_cache_state as *const Arc<Mutex<ScriptCacheState>>,
    };
    let ctx = &mut script_runtime.ctx;
    JS_SetContextOpaque(ctx, &mut exec as *mut ScriptExec as *mut c_void);
    let result = JS_Eval(ctx, wrapped_script, "<eval>", JS_EVAL_RETVAL);
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
    persist_state: &Arc<Mutex<PersistState>>,
    script_cache_state: &Arc<Mutex<ScriptCacheState>>,
    db_index: &mut usize,
    script_runtime: &mut ScriptRuntime,
    args: &[Arc<[u8]>],
) -> RespValue {
    if args.len() < 2 {
        return RespValue::Error("ERR wrong number of arguments for 'EVALSHA'".to_string());
    }
    let sha = match std::str::from_utf8(args[0].as_ref()) {
        Ok(s) => s,
        Err(_) => return RespValue::Error("ERR invalid script".to_string()),
    };
    let cache = async_std::task::block_on(script_cache_state.lock());
    let wrapped = match cache.get(sha) {
        Some(s) => wrap_eval_script(s),
        None => {
            return RespValue::Error("NOSCRIPT No matching script. Please use EVAL.".to_string())
        }
    };
    eval_wrapped_script(
        state,
        persist_state,
        script_cache_state,
        db_index,
        script_runtime,
        &wrapped,
        &args[1..],
    )
}

fn wrap_eval_script(script: &str) -> String {
    let mut wrapped = String::with_capacity(script.len() + 48);
    wrapped.push_str("function __redis_script__(){\n");
    wrapped.push_str(script);
    wrapped.push_str("\n}\n__redis_script__()");
    wrapped
}

fn handle_script_command(script_cache_state: &Arc<Mutex<ScriptCacheState>>, args: &[Arc<[u8]>]) -> RespValue {
    if args.len() < 1 {
        return RespValue::Error("ERR wrong number of arguments for 'SCRIPT'".to_string());
    }
    let sub = to_upper_ascii(args[0].as_ref());
    match sub.as_ref() {
        "LOAD" => {
            if args.len() != 2 {
                return RespValue::Error("ERR wrong number of arguments for 'SCRIPT LOAD'".to_string());
            }
            let script = match std::str::from_utf8(args[1].as_ref()) {
                Ok(s) => s,
                Err(_) => return RespValue::Error("ERR invalid script".to_string()),
            };
            let sha = sha1_hex(script.as_bytes());
            let mut cache = async_std::task::block_on(script_cache_state.lock());
            cache.insert(sha.clone(), script.to_string());
            RespValue::Blob(sha.into_bytes().into())
        }
        "EXISTS" => {
            if args.len() < 2 {
                return RespValue::Error("ERR wrong number of arguments for 'SCRIPT EXISTS'".to_string());
            }
            let cache = async_std::task::block_on(script_cache_state.lock());
            let mut out = Vec::with_capacity(args.len() - 1);
            for sha in &args[1..] {
                let s = match std::str::from_utf8(sha.as_ref()) {
                    Ok(v) => v,
                    Err(_) => "",
                };
                let exists = cache.contains_key(s);
                out.push(RespValue::Integer(if exists { 1 } else { 0 }));
            }
            RespValue::Array(out)
        }
        "FLUSH" => {
            let mut cache = async_std::task::block_on(script_cache_state.lock());
            cache.clear();
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
    mem_size: usize,
    reset_threshold_pct: u8,
    ctx: JSContextImpl,
    cfuncs: Vec<JSCFunctionDef>,
}

unsafe impl Send for ScriptRuntime {}

impl ScriptRuntime {
    fn new(config: &ScriptRuntimeConfig) -> Self {
        let mem_size = config.mem_size.max(1024);
        let (mem, ctx, cfuncs) = Self::build_ctx(mem_size);
        Self {
            mem,
            mem_size,
            reset_threshold_pct: config.reset_threshold_pct,
            ctx,
            cfuncs,
        }
    }

    fn maybe_reset(&mut self) {
        let (used, total) = self.ctx.memory_usage();
        let threshold = (self.reset_threshold_pct as usize).min(100).max(1);
        if total > 0 && used * 100 >= total * threshold {
            self.reset();
        }
    }

    fn reset(&mut self) {
        let (mem, ctx, cfuncs) = Self::build_ctx(self.mem_size);
        self.mem = mem;
        self.ctx = ctx;
        self.cfuncs = cfuncs;
    }

    fn build_ctx(mem_size: usize) -> (Vec<u8>, JSContextImpl, Vec<JSCFunctionDef>) {
        let mut mem = vec![0u8; mem_size];
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
        (mem, ctx, cfuncs)
    }

    fn set_keys_argv(&mut self, keys: &[Arc<[u8]>], argv: &[Arc<[u8]>]) {
        let global = JS_GetGlobalObject(&mut self.ctx);
        let keys_arr = JS_NewArray(&mut self.ctx, keys.len() as i32);
        for (idx, key) in keys.iter().enumerate() {
            let v = JS_NewStringLen(&mut self.ctx, key.as_ref());
            let _ = JS_SetPropertyUint32(&mut self.ctx, keys_arr, idx as u32, v);
        }
        let argv_arr = JS_NewArray(&mut self.ctx, argv.len() as i32);
        for (idx, arg) in argv.iter().enumerate() {
            let v = JS_NewStringLen(&mut self.ctx, arg.as_ref());
            let _ = JS_SetPropertyUint32(&mut self.ctx, argv_arr, idx as u32, v);
        }
        let _ = JS_SetPropertyStr(&mut self.ctx, global, "KEYS", keys_arr);
        let _ = JS_SetPropertyStr(&mut self.ctx, global, "ARGV", argv_arr);
    }
}

struct ScriptExec {
    state: *mut ServerState,
    db_index: *mut usize,
    persist_state: *const Arc<Mutex<PersistState>>,
    script_cache_state: *const Arc<Mutex<ScriptCacheState>>,
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
        let mut cmd_buf = JSCStringBuf { buf: [0u8; 5] };
        let cmd_bytes = JS_ToCString(ctx, *argv, &mut cmd_buf).as_bytes().to_vec();
        let cmd = match parse_command(&cmd_bytes) {
            Some(cmd) => cmd,
            None => {
                let msg = format!("ERR unknown command '{}'", String::from_utf8_lossy(&cmd_bytes));
                return JS_ThrowInternalError(ctx, &msg);
            }
        };
        let mut args: Vec<Arc<[u8]>> = Vec::with_capacity((argc - 1).max(0) as usize);
        for i in 1..argc {
            let val = *argv.add(i as usize);
            args.push(js_value_to_arc_bytes(ctx, val));
        }
        let state = &mut *exec.state;
        let persist_state = &*exec.persist_state;
        let script_cache_state = &*exec.script_cache_state;
        let db_index = &mut *exec.db_index;
        let mut script = None;
        let resp = handle_command(
            state,
            persist_state,
            script_cache_state,
            db_index,
            &mut script,
            &cmd,
            &args,
        );
        resp_to_js(ctx, resp, magic == 1)
    }
}

fn resp_to_js(ctx: &mut JSContextImpl, resp: RespValue, is_pcall: bool) -> JSValue {
    match resp {
        RespValue::Simple(s) => JS_NewString(ctx, &s),
        RespValue::Blob(b) => JS_NewStringLen(ctx, b.as_ref()),
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
        return RespValue::Blob(num.to_string().into_bytes().into());
    }
    if JS_IsString(ctx, val) != 0 {
        let s = js_value_to_string(ctx, val);
        return RespValue::Blob(s.into_bytes().into());
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
    RespValue::Blob(s.into_bytes().into())
}

fn js_value_to_string(ctx: &mut JSContextImpl, val: JSValue) -> String {
    let mut buf = JSCStringBuf { buf: [0u8; 5] };
    JS_ToCString(ctx, val, &mut buf).to_string()
}

fn js_value_to_arc_bytes(ctx: &mut JSContextImpl, val: JSValue) -> Arc<[u8]> {
    let mut buf = JSCStringBuf { buf: [0u8; 5] };
    Arc::from(JS_ToCString(ctx, val, &mut buf).as_bytes())
}

fn value_to_args(val: RespValue) -> Result<Vec<Arc<[u8]>>, String> {
    match val {
        RespValue::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                match item {
                    RespValue::Blob(b) => out.push(b),
                    RespValue::Simple(s) => out.push(Arc::from(s.into_bytes())),
                    RespValue::Integer(n) => out.push(Arc::from(n.to_string().into_bytes())),
                    _ => return Err("ERR invalid array item".to_string()),
                }
            }
            Ok(out)
        }
        RespValue::Simple(s) => Ok(vec![Arc::from(s.into_bytes())]),
        RespValue::Blob(b) => Ok(vec![b]),
        _ => Err("ERR invalid request".to_string()),
    }
}

fn parse_set_args(args: &[Arc<[u8]>]) -> Result<(Arc<[u8]>, Arc<[u8]>, Option<u64>), String> {
    if args.len() < 2 {
        return Err("ERR wrong number of arguments for 'SET'".to_string());
    }
    let key = args[0].clone();
    let value = args[1].clone();
    let mut expire_ms = None;
    let mut idx = 2;
    while idx < args.len() {
        let opt = to_upper_ascii(args[idx].as_ref());
        if opt == "EX" {
            idx += 1;
            let sec = args.get(idx).ok_or_else(|| "ERR syntax error".to_string())?;
            expire_ms = Some(parse_u64(sec.as_ref()).unwrap_or(0).saturating_mul(1000));
        } else if opt == "PX" {
            idx += 1;
            let ms = args.get(idx).ok_or_else(|| "ERR syntax error".to_string())?;
            expire_ms = Some(parse_u64(ms.as_ref()).unwrap_or(0));
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

fn parse_command(input: &[u8]) -> Option<&'static str> {
    const COMMANDS: &[&str] = &[
        "PING", "ECHO", "SELECT", "DBSIZE", "GET", "SET", "SETNX", "MSET", "MGET", "GETSET",
        "APPEND", "INCR", "INCRBY", "DECR", "DECRBY", "STRLEN", "HSET", "HGET", "HDEL", "HGETALL",
        "HLEN", "HEXISTS", "LPUSH", "RPUSH", "LPOP", "RPOP", "LRANGE", "LLEN", "LINDEX", "LSET",
        "LINSERT", "LREM", "SADD", "SREM", "SMEMBERS", "SISMEMBER", "SCARD", "SMOVE", "ZADD", "ZRANGE",
        "ZREM", "ZCARD", "XADD", "XRANGE", "DEL", "EXISTS", "EXPIRE", "PEXPIRE", "PERSIST", "TTL",
        "PTTL", "TYPE", "KEYS", "SCAN", "FLUSHDB", "FLUSHALL", "INFO", "EVAL", "EVALSHA", "SCRIPT",
        "CONFIG", "FUNCTION", "CLIENT", "SLOWLOG", "SAVE", "BGSAVE", "REPLICAOF", "QUIT", "MULTI",
        "EXEC", "DISCARD", "SUBSCRIBE", "PUBLISH",
    ];
    COMMANDS
        .iter()
        .copied()
        .find(|cmd| input.eq_ignore_ascii_case(cmd.as_bytes()))
}

fn to_upper_ascii(input: &[u8]) -> std::borrow::Cow<'_, str> {
    let mut has_lower = false;
    for b in input {
        if b.is_ascii_lowercase() {
            has_lower = true;
            break;
        }
    }
    if !has_lower {
        if let Ok(s) = core::str::from_utf8(input) {
            return std::borrow::Cow::Borrowed(s);
        }
    }
    let upper: String = input.iter().map(|b| b.to_ascii_uppercase() as char).collect();
    std::borrow::Cow::Owned(upper)
}

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
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
