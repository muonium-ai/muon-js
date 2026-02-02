//! Persistence layer for mini-redis (snapshot + AOF).
#![allow(dead_code)]

use async_std::io;

use crate::mini_redis::store::Db;

#[cfg(feature = "mini-redis-libsql")]
use crate::mini_redis::store::Value;

pub enum Persist {
    Noop,
    #[cfg(feature = "mini-redis-libsql")]
    Libsql(LibsqlPersist),
}

impl Persist {
    pub fn aof_enabled(&self) -> bool {
        match self {
            Persist::Noop => false,
            #[cfg(feature = "mini-redis-libsql")]
            Persist::Libsql(persist) => persist.aof_enabled,
        }
    }

    pub async fn load(&self, dbs: &mut [Db]) -> io::Result<()> {
        match self {
            Persist::Noop => Ok(()),
            #[cfg(feature = "mini-redis-libsql")]
            Persist::Libsql(persist) => persist.load(dbs).await,
        }
    }

    pub async fn log_command(&self, db: usize, cmd: &str, args: &[std::sync::Arc<[u8]>]) -> io::Result<()> {
        match self {
            Persist::Noop => Ok(()),
            #[cfg(feature = "mini-redis-libsql")]
            Persist::Libsql(persist) => persist.log_command(db, cmd, args).await,
        }
    }

    pub async fn snapshot(&self, dbs: &mut [Db]) -> io::Result<()> {
        match self {
            Persist::Noop => Ok(()),
            #[cfg(feature = "mini-redis-libsql")]
            Persist::Libsql(persist) => persist.snapshot(dbs).await,
        }
    }
}

#[cfg(feature = "mini-redis-libsql")]
pub struct LibsqlPersist {
    conn: libsql::Connection,
    aof_enabled: bool,
}

#[cfg(feature = "mini-redis-libsql")]
impl LibsqlPersist {
    pub async fn open(path: &str, aof_enabled: bool) -> io::Result<Self> {
        let db = libsql::Database::open(path).map_err(to_io)?;
        let conn = db.connect().map_err(to_io)?;
        let persist = Self { conn, aof_enabled };
        persist.init_schema().await?;
        Ok(persist)
    }

    async fn init_schema(&self) -> io::Result<()> {
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS kv (db INTEGER, key BLOB, type INTEGER, value BLOB, expires_at_ms INTEGER, PRIMARY KEY(db, key))",
            (),
        ).await.map_err(to_io)?;
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS list_items (db INTEGER, key BLOB, idx INTEGER, value BLOB, PRIMARY KEY(db, key, idx))",
            (),
        ).await.map_err(to_io)?;
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS set_items (db INTEGER, key BLOB, value BLOB, PRIMARY KEY(db, key, value))",
            (),
        ).await.map_err(to_io)?;
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS hash_items (db INTEGER, key BLOB, field BLOB, value BLOB, PRIMARY KEY(db, key, field))",
            (),
        ).await.map_err(to_io)?;
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS aof_log (id INTEGER PRIMARY KEY AUTOINCREMENT, db INTEGER, cmd BLOB)",
            (),
        ).await.map_err(to_io)?;
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS zset_items (db INTEGER, key BLOB, member BLOB, score REAL, PRIMARY KEY(db, key, member))",
            (),
        ).await.map_err(to_io)?;
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS stream_entries (db INTEGER, key BLOB, entry_id BLOB, field_idx INTEGER, field BLOB, value BLOB, PRIMARY KEY(db, key, entry_id, field_idx))",
            (),
        ).await.map_err(to_io)?;
        Ok(())
    }
}

