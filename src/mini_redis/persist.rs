//! Persistence layer for mini-redis (snapshot + AOF).
#![allow(dead_code)]

use async_std::io;
use async_trait::async_trait;

use crate::mini_redis::store::{Db, Value};

#[async_trait]
pub trait Persist: Send + Sync {
    async fn load(&self, dbs: &mut [Db]) -> io::Result<()>;
    async fn log_command(&self, db: usize, cmd: &[Vec<u8>]) -> io::Result<()>;
    async fn snapshot(&self, dbs: &mut [Db]) -> io::Result<()>;
}

pub struct NoopPersist;

#[async_trait]
impl Persist for NoopPersist {
    async fn load(&self, _dbs: &mut [Db]) -> io::Result<()> {
        Ok(())
    }

    async fn log_command(&self, _db: usize, _cmd: &[Vec<u8>]) -> io::Result<()> {
        Ok(())
    }

    async fn snapshot(&self, _dbs: &mut [Db]) -> io::Result<()> {
        Ok(())
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
            "CREATE TABLE IF NOT EXISTS kv (db INTEGER, key BLOB, value BLOB, expires_at_ms INTEGER, PRIMARY KEY(db, key))",
            (),
        ).await.map_err(to_io)?;
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS aof_log (id INTEGER PRIMARY KEY AUTOINCREMENT, db INTEGER, cmd BLOB)",
            (),
        ).await.map_err(to_io)?;
        Ok(())
    }
}

#[cfg(feature = "mini-redis-libsql")]
#[async_trait]
impl Persist for LibsqlPersist {
    async fn load(&self, dbs: &mut [Db]) -> io::Result<()> {
        let mut rows = self.conn.query("SELECT db, key, value, expires_at_ms FROM kv", ()).await.map_err(to_io)?;
        while let Some(row) = rows.next().await.map_err(to_io)? {
            let db_idx: i64 = row.get(0).map_err(to_io)?;
            let key: Vec<u8> = row.get(1).map_err(to_io)?;
            let value: Vec<u8> = row.get(2).map_err(to_io)?;
            let exp: Option<i64> = row.get(3).map_err(to_io)?;
            if let Some(db) = dbs.get_mut(db_idx as usize) {
                db.set_with_expire_at(key, Value::String(value), exp.map(|v| v as u64));
            }
        }
        Ok(())
    }

    async fn log_command(&self, db: usize, cmd: &[Vec<u8>]) -> io::Result<()> {
        if !self.aof_enabled {
            return Ok(());
        }
        let encoded = encode_cmd(cmd);
        self.conn.execute("INSERT INTO aof_log (db, cmd) VALUES (?, ?)", (db as i64, encoded)).await.map_err(to_io)?;
        Ok(())
    }

    async fn snapshot(&self, dbs: &mut [Db]) -> io::Result<()> {
        self.conn.execute("DELETE FROM kv", ()).await.map_err(to_io)?;
        for (idx, db) in dbs.iter_mut().enumerate() {
            for (key, value, exp) in db.snapshot_items() {
                let Value::String(bytes) = value;
                let exp_val = exp.map(|v| v as i64);
                self.conn.execute(
                    "INSERT INTO kv (db, key, value, expires_at_ms) VALUES (?, ?, ?, ?)",
                    (idx as i64, key, bytes, exp_val),
                ).await.map_err(to_io)?;
            }
        }
        Ok(())
    }
}

#[cfg(feature = "mini-redis-libsql")]
fn encode_cmd(cmd: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"*");
    out.extend_from_slice(cmd.len().to_string().as_bytes());
    out.extend_from_slice(b"\r\n");
    for item in cmd {
        out.extend_from_slice(b"$");
        out.extend_from_slice(item.len().to_string().as_bytes());
        out.extend_from_slice(b"\r\n");
        out.extend_from_slice(item);
        out.extend_from_slice(b"\r\n");
    }
    out
}

#[cfg(feature = "mini-redis-libsql")]
fn to_io<E: std::fmt::Display>(err: E) -> io::Error {
    io::Error::new(io::ErrorKind::Other, err.to_string())
}
