//! In-memory multi-DB store with TTL support and internal key-sharding.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

const NUM_SHARDS: usize = 64;

#[derive(Clone, Debug)]
pub enum Value {
    String(Arc<[u8]>),
    List(VecDeque<Arc<[u8]>>),
    Set(HashSet<Arc<[u8]>>),
    Hash(HashMap<Arc<[u8]>, Arc<[u8]>>),
    ZSet(HashMap<Vec<u8>, f64>),
    Stream(Vec<(String, Vec<(Vec<u8>, Vec<u8>)>)>),
}

/// Per-shard storage bucket. Holds the data and expiry maps for a subset of keys.
#[derive(Default)]
struct Shard {
    data: HashMap<Vec<u8>, Value>,
    expires: HashMap<Vec<u8>, u64>,
}

/// Thread-safe, internally-sharded key-value store.
///
/// All public methods take `&self`; interior mutability is provided by per-shard
/// `std::sync::Mutex` locks. This allows multiple threads/tasks to operate on
/// different keys concurrently without an external Mutex.
pub struct Db {
    shards: Vec<StdMutex<Shard>>,
}

impl Default for Db {
    fn default() -> Self {
        Self::new()
    }
}

impl Db {
    pub fn new() -> Self {
        let shards = (0..NUM_SHARDS)
            .map(|_| StdMutex::new(Shard::default()))
            .collect();
        Self { shards }
    }

    /// FNV-1a hash to select a shard bucket.
    #[inline]
    fn shard_index(key: &[u8]) -> usize {
        let mut h: u64 = 0xcbf29ce484222325;
        for &b in key {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        h as usize % NUM_SHARDS
    }

    #[inline]
    fn shard(&self, key: &[u8]) -> std::sync::MutexGuard<'_, Shard> {
        self.shards[Self::shard_index(key)].lock().unwrap()
    }
}

impl Shard {
    fn set(&mut self, key: Vec<u8>, value: Value, expire_at_ms: Option<u64>) {
        if let Some(ts) = expire_at_ms {
            self.expires.insert(key.clone(), ts);
        } else {
            self.expires.remove(&key);
        }
        self.data.insert(key, value);
    }

    fn set_with_expire_at(&mut self, key: Vec<u8>, value: Value, expire_at_ms: Option<u64>) {
        self.set(key, value, expire_at_ms);
    }

    fn remove(&mut self, key: &[u8]) -> bool {
        let existed = self.data.remove(key).is_some();
        self.expires.remove(key);
        existed
    }

    fn exists(&mut self, key: &[u8]) -> bool {
        if self.is_expired(key) {
            self.remove(key);
            return false;
        }
        self.data.contains_key(key)
    }

    fn ttl_ms(&mut self, key: &[u8]) -> Option<i64> {
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

    fn set_expire_ms(&mut self, key: &[u8], ttl_ms: u64) -> bool {
        if !self.data.contains_key(key) {
            return false;
        }
        self.expires.insert(key.to_vec(), now_ms().saturating_add(ttl_ms));
        true
    }

    fn persist(&mut self, key: &[u8]) -> i64 {
        if self.is_expired(key) {
            self.remove(key);
            return 0;
        }
        if self.data.contains_key(key) {
            if self.expires.remove(key).is_some() {
                return 1;
            }
            return 0;
        }
        0
    }

    fn purge_expired_all(&mut self) {
        let now = now_ms();
        let expired: Vec<Vec<u8>> = self.expires
            .iter()
            .filter_map(|(k, &ts)| if ts <= now { Some(k.clone()) } else { None })
            .collect();
        for key in expired {
            self.remove(&key);
        }
    }

    fn snapshot_items(&mut self) -> Vec<(Vec<u8>, Value, Option<u64>)> {
        self.purge_expired_all();
        let mut out = Vec::with_capacity(self.data.len());
        for (k, v) in self.data.iter() {
            let exp = self.expires.get(k).copied();
            out.push((k.clone(), v.clone(), exp));
        }
        out
    }

    fn len(&mut self) -> usize {
        self.purge_expired_all();
        self.data.len()
    }

    fn keys(&mut self) -> Vec<Vec<u8>> {
        self.purge_expired_all();
        self.data.keys().cloned().collect()
    }

    fn keys_matching(&mut self, pattern: &[u8]) -> Vec<Vec<u8>> {
        self.purge_expired_all();
        let mut out = Vec::new();
        for key in self.data.keys() {
            if Self::glob_match(key, pattern) {
                out.push(key.clone());
            }
        }
        out.sort();
        out
    }

    fn flush(&mut self) -> usize {
        let count = self.data.len();
        self.data.clear();
        self.expires.clear();
        count
    }

    fn value_type(&mut self, key: &[u8]) -> Option<&'static str> {
        if self.is_expired(key) {
            self.remove(key);
            return None;
        }
        match self.data.get(key) {
            Some(Value::String(_)) => Some("string"),
            Some(Value::List(_)) => Some("list"),
            Some(Value::Set(_)) => Some("set"),
            Some(Value::Hash(_)) => Some("hash"),
            Some(Value::ZSet(_)) => Some("zset"),
            Some(Value::Stream(_)) => Some("stream"),
            None => None,
        }
    }

