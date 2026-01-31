//! In-memory multi-DB store with TTL support.

use std::collections::HashMap;

#[derive(Clone, Debug)]
pub enum Value {
    String(Vec<u8>),
}

#[derive(Default)]
pub struct Db {
    data: HashMap<Vec<u8>, Value>,
    expires: HashMap<Vec<u8>, u64>,
}

impl Db {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&mut self, key: &[u8]) -> Option<Value> {
        if self.is_expired(key) {
            self.remove(key);
            return None;
        }
        self.data.get(key).cloned()
    }

    pub fn set(&mut self, key: Vec<u8>, value: Value, expire_at_ms: Option<u64>) {
        if let Some(ts) = expire_at_ms {
            self.expires.insert(key.clone(), ts);
        } else {
            self.expires.remove(&key);
        }
        self.data.insert(key, value);
    }

    pub fn remove(&mut self, key: &[u8]) -> bool {
        let existed = self.data.remove(key).is_some();
        self.expires.remove(key);
        existed
    }

    pub fn exists(&mut self, key: &[u8]) -> bool {
        if self.is_expired(key) {
            self.remove(key);
            return false;
        }
        self.data.contains_key(key)
    }

    pub fn ttl_ms(&mut self, key: &[u8]) -> Option<i64> {
        if !self.data.contains_key(key) {
            return None;
        }
        if let Some(&ts) = self.expires.get(key) {
            let now = now_ms();
            if ts <= now {
                self.remove(key);
                return None;
            }
            return Some((ts - now) as i64);
        }
        Some(-1)
    }

    pub fn set_expire_ms(&mut self, key: &[u8], ttl_ms: u64) -> bool {
        if !self.data.contains_key(key) {
            return false;
        }
        self.expires.insert(key.to_vec(), now_ms().saturating_add(ttl_ms));
        true
    }

    pub fn purge_expired_all(&mut self) {
        let now = now_ms();
        let expired: Vec<Vec<u8>> = self.expires
            .iter()
            .filter_map(|(k, &ts)| if ts <= now { Some(k.clone()) } else { None })
            .collect();
        for key in expired {
            self.remove(&key);
        }
    }

    pub fn len(&mut self) -> usize {
        self.purge_expired_all();
        self.data.len()
    }

    pub fn value_type(&mut self, key: &[u8]) -> Option<&'static str> {
        if self.is_expired(key) {
            self.remove(key);
            return None;
        }
        match self.data.get(key) {
            Some(Value::String(_)) => Some("string"),
            None => None,
        }
    }

    fn is_expired(&self, key: &[u8]) -> bool {
        if let Some(&ts) = self.expires.get(key) {
            return ts <= now_ms();
        }
        false
    }
}

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}
