//! RESP3 server and command dispatcher.

use async_std::io::{self, BufReader};
use async_std::net::{TcpListener, TcpStream};
use async_std::sync::{Arc, Mutex};
use async_std::task;

use crate::mini_redis::resp::{read_value, write_value, RespValue};
use crate::mini_redis::persist::Persist;
use crate::mini_redis::store::{Db, Value};

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
}

impl ServerState {
    fn new(dbs: Vec<Db>, persist: Option<Persist>) -> Self {
        Self { dbs, persist }
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
        let resp = {
            let mut guard = state.lock().await;
            handle_command(&mut guard, &mut current_db, &cmd, &args[1..]).await
        };
        write_value(&mut &stream, &resp).await?;
        if cmd == "QUIT" {
            let _ = peer;
            return Ok(());
        }
    }
}

async fn handle_command(state: &mut ServerState, db_index: &mut usize, cmd: &str, args: &[Vec<u8>]) -> RespValue {
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
                match db.get(key) {
                    Some(Value::String(v)) => RespValue::Blob(v),
                    Some(_) => RespValue::Error("WRONGTYPE Operation against a key holding the wrong kind of value".to_string()),
                    None => RespValue::Null,
                }
            }
            None => RespValue::Error("ERR wrong number of arguments for 'GET'".to_string()),
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
        "SET" => match parse_set_args(args) {
            Ok((key, value, expire_ms)) => {
                let db = &mut state.dbs[*db_index];
                let expire_at = expire_ms.map(|ms| now_ms().saturating_add(ms));
                db.set(key, Value::String(value), expire_at);
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
        "INFO" => RespValue::Blob(b"mini-redis:1\r\n".to_vec()),
        "EVAL" | "EVALSHA" | "SCRIPT" => RespValue::Error("ERR scripting not implemented".to_string()),
        "QUIT" => RespValue::Simple("OK".to_string()),
        _ => RespValue::Error(format!("ERR unknown command '{}'", cmd)),
    }
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