    fn glob_class_match(pattern: &[u8], mut i: usize, ch: u8) -> Option<(bool, usize)> {
        let mut neg = false;
        let mut matched = false;
        if i >= pattern.len() {
            return None;
        }
        if pattern[i] == b'^' || pattern[i] == b'!' {
            neg = true;
            i += 1;
        }
        let start = i;
        while i < pattern.len() {
            let mut c = pattern[i];
            if c == b']' && i > start {
                let res = if neg { !matched } else { matched };
                return Some((res, i + 1));
            }
            if c == b'\\' && i + 1 < pattern.len() {
                i += 1;
                c = pattern[i];
            }
            if i + 2 < pattern.len() && pattern[i + 1] == b'-' && pattern[i + 2] != b']' {
                let mut end = pattern[i + 2];
                if end == b'\\' && i + 3 < pattern.len() {
                    end = pattern[i + 3];
                }
                if c <= ch && ch <= end {
                    matched = true;
                }
                i += 3;
                continue;
            }
            if c == ch {
                matched = true;
            }
            i += 1;
        }
        None
    }

    fn glob_match(text: &[u8], pattern: &[u8]) -> bool {
        let mut ti = 0usize;
        let mut pi = 0usize;
        let mut star_pi: Option<usize> = None;
        let mut star_ti = 0usize;

        while ti <= text.len() {
            if pi < pattern.len() {
                match pattern[pi] {
                    b'*' => {
                        star_pi = Some(pi);
                        pi += 1;
                        star_ti = ti;
                        continue;
                    }
                    b'?' => {
                        if ti >= text.len() {
                            return false;
                        }
                        ti += 1;
                        pi += 1;
                        continue;
                    }
                    b'[' => {
                        if ti >= text.len() {
                            return false;
                        }
                        match Self::glob_class_match(pattern, pi + 1, text[ti]) {
                            Some((ok, next_pi)) => {
                                if ok {
                                    ti += 1;
                                    pi = next_pi;
                                    continue;
                                }
                            }
                            None => {
                                if text[ti] == b'[' {
                                    ti += 1;
                                    pi += 1;
                                    continue;
                                }
                            }
                        }
                    }
                    b'\\' => {
                        if pi + 1 < pattern.len() {
                            pi += 1;
                        }
                        if ti < text.len() && pattern[pi] == text[ti] {
                            ti += 1;
                            pi += 1;
                            continue;
                        }
                    }
                    c => {
                        if ti < text.len() && c == text[ti] {
                            ti += 1;
                            pi += 1;
                            continue;
                        }
                    }
                }
            } else if ti == text.len() {
                return true;
            }

            if let Some(sp) = star_pi {
                pi = sp + 1;
                star_ti += 1;
                ti = star_ti;
                continue;
            }
            return false;
        }
        while pi < pattern.len() && pattern[pi] == b'*' {
            pi += 1;
        }
        pi == pattern.len() && ti == text.len()
    }