#[cfg(feature = "mini-redis-libsql")]
impl LibsqlPersist {
    async fn load(&self, dbs: &mut [Db]) -> io::Result<()> {
        let mut rows = self.conn.query("SELECT db, key, type, value, expires_at_ms FROM kv", ()).await.map_err(to_io)?;
        while let Some(row) = rows.next().await.map_err(to_io)? {
            let db_idx: i64 = row.get(0).map_err(to_io)?;
            let key: Vec<u8> = row.get(1).map_err(to_io)?;
            let typ: i64 = row.get(2).map_err(to_io)?;
            let value: Option<Vec<u8>> = row.get(3).map_err(to_io)?;
            let exp: Option<i64> = row.get(4).map_err(to_io)?;
            if let Some(db) = dbs.get_mut(db_idx as usize) {
                match typ {
                    0 => {
                        db.set_with_expire_at(key, Value::String(value.unwrap_or_default()), exp.map(|v| v as u64));
                    }
                    1 => {
                        let list = load_list(&self.conn, db_idx as usize, &key).await?;
                        db.set_with_expire_at(key, Value::List(list), exp.map(|v| v as u64));
                    }
                    2 => {
                        let set = load_set(&self.conn, db_idx as usize, &key).await?;
                        db.set_with_expire_at(key, Value::Set(set), exp.map(|v| v as u64));
                    }
                    3 => {
                        let hash = load_hash(&self.conn, db_idx as usize, &key).await?;
                        db.set_with_expire_at(key, Value::Hash(hash), exp.map(|v| v as u64));
                    }
                    4 => {
                        let zset = load_zset(&self.conn, db_idx as usize, &key).await?;
                        db.set_with_expire_at(key, Value::ZSet(zset), exp.map(|v| v as u64));
                    }
                    5 => {
                        let stream = load_stream(&self.conn, db_idx as usize, &key).await?;
                        db.set_with_expire_at(key, Value::Stream(stream), exp.map(|v| v as u64));
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    async fn log_command(&self, db: usize, cmd: &str, args: &[std::sync::Arc<[u8]>]) -> io::Result<()> {
            if !self.aof_enabled {
                return Ok(());
            }
            let encoded = encode_cmd(cmd, args);
            self.conn.execute("INSERT INTO aof_log (db, cmd) VALUES (?, ?)", (db as i64, encoded)).await.map_err(to_io)?;
            Ok(())
        }

    async fn snapshot(&self, dbs: &mut [Db]) -> io::Result<()> {
        self.conn.execute("DELETE FROM kv", ()).await.map_err(to_io)?;
        self.conn.execute("DELETE FROM list_items", ()).await.map_err(to_io)?;
        self.conn.execute("DELETE FROM set_items", ()).await.map_err(to_io)?;
        self.conn.execute("DELETE FROM hash_items", ()).await.map_err(to_io)?;
        self.conn.execute("DELETE FROM zset_items", ()).await.map_err(to_io)?;
        self.conn.execute("DELETE FROM stream_entries", ()).await.map_err(to_io)?;
        for (idx, db) in dbs.iter_mut().enumerate() {
            for (key, value, exp) in db.snapshot_items() {
                let exp_val = exp.map(|v| v as i64);
                match value {
                    Value::String(bytes) => {
                        self.conn.execute(
                            "INSERT INTO kv (db, key, type, value, expires_at_ms) VALUES (?, ?, ?, ?, ?)",
                            (idx as i64, key, 0i64, bytes, exp_val),
                        ).await.map_err(to_io)?;
                    }
                    Value::List(items) => {
                        self.conn.execute(
                            "INSERT INTO kv (db, key, type, value, expires_at_ms) VALUES (?, ?, ?, ?, ?)",
                            (idx as i64, key.clone(), 1i64, Vec::<u8>::new(), exp_val),
                        ).await.map_err(to_io)?;
                        for (i, item) in items.iter().enumerate() {
                            self.conn.execute(
                                "INSERT INTO list_items (db, key, idx, value) VALUES (?, ?, ?, ?)",
                                (idx as i64, key.clone(), i as i64, item.clone()),
                            ).await.map_err(to_io)?;
                        }
                    }
                    Value::Set(items) => {
                        self.conn.execute(
                            "INSERT INTO kv (db, key, type, value, expires_at_ms) VALUES (?, ?, ?, ?, ?)",
                            (idx as i64, key.clone(), 2i64, Vec::<u8>::new(), exp_val),
                        ).await.map_err(to_io)?;
                        for item in items.iter() {
                            self.conn.execute(
                                "INSERT INTO set_items (db, key, value) VALUES (?, ?, ?)",
                                (idx as i64, key.clone(), item.clone()),
                            ).await.map_err(to_io)?;
                        }
                    }
                    Value::Hash(items) => {
                        self.conn.execute(
                            "INSERT INTO kv (db, key, type, value, expires_at_ms) VALUES (?, ?, ?, ?, ?)",
                            (idx as i64, key.clone(), 3i64, Vec::<u8>::new(), exp_val),
                        ).await.map_err(to_io)?;
                        for (field, val) in items.iter() {
                            self.conn.execute(
                                "INSERT INTO hash_items (db, key, field, value) VALUES (?, ?, ?, ?)",
                                (idx as i64, key.clone(), field.clone(), val.clone()),
                            ).await.map_err(to_io)?;
                        }
                    }
                    Value::ZSet(items) => {
                        self.conn.execute(
                            "INSERT INTO kv (db, key, type, value, expires_at_ms) VALUES (?, ?, ?, ?, ?)",
                            (idx as i64, key.clone(), 4i64, Vec::<u8>::new(), exp_val),
                        ).await.map_err(to_io)?;
                        for (member, score) in items.iter() {
                            self.conn.execute(
                                "INSERT INTO zset_items (db, key, member, score) VALUES (?, ?, ?, ?)",
                                (idx as i64, key.clone(), member.clone(), *score),
                            ).await.map_err(to_io)?;
                        }
                    }
                    Value::Stream(items) => {
                        self.conn.execute(
                            "INSERT INTO kv (db, key, type, value, expires_at_ms) VALUES (?, ?, ?, ?, ?)",
                            (idx as i64, key.clone(), 5i64, Vec::<u8>::new(), exp_val),
                        ).await.map_err(to_io)?;
                        for (entry_id, fields) in items.iter() {
                            for (field_idx, (field, val)) in fields.iter().enumerate() {
                                self.conn.execute(
                                    "INSERT INTO stream_entries (db, key, entry_id, field_idx, field, value) VALUES (?, ?, ?, ?, ?, ?)",
                                    (idx as i64, key.clone(), entry_id.as_bytes().to_vec(), field_idx as i64, field.clone(), val.clone()),
                                ).await.map_err(to_io)?;
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(feature = "mini-redis-libsql")]
fn encode_cmd(cmd: &str, args: &[std::sync::Arc<[u8]>]) -> Vec<u8> {
    let count = args.len() + 1;
    let capacity = 16 + cmd.len() + args.iter().map(|a| 16 + a.len()).sum::<usize>();
    let mut out = Vec::with_capacity(capacity);
    out.extend_from_slice(b"*");
    out.extend_from_slice(count.to_string().as_bytes());
    out.extend_from_slice(b"\r\n$");
    out.extend_from_slice(cmd.len().to_string().as_bytes());
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(cmd.as_bytes());
    out.extend_from_slice(b"\r\n");
    for item in args {
        out.extend_from_slice(b"$");
        out.extend_from_slice(item.len().to_string().as_bytes());
        out.extend_from_slice(b"\r\n");
        out.extend_from_slice(item.as_ref());
        out.extend_from_slice(b"\r\n");
    }
    out
}

#[cfg(feature = "mini-redis-libsql")]
fn to_io<E: std::fmt::Display>(err: E) -> io::Error {
    io::Error::new(io::ErrorKind::Other, err.to_string())
}

#[cfg(feature = "mini-redis-libsql")]
async fn load_list(conn: &libsql::Connection, db: usize, key: &[u8]) -> io::Result<std::collections::VecDeque<Vec<u8>>> {
    let mut rows = conn
        .query("SELECT idx, value FROM list_items WHERE db = ? AND key = ? ORDER BY idx ASC", (db as i64, key.to_vec()))
        .await
        .map_err(to_io)?;
    let mut out = std::collections::VecDeque::new();
    while let Some(row) = rows.next().await.map_err(to_io)? {
        let value: Vec<u8> = row.get(1).map_err(to_io)?;
        out.push_back(value);
    }
    Ok(out)
}

#[cfg(feature = "mini-redis-libsql")]
async fn load_set(conn: &libsql::Connection, db: usize, key: &[u8]) -> io::Result<std::collections::HashSet<Vec<u8>>> {
    let mut rows = conn
        .query("SELECT value FROM set_items WHERE db = ? AND key = ?", (db as i64, key.to_vec()))
        .await
        .map_err(to_io)?;
    let mut out = std::collections::HashSet::new();
    while let Some(row) = rows.next().await.map_err(to_io)? {
        let value: Vec<u8> = row.get(0).map_err(to_io)?;
        out.insert(value);
    }
    Ok(out)
}

#[cfg(feature = "mini-redis-libsql")]
async fn load_hash(conn: &libsql::Connection, db: usize, key: &[u8]) -> io::Result<std::collections::HashMap<Vec<u8>, Vec<u8>>> {
    let mut rows = conn
        .query("SELECT field, value FROM hash_items WHERE db = ? AND key = ?", (db as i64, key.to_vec()))
        .await
        .map_err(to_io)?;
    let mut out = std::collections::HashMap::new();
    while let Some(row) = rows.next().await.map_err(to_io)? {
        let field: Vec<u8> = row.get(0).map_err(to_io)?;
        let value: Vec<u8> = row.get(1).map_err(to_io)?;
        out.insert(field, value);
    }
    Ok(out)
}

#[cfg(feature = "mini-redis-libsql")]
async fn load_zset(conn: &libsql::Connection, db: usize, key: &[u8]) -> io::Result<Vec<(Vec<u8>, f64)>> {
    let mut rows = conn
        .query("SELECT member, score FROM zset_items WHERE db = ? AND key = ? ORDER BY score ASC", (db as i64, key.to_vec()))
        .await
        .map_err(to_io)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.map_err(to_io)? {
        let member: Vec<u8> = row.get(0).map_err(to_io)?;
        let score: f64 = row.get(1).map_err(to_io)?;
        out.push((member, score));
    }
    Ok(out)
}

#[cfg(feature = "mini-redis-libsql")]
async fn load_stream(conn: &libsql::Connection, db: usize, key: &[u8]) -> io::Result<Vec<(String, Vec<(Vec<u8>, Vec<u8>)>)>> {
    let mut rows = conn
        .query(
            "SELECT entry_id, field_idx, field, value FROM stream_entries WHERE db = ? AND key = ? ORDER BY entry_id ASC, field_idx ASC",
            (db as i64, key.to_vec()),
        )
        .await
        .map_err(to_io)?;
    let mut out: Vec<(String, Vec<(Vec<u8>, Vec<u8>)>)> = Vec::new();
    let mut current_id: Option<String> = None;
    let mut current_fields: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    while let Some(row) = rows.next().await.map_err(to_io)? {
        let entry_id_bytes: Vec<u8> = row.get(0).map_err(to_io)?;
        let field: Vec<u8> = row.get(2).map_err(to_io)?;
        let value: Vec<u8> = row.get(3).map_err(to_io)?;
        let entry_id = String::from_utf8(entry_id_bytes).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).to_string());
        if current_id.as_deref() != Some(entry_id.as_str()) {
            if let Some(prev_id) = current_id.take() {
                out.push((prev_id, current_fields));
                current_fields = Vec::new();
            }
            current_id = Some(entry_id);
        }
        current_fields.push((field, value));
    }
    if let Some(prev_id) = current_id.take() {
        out.push((prev_id, current_fields));
    }
    Ok(out)
}
