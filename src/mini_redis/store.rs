//! In-memory multi-DB store with TTL support.

use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug)]
pub enum Value {
    String(Vec<u8>),
    List(Vec<Vec<u8>>),
    Set(HashSet<Vec<u8>>),
    Hash(HashMap<Vec<u8>, Vec<u8>>),
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

    pub fn set_with_expire_at(&mut self, key: Vec<u8>, value: Value, expire_at_ms: Option<u64>) {
        self.set(key, value, expire_at_ms);
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

    pub fn snapshot_items(&mut self) -> Vec<(Vec<u8>, Value, Option<u64>)> {
        self.purge_expired_all();
        let mut out = Vec::with_capacity(self.data.len());
        for (k, v) in self.data.iter() {
            let exp = self.expires.get(k).copied();
            out.push((k.clone(), v.clone(), exp));
        }
        out
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
            Some(Value::List(_)) => Some("list"),
            Some(Value::Set(_)) => Some("set"),
            Some(Value::Hash(_)) => Some("hash"),
            None => None,
        }
    }

    pub fn list_push(&mut self, key: &[u8], values: &[Vec<u8>], left: bool) -> Result<i64, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        let entry = self.data.entry(key.to_vec()).or_insert_with(|| Value::List(Vec::new()));
        match entry {
            Value::List(list) => {
                if left {
                    for value in values {
                        list.insert(0, value.clone());
                    }
                } else {
                    for value in values {
                        list.push(value.clone());
                    }
                }
                Ok(list.len() as i64)
            }
            _ => Err(()),
        }
    }

    pub fn list_pop(&mut self, key: &[u8], left: bool) -> Result<Option<Vec<u8>>, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        let entry = self.data.get_mut(key);
        match entry {
            Some(Value::List(list)) => {
                let out = if list.is_empty() {
                    None
                } else if left {
                    Some(list.remove(0))
                } else {
                    list.pop()
                };
                if list.is_empty() {
                    self.data.remove(key);
                    self.expires.remove(key);
                }
                Ok(out)
            }
            Some(_) => Err(()),
            None => Ok(None),
        }
    }

    pub fn list_range(&mut self, key: &[u8], start: i64, stop: i64) -> Result<Vec<Vec<u8>>, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        match self.data.get(key) {
            Some(Value::List(list)) => {
                let len = list.len() as i64;
                if len == 0 {
                    return Ok(Vec::new());
                }
                let mut s = if start < 0 { len + start } else { start };
                let mut e = if stop < 0 { len + stop } else { stop };
                if s < 0 {
                    s = 0;
                }
                if e < 0 {
                    return Ok(Vec::new());
                }
                if s >= len {
                    return Ok(Vec::new());
                }
                if e >= len {
                    e = len - 1;
                }
                if s > e {
                    return Ok(Vec::new());
                }
                let mut out = Vec::with_capacity((e - s + 1) as usize);
                for i in s..=e {
                    out.push(list[i as usize].clone());
                }
                Ok(out)
            }
            Some(_) => Err(()),
            None => Ok(Vec::new()),
        }
    }

    pub fn list_len(&mut self, key: &[u8]) -> Result<i64, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        match self.data.get(key) {
            Some(Value::List(list)) => Ok(list.len() as i64),
            Some(_) => Err(()),
            None => Ok(0),
        }
    }

    pub fn get_string(&mut self, key: &[u8]) -> Result<Option<Vec<u8>>, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        match self.data.get(key) {
            Some(Value::String(val)) => Ok(Some(val.clone())),
            Some(_) => Err(()),
            None => Ok(None),
        }
    }

    pub fn set_string(&mut self, key: Vec<u8>, value: Vec<u8>, expire_at_ms: Option<u64>) {
        self.set(key, Value::String(value), expire_at_ms);
    }

    pub fn set_nx(&mut self, key: Vec<u8>, value: Vec<u8>) -> Result<bool, ()> {
        if self.is_expired(&key) {
            self.remove(&key);
        }
        if self.data.contains_key(&key) {
            return Ok(false);
        }
        self.data.insert(key, Value::String(value));
        Ok(true)
    }

    pub fn append(&mut self, key: Vec<u8>, value: &[u8]) -> Result<i64, ()> {
        if self.is_expired(&key) {
            self.remove(&key);
        }
        match self.data.get_mut(&key) {
            Some(Value::String(buf)) => {
                buf.extend_from_slice(value);
                Ok(buf.len() as i64)
            }
            Some(_) => Err(()),
            None => {
                self.data.insert(key.clone(), Value::String(value.to_vec()));
                Ok(value.len() as i64)
            }
        }
    }

    pub fn incr_by(&mut self, key: Vec<u8>, delta: i64) -> Result<i64, ()> {
        if self.is_expired(&key) {
            self.remove(&key);
        }
        match self.data.get_mut(&key) {
            Some(Value::String(buf)) => {
                let s = std::str::from_utf8(buf).map_err(|_| ())?;
                let n: i64 = s.parse().map_err(|_| ())?;
                let next = n.saturating_add(delta);
                *buf = next.to_string().into_bytes();
                Ok(next)
            }
            Some(_) => Err(()),
            None => {
                let next = delta;
                self.data.insert(key, Value::String(next.to_string().into_bytes()));
                Ok(next)
            }
        }
    }

    pub fn hash_set(&mut self, key: &[u8], field: Vec<u8>, value: Vec<u8>) -> Result<bool, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        let entry = self.data.entry(key.to_vec()).or_insert_with(|| Value::Hash(HashMap::new()));
        match entry {
            Value::Hash(map) => Ok(map.insert(field, value).is_none()),
            _ => Err(()),
        }
    }

    pub fn hash_get(&mut self, key: &[u8], field: &[u8]) -> Result<Option<Vec<u8>>, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        match self.data.get(key) {
            Some(Value::Hash(map)) => Ok(map.get(field).cloned()),
            Some(_) => Err(()),
            None => Ok(None),
        }
    }

    pub fn hash_del(&mut self, key: &[u8], fields: &[Vec<u8>]) -> Result<i64, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        match self.data.get_mut(key) {
            Some(Value::Hash(map)) => {
                let mut removed = 0;
                for field in fields {
                    if map.remove(field).is_some() {
                        removed += 1;
                    }
                }
                if map.is_empty() {
                    self.data.remove(key);
                    self.expires.remove(key);
                }
                Ok(removed)
            }
            Some(_) => Err(()),
            None => Ok(0),
        }
    }

    pub fn hash_len(&mut self, key: &[u8]) -> Result<i64, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        match self.data.get(key) {
            Some(Value::Hash(map)) => Ok(map.len() as i64),
            Some(_) => Err(()),
            None => Ok(0),
        }
    }

    pub fn hash_exists(&mut self, key: &[u8], field: &[u8]) -> Result<bool, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        match self.data.get(key) {
            Some(Value::Hash(map)) => Ok(map.contains_key(field)),
            Some(_) => Err(()),
            None => Ok(false),
        }
    }

    pub fn hash_getall(&mut self, key: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        match self.data.get(key) {
            Some(Value::Hash(map)) => {
                let mut out = Vec::with_capacity(map.len() * 2);
                for (field, value) in map.iter() {
                    out.push((field.clone(), value.clone()));
                }
                Ok(out)
            }
            Some(_) => Err(()),
            None => Ok(Vec::new()),
        }
    }

    pub fn set_add(&mut self, key: &[u8], members: &[Vec<u8>]) -> Result<i64, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        let entry = self.data.entry(key.to_vec()).or_insert_with(|| Value::Set(HashSet::new()));
        match entry {
            Value::Set(set) => {
                let mut added = 0;
                for member in members {
                    if set.insert(member.clone()) {
                        added += 1;
                    }
                }
                Ok(added)
            }
            _ => Err(()),
        }
    }

    pub fn set_remove(&mut self, key: &[u8], members: &[Vec<u8>]) -> Result<i64, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        match self.data.get_mut(key) {
            Some(Value::Set(set)) => {
                let mut removed = 0;
                for member in members {
                    if set.remove(member) {
                        removed += 1;
                    }
                }
                if set.is_empty() {
                    self.data.remove(key);
                    self.expires.remove(key);
                }
                Ok(removed)
            }
            Some(_) => Err(()),
            None => Ok(0),
        }
    }

    pub fn set_members(&mut self, key: &[u8]) -> Result<Vec<Vec<u8>>, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        match self.data.get(key) {
            Some(Value::Set(set)) => Ok(set.iter().cloned().collect()),
            Some(_) => Err(()),
            None => Ok(Vec::new()),
        }
    }

    pub fn set_is_member(&mut self, key: &[u8], member: &[u8]) -> Result<bool, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        match self.data.get(key) {
            Some(Value::Set(set)) => Ok(set.contains(member)),
            Some(_) => Err(()),
            None => Ok(false),
        }
    }

    pub fn set_card(&mut self, key: &[u8]) -> Result<i64, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        match self.data.get(key) {
            Some(Value::Set(set)) => Ok(set.len() as i64),
            Some(_) => Err(()),
            None => Ok(0),
        }
    }

    pub fn set_move(&mut self, source: &[u8], dest: &[u8], member: &[u8]) -> Result<bool, ()> {
        if self.is_expired(source) {
            self.remove(source);
        }
        if self.is_expired(dest) {
            self.remove(dest);
        }
        let remove_result = match self.data.get_mut(source) {
            Some(Value::Set(set)) => set.remove(member),
            Some(_) => return Err(()),
            None => return Ok(false),
        };
        if !remove_result {
            return Ok(false);
        }
        if let Some(Value::Set(set)) = self.data.get(source) {
            if set.is_empty() {
                self.data.remove(source);
                self.expires.remove(source);
            }
        }
        let entry = self.data.entry(dest.to_vec()).or_insert_with(|| Value::Set(HashSet::new()));
        match entry {
            Value::Set(set) => {
                set.insert(member.to_vec());
                Ok(true)
            }
            _ => Err(()),
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