    fn list_push(&mut self, key: &[u8], values: &[Arc<[u8]>], left: bool) -> Result<i64, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        if let Some(entry) = self.data.get_mut(key) {
            return match entry {
                Value::List(list) => {
                    if left {
                        for value in values {
                            list.push_front(value.clone());
                        }
                    } else {
                        for value in values {
                            list.push_back(value.clone());
                        }
                    }
                    Ok(list.len() as i64)
                }
                _ => Err(()),
            };
        }
        let mut list = VecDeque::new();
        if left {
            for value in values {
                list.push_front(value.clone());
            }
        } else {
            for value in values {
                list.push_back(value.clone());
            }
        }
        let len = list.len() as i64;
        self.data.insert(key.to_vec(), Value::List(list));
        Ok(len)
    }

    fn list_pop(&mut self, key: &[u8], left: bool) -> Result<Option<Arc<[u8]>>, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        let entry = self.data.get_mut(key);
        match entry {
            Some(Value::List(list)) => {
                let out = if list.is_empty() {
                    None
                } else if left {
                    list.pop_front()
                } else {
                    list.pop_back()
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

    fn list_range(&mut self, key: &[u8], start: i64, stop: i64) -> Result<Vec<Arc<[u8]>>, ()> {
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
                let start_idx = s as usize;
                let count = (e - s + 1) as usize;
                let mut out = Vec::with_capacity(count);
                let (front, back) = list.as_slices();
                if start_idx < front.len() {
                    let end = (start_idx + count).min(front.len());
                    for item in &front[start_idx..end] {
                        out.push(item.clone());
                    }
                    if out.len() < count {
                        let remaining = count - out.len();
                        let end_back = remaining.min(back.len());
                        for item in &back[..end_back] {
                            out.push(item.clone());
                        }
                    }
                } else {
                    let back_start = start_idx - front.len();
                    let end = (back_start + count).min(back.len());
                    for item in &back[back_start..end] {
                        out.push(item.clone());
                    }
                }
                Ok(out)
            }
            Some(_) => Err(()),
            None => Ok(Vec::new()),
        }
    }

    fn list_len(&mut self, key: &[u8]) -> Result<i64, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        match self.data.get(key) {
            Some(Value::List(list)) => Ok(list.len() as i64),
            Some(_) => Err(()),
            None => Ok(0),
        }
    }

    fn list_index(&mut self, key: &[u8], index: i64) -> Result<Option<Arc<[u8]>>, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        match self.data.get(key) {
            Some(Value::List(list)) => {
                let len = list.len() as i64;
                let idx = if index < 0 { len + index } else { index };
                if idx < 0 || idx >= len {
                    return Ok(None);
                }
                Ok(list.get(idx as usize).cloned())
            }
            Some(_) => Err(()),
            None => Ok(None),
        }
    }

    fn list_set(&mut self, key: &[u8], index: i64, value: &[u8]) -> Result<(), ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        match self.data.get_mut(key) {
            Some(Value::List(list)) => {
                let len = list.len() as i64;
                let idx = if index < 0 { len + index } else { index };
                if idx < 0 || idx >= len {
                    return Err(());
                }
                if let Some(slot) = list.get_mut(idx as usize) {
                    *slot = Arc::from(value.to_vec());
                }
                Ok(())
            }
            Some(_) => Err(()),
            None => Err(()),
        }
    }

    fn list_insert(&mut self, key: &[u8], before: bool, pivot: &[u8], value: &[u8]) -> Result<i64, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        match self.data.get_mut(key) {
            Some(Value::List(list)) => {
                let pos = list.iter().position(|v| v.as_ref() == pivot);
                match pos {
                    Some(idx) => {
                        let insert_at = if before { idx } else { idx + 1 };
                        list.insert(insert_at, Arc::from(value.to_vec()));
                        Ok(list.len() as i64)
                    }
                    None => Ok(-1),
                }
            }
            Some(_) => Err(()),
            None => Ok(0),
        }
    }

    fn list_rem(&mut self, key: &[u8], count: i64, value: &[u8]) -> Result<i64, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        match self.data.get_mut(key) {
            Some(Value::List(list)) => {
                let mut removed = 0i64;
                if count == 0 {
                    list.retain(|v| {
                        if v.as_ref() == value {
                            removed += 1;
                            false
                        } else {
                            true
                        }
                    });
                } else if count > 0 {
                    let mut i = 0usize;
                    while i < list.len() && removed < count {
                        if list.get(i).map(|v| v.as_ref() == value).unwrap_or(false) {
                            list.remove(i);
                            removed += 1;
                        } else {
                            i += 1;
                        }
                    }
                } else {
                    let mut i = list.len();
                    while i > 0 && removed < (-count) {
                        i -= 1;
                        if list.get(i).map(|v| v.as_ref() == value).unwrap_or(false) {
                            list.remove(i);
                            removed += 1;
                        }
                    }
                }
                if list.is_empty() {
                    self.data.remove(key);
                    self.expires.remove(key);
                }
                Ok(removed)
            }
            Some(_) => Err(()),
            None => Ok(0),
        }
    }

    /// LPUSHX / RPUSHX: push only if the list already exists.
    fn list_push_x(&mut self, key: &[u8], values: &[Arc<[u8]>], left: bool) -> Result<i64, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        match self.data.get_mut(key) {
            Some(Value::List(list)) => {
                if left {
                    for value in values {
                        list.push_front(value.clone());
                    }
                } else {
                    for value in values {
                        list.push_back(value.clone());
                    }
                }
                Ok(list.len() as i64)
            }
            Some(_) => Err(()),
            None => Ok(0), // key doesn't exist → return 0 per Redis spec
        }
    }

    /// LTRIM: trim list to [start..=stop] range
    fn list_trim(&mut self, key: &[u8], start: i64, stop: i64) -> Result<(), ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        match self.data.get_mut(key) {
            Some(Value::List(list)) => {
                let len = list.len() as i64;
                let s = if start < 0 { (len + start).max(0) } else { start.min(len) } as usize;
                let e = if stop < 0 { (len + stop).max(-1) } else { stop.min(len - 1) } as usize;
                if s > e || s >= list.len() {
                    list.clear();
                } else {
                    // Drain from back first, then front, to keep indices valid
                    list.truncate(e + 1);
                    list.drain(..s);
                }
                if list.is_empty() {
                    self.data.remove(key);
                    self.expires.remove(key);
                }
                Ok(())
            }
            Some(_) => Err(()),
            None => Ok(()), // non-existent key → treat as empty list, no-op
        }
    }

    /// HINCRBY: increment hash field by delta
    fn hash_incr_by(&mut self, key: &[u8], field: &[u8], delta: i64) -> Result<i64, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        if let Some(entry) = self.data.get_mut(key) {
            return match entry {
                Value::Hash(map) => {
                    let field_key: Arc<[u8]> = Arc::from(field);
                    let current = match map.get(&field_key) {
                        Some(val) => {
                            let s = std::str::from_utf8(val.as_ref()).map_err(|_| ())?;
                            s.parse::<i64>().map_err(|_| ())?
                        }
                        None => 0,
                    };
                    let new_val = current.wrapping_add(delta);
                    map.insert(field_key, Arc::from(new_val.to_string().into_bytes()));
                    Ok(new_val)
                }
                _ => Err(()),
            };
        }
        let field_key: Arc<[u8]> = Arc::from(field);
        let new_val = delta;
        let mut map = HashMap::new();
        map.insert(field_key, Arc::from(new_val.to_string().into_bytes()));
        self.data.insert(key.to_vec(), Value::Hash(map));
        Ok(new_val)
    }

    /// HSETNX: set hash field only if it doesn't already exist
    fn hash_set_nx(&mut self, key: &[u8], field: Arc<[u8]>, value: Arc<[u8]>) -> Result<bool, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        if let Some(entry) = self.data.get_mut(key) {
            return match entry {
                Value::Hash(map) => {
                    use std::collections::hash_map::Entry;
                    match map.entry(field) {
                        Entry::Occupied(_) => Ok(false),
                        Entry::Vacant(e) => {
                            e.insert(value);
                            Ok(true)
                        }
                    }
                }
                _ => Err(()),
            };
        }
        let mut map = HashMap::new();
        map.insert(field, value);
        self.data.insert(key.to_vec(), Value::Hash(map));
        Ok(true)
    }

    /// SUNION: return the union of multiple sets
    fn set_union(&mut self, keys: &[&[u8]]) -> Result<Vec<Arc<[u8]>>, ()> {
        let mut result: HashSet<Arc<[u8]>> = HashSet::new();
        for &key in keys {
            if self.is_expired(key) {
                self.remove(key);
            }
            match self.data.get(key) {
                Some(Value::Set(set)) => {
                    for member in set {
                        result.insert(member.clone());
                    }
                }
                Some(_) => return Err(()),
                None => {} // non-existent key = empty set
            }
        }
        Ok(result.into_iter().collect())
    }

    /// SINTER: return the intersection of multiple sets
    fn set_inter(&mut self, keys: &[&[u8]]) -> Result<Vec<Arc<[u8]>>, ()> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        // Clean up expired keys
        for &key in keys {
            if self.is_expired(key) {
                self.remove(key);
            }
        }
        // Find the smallest set (optimization) and intersect
        let first_key = keys[0];
        let first_set = match self.data.get(first_key) {
            Some(Value::Set(set)) => set,
            Some(_) => return Err(()),
            None => return Ok(Vec::new()), // empty intersection
        };
        let mut result: Vec<Arc<[u8]>> = Vec::new();
        'outer: for member in first_set.iter() {
            for &key in &keys[1..] {
                match self.data.get(key) {
                    Some(Value::Set(set)) => {
                        if !set.contains(member.as_ref()) {
                            continue 'outer;
                        }
                    }
                    Some(_) => return Err(()),
                    None => return Ok(Vec::new()), // empty intersection
                }
            }
            result.push(member.clone());
        }
        Ok(result)
    }

    /// XLEN: return the number of entries in a stream
    fn stream_len(&mut self, key: &[u8]) -> Result<i64, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        match self.data.get(key) {
            Some(Value::Stream(items)) => Ok(items.len() as i64),
            Some(_) => Err(()),
            None => Ok(0),
        }
    }

    /// XREVRANGE: return stream entries in reverse order
    fn stream_rev_range(&mut self, key: &[u8], start: &str, end: &str) -> Result<Vec<(String, Vec<(Vec<u8>, Vec<u8>)>)>, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        match self.data.get(key) {
            Some(Value::Stream(items)) => {
                // For XREVRANGE, start is the higher ID and end is the lower ID
                // "+" means max, "-" means min
                if start == "+" && end == "-" {
                    let mut reversed = items.clone();
                    reversed.reverse();
                    return Ok(reversed);
                }
                Ok(Vec::new())
            }
            Some(_) => Err(()),
            None => Ok(Vec::new()),
        }
    }

    /// XDEL: remove entries by ID
    fn stream_del(&mut self, key: &[u8], ids: &[&str]) -> Result<i64, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        match self.data.get_mut(key) {
            Some(Value::Stream(items)) => {
                let before = items.len();
                items.retain(|(id, _)| !ids.contains(&id.as_str()));
                Ok((before - items.len()) as i64)
            }
            Some(_) => Err(()),
            None => Ok(0),
        }
    }

    fn get_string(&mut self, key: &[u8]) -> Result<Option<Arc<[u8]>>, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        match self.data.get(key) {
            Some(Value::String(val)) => Ok(Some(val.clone())),
            Some(_) => Err(()),
            None => Ok(None),
        }
    }

    fn set_string(&mut self, key: Vec<u8>, value: Arc<[u8]>, expire_at_ms: Option<u64>) {
        self.set(key, Value::String(value), expire_at_ms);
    }

    /// SET from borrowed slices — avoids caller needing pre-allocated Vec/Arc.
    fn set_string_from_slices(&mut self, key: &[u8], value: &[u8], expire_at_ms: Option<u64>) {
        self.set(key.to_vec(), Value::String(Arc::from(value)), expire_at_ms);
    }

    fn set_nx(&mut self, key: Vec<u8>, value: Arc<[u8]>) -> Result<bool, ()> {
        if self.is_expired(&key) {
            self.remove(&key);
        }
        if self.data.contains_key(&key) {
            return Ok(false);
        }
        self.data.insert(key, Value::String(value));
        Ok(true)
    }

    fn append(&mut self, key: Vec<u8>, value: &[u8]) -> Result<i64, ()> {
        if self.is_expired(&key) {
            self.remove(&key);
        }
        match self.data.get_mut(&key) {
            Some(Value::String(buf)) => {
                let mut v = Vec::with_capacity(buf.len() + value.len());
                v.extend_from_slice(buf);
                v.extend_from_slice(value);
                let len = v.len();
                *buf = Arc::from(v);
                Ok(len as i64)
            }
            Some(_) => Err(()),
            None => {
                self.data.insert(key.clone(), Value::String(Arc::from(value)));
                Ok(value.len() as i64)
            }
        }
    }

    fn incr_by(&mut self, key: &[u8], delta: i64) -> Result<i64, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        match self.data.get_mut(key) {
            Some(Value::String(buf)) => {
                let n = parse_i64_bytes(buf)?;
                let next = n.saturating_add(delta);
                let mut out = Vec::with_capacity(20);
                write_i64_bytes(&mut out, next);
                *buf = Arc::from(out);
                Ok(next)
            }
            Some(_) => Err(()),
            None => {
                let next = delta;
                let mut out = Vec::with_capacity(20);
                write_i64_bytes(&mut out, next);
                self.data.insert(key.to_vec(), Value::String(Arc::from(out)));
                Ok(next)
            }
        }
    }

    fn hash_set(&mut self, key: &[u8], field: Arc<[u8]>, value: Arc<[u8]>) -> Result<bool, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        if let Some(entry) = self.data.get_mut(key) {
            return match entry {
                Value::Hash(map) => Ok(map.insert(field, value).is_none()),
                _ => Err(()),
            };
        }
        let mut map = HashMap::new();
        map.insert(field, value);
        self.data.insert(key.to_vec(), Value::Hash(map));
        Ok(true)
    }

    /// Like hash_set but takes borrowed slices, avoiding Arc allocation overhead.
    fn hash_set_bytes(&mut self, key: &[u8], field: &[u8], value: &[u8]) -> Result<bool, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        if let Some(entry) = self.data.get_mut(key) {
            return match entry {
                Value::Hash(map) => Ok(map.insert(Arc::from(field), Arc::from(value)).is_none()),
                _ => Err(()),
            };
        }
        let mut map = HashMap::new();
        map.insert(Arc::from(field), Arc::from(value));
        self.data.insert(key.to_vec(), Value::Hash(map));
        Ok(true)
    }

    fn hash_get(&mut self, key: &[u8], field: &[u8]) -> Result<Option<Arc<[u8]>>, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        match self.data.get(key) {
            Some(Value::Hash(map)) => Ok(map.get(field).cloned()),
            Some(_) => Err(()),
            None => Ok(None),
        }
    }

    fn hash_del(&mut self, key: &[u8], fields: &[Arc<[u8]>]) -> Result<i64, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        match self.data.get_mut(key) {
            Some(Value::Hash(map)) => {
                let mut removed = 0;
                for field in fields {
                    if map.remove(field.as_ref()).is_some() {
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

    fn hash_len(&mut self, key: &[u8]) -> Result<i64, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        match self.data.get(key) {
            Some(Value::Hash(map)) => Ok(map.len() as i64),
            Some(_) => Err(()),
            None => Ok(0),
        }
    }

    fn hash_exists(&mut self, key: &[u8], field: &[u8]) -> Result<bool, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        match self.data.get(key) {
            Some(Value::Hash(map)) => Ok(map.contains_key(field)),
            Some(_) => Err(()),
            None => Ok(false),
        }
    }

    fn hash_getall(&mut self, key: &[u8]) -> Result<Vec<(Arc<[u8]>, Arc<[u8]>)>, ()> {
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

    fn set_add(&mut self, key: &[u8], members: &[Arc<[u8]>]) -> Result<i64, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        if let Some(entry) = self.data.get_mut(key) {
            return match entry {
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
            };
        }
        let mut set = HashSet::new();
        let mut added = 0;
        for member in members {
            if set.insert(member.clone()) {
                added += 1;
            }
        }
        self.data.insert(key.to_vec(), Value::Set(set));
        Ok(added)
    }

    /// Single-member SADD: takes borrowed bytes, only creates Arc when inserting.
    fn set_add_single_bytes(&mut self, key: &[u8], member: &[u8]) -> Result<i64, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        if let Some(entry) = self.data.get_mut(key) {
            return match entry {
                Value::Set(set) => {
                    if set.contains(member) {
                        Ok(0)
                    } else {
                        set.insert(Arc::from(member));
                        Ok(1)
                    }
                }
                _ => Err(()),
            };
        }
        let mut set = HashSet::new();
        set.insert(Arc::from(member));
        self.data.insert(key.to_vec(), Value::Set(set));
        Ok(1)
    }

    fn set_remove(&mut self, key: &[u8], members: &[Arc<[u8]>]) -> Result<i64, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        match self.data.get_mut(key) {
            Some(Value::Set(set)) => {
                let mut removed = 0;
                for member in members {
                    if set.remove(member.as_ref()) {
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

    fn set_members(&mut self, key: &[u8]) -> Result<Vec<Arc<[u8]>>, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        match self.data.get(key) {
            Some(Value::Set(set)) => Ok(set.iter().cloned().collect()),
            Some(_) => Err(()),
            None => Ok(Vec::new()),
        }
    }

    fn set_is_member(&mut self, key: &[u8], member: &[u8]) -> Result<bool, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        match self.data.get(key) {
            Some(Value::Set(set)) => Ok(set.contains(member)),
            Some(_) => Err(()),
            None => Ok(false),
        }
    }

    fn set_card(&mut self, key: &[u8]) -> Result<i64, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        match self.data.get(key) {
            Some(Value::Set(set)) => Ok(set.len() as i64),
            Some(_) => Err(()),
            None => Ok(0),
        }
    }

    fn set_move(&mut self, source: &[u8], dest: &[u8], member: &[u8]) -> Result<bool, ()> {
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
        if let Some(entry) = self.data.get_mut(dest) {
            return match entry {
                Value::Set(set) => {
                    set.insert(Arc::from(member));
                    Ok(true)
                }
                _ => Err(()),
            };
        }
        let mut set = HashSet::new();
        set.insert(Arc::from(member));
        self.data.insert(dest.to_vec(), Value::Set(set));
        Ok(true)
    }

    fn zadd(&mut self, key: &[u8], score: f64, member: Vec<u8>) -> Result<bool, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        if let Some(entry) = self.data.get_mut(key) {
            return match entry {
                Value::ZSet(items) => Ok(items.insert(member, score).is_none()),
                _ => Err(()),
            };
        }
        let mut items = HashMap::new();
        items.insert(member, score);
        self.data.insert(key.to_vec(), Value::ZSet(items));
        Ok(true)
    }

    fn zrem(&mut self, key: &[u8], members: &[Vec<u8>]) -> Result<i64, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        match self.data.get_mut(key) {
            Some(Value::ZSet(items)) => {
                let mut removed = 0i64;
                for member in members {
                    if items.remove(member).is_some() {
                        removed += 1;
                    }
                }
                if items.is_empty() {
                    self.data.remove(key);
                    self.expires.remove(key);
                }
                Ok(removed)
            }
            Some(_) => Err(()),
            None => Ok(0),
        }
    }

    fn zcard(&mut self, key: &[u8]) -> Result<i64, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        match self.data.get(key) {
            Some(Value::ZSet(items)) => Ok(items.len() as i64),
            Some(_) => Err(()),
            None => Ok(0),
        }
    }

    fn zrange(&mut self, key: &[u8], start: i64, stop: i64) -> Result<Vec<Vec<u8>>, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        match self.data.get(key) {
            Some(Value::ZSet(items)) => {
                if items.is_empty() {
                    return Ok(Vec::new());
                }
                let mut sorted: Vec<(Vec<u8>, f64)> = items
                    .iter()
                    .map(|(member, score)| (member.clone(), *score))
                    .collect();
                sorted.sort_by(|a, b| {
                    a.1.partial_cmp(&b.1)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then_with(|| a.0.cmp(&b.0))
                });
                let len = sorted.len() as i64;
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
                    out.push(sorted[i as usize].0.clone());
                }
                Ok(out)
            }
            Some(_) => Err(()),
            None => Ok(Vec::new()),
        }
    }

    fn stream_add(&mut self, key: &[u8], id: &str, fields: Vec<(Vec<u8>, Vec<u8>)>) -> Result<String, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        if id != "*" {
            return Err(());
        }
        if let Some(entry) = self.data.get_mut(key) {
            return match entry {
                Value::Stream(items) => {
                    let next_id = format!("{}-0", items.len() + 1);
                    items.push((next_id.clone(), fields));
                    Ok(next_id)
                }
                _ => Err(()),
            };
        }
        let next_id = "1-0".to_string();
        self.data
            .insert(key.to_vec(), Value::Stream(vec![(next_id.clone(), fields)]));
        Ok(next_id)
    }

    fn stream_range(&mut self, key: &[u8], start: &str, end: &str) -> Result<Vec<(String, Vec<(Vec<u8>, Vec<u8>)>)>, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        match self.data.get(key) {
            Some(Value::Stream(items)) => {
                if start == "-" && end == "+" {
                    return Ok(items.clone());
                }
                Ok(Vec::new())
            }
            Some(_) => Err(()),
            None => Ok(Vec::new()),
        }
    }

    fn is_expired(&self, key: &[u8]) -> bool {
        if self.expires.is_empty() {
            return false;
        }
        if let Some(&ts) = self.expires.get(key) {
            return ts <= now_ms();
        }
        false
    }
}

