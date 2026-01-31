//! In-memory multi-DB store with TTL support.

use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug)]
pub enum Value {
    String(Vec<u8>),
    List(Vec<Vec<u8>>),
    Set(HashSet<Vec<u8>>),
    Hash(HashMap<Vec<u8>, Vec<u8>>),
    ZSet(Vec<(Vec<u8>, f64)>),
    Stream(Vec<(String, Vec<(Vec<u8>, Vec<u8>)>)>),
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

    pub fn persist(&mut self, key: &[u8]) -> i64 {
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

    pub fn keys(&mut self) -> Vec<Vec<u8>> {
        self.purge_expired_all();
        self.data.keys().cloned().collect()
    }

    pub fn keys_matching(&mut self, pattern: &[u8]) -> Vec<Vec<u8>> {
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

    pub fn flush(&mut self) -> usize {
        let count = self.data.len();
        self.data.clear();
        self.expires.clear();
        count
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

    pub fn list_index(&mut self, key: &[u8], index: i64) -> Result<Option<Vec<u8>>, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        match self.data.get(key) {
            Some(Value::List(list)) => {
                let len = list.len() as i64;
                let mut idx = if index < 0 { len + index } else { index };
                if idx < 0 || idx >= len {
                    return Ok(None);
                }
                Ok(Some(list[idx as usize].clone()))
            }
            Some(_) => Err(()),
            None => Ok(None),
        }
    }

    pub fn list_set(&mut self, key: &[u8], index: i64, value: &[u8]) -> Result<(), ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        match self.data.get_mut(key) {
            Some(Value::List(list)) => {
                let len = list.len() as i64;
                let mut idx = if index < 0 { len + index } else { index };
                if idx < 0 || idx >= len {
                    return Err(());
                }
                list[idx as usize] = value.to_vec();
                Ok(())
            }
            Some(_) => Err(()),
            None => Err(()),
        }
    }

    pub fn list_insert(&mut self, key: &[u8], before: bool, pivot: &[u8], value: &[u8]) -> Result<i64, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        match self.data.get_mut(key) {
            Some(Value::List(list)) => {
                let pos = list.iter().position(|v| v.as_slice() == pivot);
                match pos {
                    Some(idx) => {
                        let insert_at = if before { idx } else { idx + 1 };
                        list.insert(insert_at, value.to_vec());
                        Ok(list.len() as i64)
                    }
                    None => Ok(-1),
                }
            }
            Some(_) => Err(()),
            None => Ok(0),
        }
    }

    pub fn list_rem(&mut self, key: &[u8], count: i64, value: &[u8]) -> Result<i64, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        match self.data.get_mut(key) {
            Some(Value::List(list)) => {
                let mut removed = 0i64;
                if count == 0 {
                    list.retain(|v| {
                        if v.as_slice() == value {
                            removed += 1;
                            false
                        } else {
                            true
                        }
                    });
                } else if count > 0 {
                    let mut i = 0usize;
                    while i < list.len() && removed < count {
                        if list[i].as_slice() == value {
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
                        if list[i].as_slice() == value {
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

    pub fn zadd(&mut self, key: &[u8], score: f64, member: Vec<u8>) -> Result<bool, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        let entry = self.data.entry(key.to_vec()).or_insert_with(|| Value::ZSet(Vec::new()));
        match entry {
            Value::ZSet(items) => {
                for item in items.iter_mut() {
                    if item.0 == member {
                        item.1 = score;
                        return Ok(false);
                    }
                }
                items.push((member, score));
                Ok(true)
            }
            _ => Err(()),
        }
    }

    pub fn zrem(&mut self, key: &[u8], members: &[Vec<u8>]) -> Result<i64, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        match self.data.get_mut(key) {
            Some(Value::ZSet(items)) => {
                let before = items.len();
                items.retain(|(m, _)| !members.iter().any(|x| x == m));
                let removed = (before - items.len()) as i64;
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

    pub fn zcard(&mut self, key: &[u8]) -> Result<i64, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        match self.data.get(key) {
            Some(Value::ZSet(items)) => Ok(items.len() as i64),
            Some(_) => Err(()),
            None => Ok(0),
        }
    }

    pub fn zrange(&mut self, key: &[u8], start: i64, stop: i64) -> Result<Vec<Vec<u8>>, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        match self.data.get(key) {
            Some(Value::ZSet(items)) => {
                if items.is_empty() {
                    return Ok(Vec::new());
                }
                let mut sorted = items.clone();
                sorted.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal).then_with(|| a.0.cmp(&b.0)));
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

    pub fn stream_add(&mut self, key: &[u8], id: &str, fields: Vec<(Vec<u8>, Vec<u8>)>) -> Result<String, ()> {
        if self.is_expired(key) {
            self.remove(key);
        }
        if id != "*" {
            return Err(());
        }
        let entry = self.data.entry(key.to_vec()).or_insert_with(|| Value::Stream(Vec::new()));
        match entry {
            Value::Stream(items) => {
                let next_id = format!("{}-0", items.len() + 1);
                items.push((next_id.clone(), fields));
                Ok(next_id)
            }
            _ => Err(()),
        }
    }

    pub fn stream_range(&mut self, key: &[u8], start: &str, end: &str) -> Result<Vec<(String, Vec<(Vec<u8>, Vec<u8>)>)>, ()> {
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