// ---------------------------------------------------------------------------
// Db: sharded delegation layer
// ---------------------------------------------------------------------------
// All public methods take &self and lock only the relevant shard(s).
// For multi-key operations that may span shards, locks are acquired in shard
// index order to prevent deadlocks.
// ---------------------------------------------------------------------------

impl Db {
    // --- core key operations (single shard) ---

    pub fn set(&self, key: Vec<u8>, value: Value, expire_at_ms: Option<u64>) {
        self.shard(&key).set(key, value, expire_at_ms);
    }

    pub fn set_with_expire_at(&self, key: Vec<u8>, value: Value, expire_at_ms: Option<u64>) {
        self.shard(&key).set_with_expire_at(key, value, expire_at_ms);
    }

    pub fn remove(&self, key: &[u8]) -> bool {
        self.shard(key).remove(key)
    }

    pub fn exists(&self, key: &[u8]) -> bool {
        self.shard(key).exists(key)
    }

    pub fn ttl_ms(&self, key: &[u8]) -> Option<i64> {
        self.shard(key).ttl_ms(key)
    }

    pub fn set_expire_ms(&self, key: &[u8], ttl_ms: u64) -> bool {
        self.shard(key).set_expire_ms(key, ttl_ms)
    }

    pub fn persist(&self, key: &[u8]) -> i64 {
        self.shard(key).persist(key)
    }

    pub fn value_type(&self, key: &[u8]) -> Option<&'static str> {
        self.shard(key).value_type(key)
    }

    // --- all-shard operations ---

    pub fn purge_expired_all(&self) {
        for shard in &self.shards {
            shard.lock().unwrap().purge_expired_all();
        }
    }

    pub fn snapshot_items(&self) -> Vec<(Vec<u8>, Value, Option<u64>)> {
        let mut out = Vec::new();
        for shard in &self.shards {
            let mut s = shard.lock().unwrap();
            out.extend(s.snapshot_items());
        }
        out
    }

    pub fn len(&self) -> usize {
        let mut total = 0;
        for shard in &self.shards {
            total += shard.lock().unwrap().len();
        }
        total
    }

    pub fn keys(&self) -> Vec<Vec<u8>> {
        let mut all = Vec::new();
        for shard in &self.shards {
            all.extend(shard.lock().unwrap().keys());
        }
        all
    }

    pub fn keys_matching(&self, pattern: &[u8]) -> Vec<Vec<u8>> {
        let mut all = Vec::new();
        for shard in &self.shards {
            all.extend(shard.lock().unwrap().keys_matching(pattern));
        }
        all.sort();
        all
    }

    pub fn flush(&self) -> usize {
        let mut total = 0;
        for shard in &self.shards {
            total += shard.lock().unwrap().flush();
        }
        total
    }

    // --- string operations ---

    pub fn get_string(&self, key: &[u8]) -> Result<Option<Arc<[u8]>>, ()> {
        self.shard(key).get_string(key)
    }

    pub fn set_string(&self, key: Vec<u8>, value: Arc<[u8]>, expire_at_ms: Option<u64>) {
        self.shard(&key).set_string(key, value, expire_at_ms);
    }

    /// SET from borrowed slices — single shard lock, internal alloc only.
    pub fn set_string_from_slices(&self, key: &[u8], value: &[u8], expire_at_ms: Option<u64>) {
        self.shard(key).set_string_from_slices(key, value, expire_at_ms);
    }

    pub fn set_nx(&self, key: Vec<u8>, value: Arc<[u8]>) -> Result<bool, ()> {
        self.shard(&key).set_nx(key, value)
    }

    pub fn append(&self, key: Vec<u8>, value: &[u8]) -> Result<i64, ()> {
        self.shard(&key).append(key, value)
    }

    pub fn incr_by(&self, key: &[u8], delta: i64) -> Result<i64, ()> {
        self.shard(key).incr_by(key, delta)
    }

    // --- list operations ---

    pub fn list_push(&self, key: &[u8], values: &[Arc<[u8]>], left: bool) -> Result<i64, ()> {
        self.shard(key).list_push(key, values, left)
    }

    pub fn list_pop(&self, key: &[u8], left: bool) -> Result<Option<Arc<[u8]>>, ()> {
        self.shard(key).list_pop(key, left)
    }

    pub fn list_range(&self, key: &[u8], start: i64, stop: i64) -> Result<Vec<Arc<[u8]>>, ()> {
        self.shard(key).list_range(key, start, stop)
    }

    pub fn list_len(&self, key: &[u8]) -> Result<i64, ()> {
        self.shard(key).list_len(key)
    }

    pub fn list_index(&self, key: &[u8], index: i64) -> Result<Option<Arc<[u8]>>, ()> {
        self.shard(key).list_index(key, index)
    }

    pub fn list_set(&self, key: &[u8], index: i64, value: &[u8]) -> Result<(), ()> {
        self.shard(key).list_set(key, index, value)
    }

    pub fn list_insert(&self, key: &[u8], before: bool, pivot: &[u8], value: &[u8]) -> Result<i64, ()> {
        self.shard(key).list_insert(key, before, pivot, value)
    }

    pub fn list_rem(&self, key: &[u8], count: i64, value: &[u8]) -> Result<i64, ()> {
        self.shard(key).list_rem(key, count, value)
    }

    // --- hash operations ---

    pub fn hash_set(&self, key: &[u8], field: Arc<[u8]>, value: Arc<[u8]>) -> Result<bool, ()> {
        self.shard(key).hash_set(key, field, value)
    }

    /// Like hash_set but takes borrowed slices, avoiding Arc allocation overhead.
    pub fn hash_set_bytes(&self, key: &[u8], field: &[u8], value: &[u8]) -> Result<bool, ()> {
        self.shard(key).hash_set_bytes(key, field, value)
    }

    pub fn hash_get(&self, key: &[u8], field: &[u8]) -> Result<Option<Arc<[u8]>>, ()> {
        self.shard(key).hash_get(key, field)
    }

    pub fn hash_del(&self, key: &[u8], fields: &[Arc<[u8]>]) -> Result<i64, ()> {
        self.shard(key).hash_del(key, fields)
    }

    pub fn hash_len(&self, key: &[u8]) -> Result<i64, ()> {
        self.shard(key).hash_len(key)
    }

    pub fn hash_exists(&self, key: &[u8], field: &[u8]) -> Result<bool, ()> {
        self.shard(key).hash_exists(key, field)
    }

    pub fn hash_getall(&self, key: &[u8]) -> Result<Vec<(Arc<[u8]>, Arc<[u8]>)>, ()> {
        self.shard(key).hash_getall(key)
    }

    // --- set operations ---

    pub fn set_add(&self, key: &[u8], members: &[Arc<[u8]>]) -> Result<i64, ()> {
        self.shard(key).set_add(key, members)
    }

    /// Single-member SADD: takes borrowed bytes, avoids Vec and Arc alloc when member exists.
    pub fn set_add_single_bytes(&self, key: &[u8], member: &[u8]) -> Result<i64, ()> {
        self.shard(key).set_add_single_bytes(key, member)
    }

    pub fn set_remove(&self, key: &[u8], members: &[Arc<[u8]>]) -> Result<i64, ()> {
        self.shard(key).set_remove(key, members)
    }

    pub fn set_members(&self, key: &[u8]) -> Result<Vec<Arc<[u8]>>, ()> {
        self.shard(key).set_members(key)
    }

    pub fn set_is_member(&self, key: &[u8], member: &[u8]) -> Result<bool, ()> {
        self.shard(key).set_is_member(key, member)
    }

    pub fn set_card(&self, key: &[u8]) -> Result<i64, ()> {
        self.shard(key).set_card(key)
    }

    /// SMOVE across shards: lock both shard buckets in index order to avoid deadlock.
    pub fn set_move(&self, source: &[u8], dest: &[u8], member: &[u8]) -> Result<bool, ()> {
        let si = Self::shard_index(source);
        let di = Self::shard_index(dest);
        if si == di {
            return self.shards[si].lock().unwrap().set_move(source, dest, member);
        }
        // Lock in ascending index order to prevent deadlock.
        let (first, second) = if si < di { (si, di) } else { (di, si) };
        let mut g1 = self.shards[first].lock().unwrap();
        let mut g2 = self.shards[second].lock().unwrap();
        let (src, dst) = if si < di { (&mut *g1, &mut *g2) } else { (&mut *g2, &mut *g1) };
        // Inline cross-shard move: remove from source shard, add in dest shard.
        if src.is_expired(source) { src.remove(source); }
        if dst.is_expired(dest) { dst.remove(dest); }
        let removed = match src.data.get_mut(source) {
            Some(Value::Set(set)) => set.remove(member),
            Some(_) => return Err(()),
            None => return Ok(false),
        };
        if !removed { return Ok(false); }
        if let Some(Value::Set(set)) = src.data.get(source) {
            if set.is_empty() {
                src.data.remove(source);
                src.expires.remove(source);
            }
        }
        if let Some(entry) = dst.data.get_mut(dest) {
            return match entry {
                Value::Set(set) => {
                    set.insert(Arc::from(member));
                    Ok(true)
                }
                _ => Err(()),
            };
        }
        let mut set = HashSet::new();
        set.insert(Arc::from(member));
        dst.data.insert(dest.to_vec(), Value::Set(set));
        Ok(true)
    }

    // --- sorted set operations ---

    pub fn zadd(&self, key: &[u8], score: f64, member: Vec<u8>) -> Result<bool, ()> {
        self.shard(key).zadd(key, score, member)
    }

    pub fn zrem(&self, key: &[u8], members: &[Vec<u8>]) -> Result<i64, ()> {
        self.shard(key).zrem(key, members)
    }

    pub fn zcard(&self, key: &[u8]) -> Result<i64, ()> {
        self.shard(key).zcard(key)
    }

    pub fn zrange(&self, key: &[u8], start: i64, stop: i64) -> Result<Vec<Vec<u8>>, ()> {
        self.shard(key).zrange(key, start, stop)
    }

    // --- stream operations ---

    pub fn stream_add(&self, key: &[u8], id: &str, fields: Vec<(Vec<u8>, Vec<u8>)>) -> Result<String, ()> {
        self.shard(key).stream_add(key, id, fields)
    }

    pub fn stream_range(&self, key: &[u8], start: &str, end: &str) -> Result<Vec<(String, Vec<(Vec<u8>, Vec<u8>)>)>, ()> {
        self.shard(key).stream_range(key, start, end)
    }

    // --- additional list operations ---

    pub fn list_push_x(&self, key: &[u8], values: &[Arc<[u8]>], left: bool) -> Result<i64, ()> {
        self.shard(key).list_push_x(key, values, left)
    }

    pub fn list_trim(&self, key: &[u8], start: i64, stop: i64) -> Result<(), ()> {
        self.shard(key).list_trim(key, start, stop)
    }

    // --- additional hash operations ---

    pub fn hash_incr_by(&self, key: &[u8], field: &[u8], delta: i64) -> Result<i64, ()> {
        self.shard(key).hash_incr_by(key, field, delta)
    }

    pub fn hash_set_nx(&self, key: &[u8], field: Arc<[u8]>, value: Arc<[u8]>) -> Result<bool, ()> {
        self.shard(key).hash_set_nx(key, field, value)
    }

    // --- additional set operations ---

    pub fn set_union(&self, keys: &[&[u8]]) -> Result<Vec<Arc<[u8]>>, ()> {
        // All keys may hash to different shards; for simplicity, lock the first
        // shard and delegate.  Since Shard::set_union iterates data directly,
        // we need all keys in the same shard.  Instead, collect members from
        // each shard individually and merge at the Db level.
        let mut result: HashSet<Arc<[u8]>> = HashSet::new();
        for &key in keys {
            match self.shard(key).set_members(key) {
                Ok(members) => {
                    for m in members {
                        result.insert(m);
                    }
                }
                Err(_) => return Err(()),
            }
        }
        Ok(result.into_iter().collect())
    }

    pub fn set_inter(&self, keys: &[&[u8]]) -> Result<Vec<Arc<[u8]>>, ()> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        // Get members from first set
        let first = match self.shard(keys[0]).set_members(keys[0]) {
            Ok(m) => m,
            Err(_) => return Err(()),
        };
        if first.is_empty() {
            return Ok(Vec::new());
        }
        // Filter by membership in all other sets
        let mut result = Vec::new();
        'outer: for member in &first {
            for &key in &keys[1..] {
                match self.shard(key).set_is_member(key, member.as_ref()) {
                    Ok(true) => {}
                    Ok(false) => continue 'outer,
                    Err(_) => return Err(()),
                }
            }
            result.push(member.clone());
        }
        Ok(result)
    }

    // --- additional stream operations ---

    pub fn stream_len(&self, key: &[u8]) -> Result<i64, ()> {
        self.shard(key).stream_len(key)
    }

    pub fn stream_rev_range(&self, key: &[u8], start: &str, end: &str) -> Result<Vec<(String, Vec<(Vec<u8>, Vec<u8>)>)>, ()> {
        self.shard(key).stream_rev_range(key, start, end)
    }

    pub fn stream_del(&self, key: &[u8], ids: &[&str]) -> Result<i64, ()> {
        self.shard(key).stream_del(key, ids)
    }
}

fn parse_i64_bytes(input: &[u8]) -> Result<i64, ()> {
    if input.is_empty() {
        return Err(());
    }
    let mut idx = 0;
    let mut sign: i128 = 1;
    if input[0] == b'-' {
        sign = -1;
        idx = 1;
    } else if input[0] == b'+' {
        idx = 1;
    }
    if idx >= input.len() {
        return Err(());
    }
    let mut value: i128 = 0;
    for &b in &input[idx..] {
        if b < b'0' || b > b'9' {
            return Err(());
        }
        value = value * 10 + (b - b'0') as i128;
        if value > (i64::MAX as i128) + 1 {
            return Err(());
        }
    }
    value *= sign;
    if value < i64::MIN as i128 || value > i64::MAX as i128 {
        return Err(());
    }
    Ok(value as i64)
}

fn write_i64_bytes(buf: &mut Vec<u8>, n: i64) {
    buf.clear();
    let mut tmp = [0u8; 20];
    let mut idx = tmp.len();
    let mut value = n as i128;
    let negative = value < 0;
    if negative {
        value = -value;
    }
    loop {
        let digit = (value % 10) as u8;
        idx -= 1;
        tmp[idx] = b'0' + digit;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    if negative {
        idx -= 1;
        tmp[idx] = b'-';
    }
    buf.extend_from_slice(&tmp[idx..]);
}

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}
