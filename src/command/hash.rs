//! Hash 命令

use std::collections::HashMap;
use std::sync::Arc;

use bytes::Bytes;
use tracing::instrument;

use crate::command::router::{self, KeyLock};
use crate::command::scan_util;
use crate::error::{Error, Result};
use crate::protocol::RespValue;
use crate::storage::memory::glob_match;
use crate::storage::{KvStorage, StoredValue, ValueType, WRONGTYPE};

pub struct HashCommands {
    storage: Arc<dyn KvStorage>,
    key_lock: Arc<KeyLock>,
}

impl HashCommands {
    pub fn new(storage: Arc<dyn KvStorage>, key_lock: Arc<KeyLock>) -> Self {
        Self { storage, key_lock }
    }

    #[instrument(name = "cmd_hash", skip(self, args), fields(cmd.name = "HSET"))]
    pub async fn hset(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        if args.len() < 3 || args.len().is_multiple_of(2) {
            return Err(router::wrong_args("HSET", ""));
        }
        let key = &args[0];
        let _lock = self.key_lock.lock(key).await;
        let mut stored = self.load_or_create_hash(db, key).await?;
        let ValueType::Hash(ref mut map) = stored.value else {
            return Err(router::wrongtype());
        };
        let mut new_fields = 0i64;
        for chunk in args[1..].chunks(2) {
            let field = chunk[0].to_vec();
            let value = chunk[1].to_vec();
            if !map.contains_key(&field) {
                new_fields += 1;
            }
            map.insert(field, value);
        }
        self.storage.set_typed(db, key, stored).await?;
        Ok(router::integer(new_fields))
    }

    /// HMSET — Redis 兼容: 始终返回 `OK` (HSET 返回新增字段数).
    #[instrument(name = "cmd_hash", skip(self, args), fields(cmd.name = "HMSET"))]
    pub async fn hmset(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        self.hset(db, args).await?;
        Ok(RespValue::SimpleString("OK".into()))
    }

    pub async fn hget(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_args("HGET", args, 2)?;
        let map = self.load_hash(db, &args[0]).await?;
        match map {
            None => Ok(router::nil_bulk()),
            Some(map) => match map.get(args[1].as_ref()) {
                Some(v) => Ok(router::bulk(v.clone())),
                None => Ok(router::nil_bulk()),
            },
        }
    }

    pub async fn hdel(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_min_args("HDEL", args, 2)?;
        let key = &args[0];
        let _lock = self.key_lock.lock(key).await;
        let Some(mut stored) = self.storage.get_typed(db, key).await? else {
            return Ok(router::integer(0));
        };
        let ValueType::Hash(ref mut map) = stored.value else {
            return Err(router::wrongtype());
        };
        let mut count = 0i64;
        for field in &args[1..] {
            if map.remove(field.as_ref()).is_some() {
                count += 1;
            }
        }
        if map.is_empty() {
            self.storage.delete(db, key).await?;
        } else {
            self.storage.set_typed(db, key, stored).await?;
        }
        Ok(router::integer(count))
    }

    pub async fn hexists(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_args("HEXISTS", args, 2)?;
        let map = self.load_hash(db, &args[0]).await?;
        let exists = map.is_some_and(|m| m.contains_key(args[1].as_ref()));
        Ok(router::integer(i64::from(exists)))
    }

    pub async fn hlen(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_args("HLEN", args, 1)?;
        let map = self.load_hash(db, &args[0]).await?;
        Ok(router::integer(map.map(|m| m.len() as i64).unwrap_or(0)))
    }

    pub async fn hkeys(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_args("HKEYS", args, 1)?;
        let map = self.load_hash(db, &args[0]).await?;
        Ok(array_of_bulk(
            map.map(|m| m.keys().cloned().collect()).unwrap_or_default(),
        ))
    }

    pub async fn hvals(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_args("HVALS", args, 1)?;
        let map = self.load_hash(db, &args[0]).await?;
        Ok(array_of_bulk(
            map.map(|m| m.values().cloned().collect())
                .unwrap_or_default(),
        ))
    }

    pub async fn hgetall(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_args("HGETALL", args, 1)?;
        let map = self.load_hash(db, &args[0]).await?;
        let mut items = Vec::new();
        if let Some(map) = map {
            for (k, v) in map {
                items.push(router::bulk(k));
                items.push(router::bulk(v));
            }
        }
        Ok(RespValue::Array(Some(items)))
    }

    pub async fn hmget(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_min_args("HMGET", args, 2)?;
        let map = self.load_hash(db, &args[0]).await?;
        let mut items = Vec::with_capacity(args.len() - 1);
        for field in &args[1..] {
            match &map {
                None => items.push(router::nil_bulk()),
                Some(m) => match m.get(field.as_ref()) {
                    Some(v) => items.push(router::bulk(v.clone())),
                    None => items.push(router::nil_bulk()),
                },
            }
        }
        Ok(RespValue::Array(Some(items)))
    }

    pub async fn hsetnx(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_args("HSETNX", args, 3)?;
        let key = &args[0];
        let _lock = self.key_lock.lock(key).await;
        let mut stored = match self.storage.get_typed(db, key).await? {
            None => StoredValue {
                value: ValueType::Hash(HashMap::new()),
                expires_at: None,
            },
            Some(s) => {
                if !matches!(s.value, ValueType::Hash(_)) {
                    return Err(router::wrongtype());
                }
                s
            }
        };
        let ValueType::Hash(ref mut map) = stored.value else {
            return Err(router::wrongtype());
        };
        if map.contains_key(args[1].as_ref()) {
            return Ok(router::integer(0));
        }
        map.insert(args[1].to_vec(), args[2].to_vec());
        self.storage.set_typed(db, key, stored).await?;
        Ok(router::integer(1))
    }

    pub async fn hscan(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_min_args("HSCAN", args, 2)?;
        let cursor = scan_util::parse_u64(&args[1])?;
        let opts = scan_util::parse_scan_options("HSCAN", args, 2)?;

        let Some(stored) = self.storage.get_typed(db, &args[0]).await? else {
            return Ok(hscan_response(0, &[]));
        };
        if !matches!(stored.value, ValueType::Hash(_)) {
            return Err(router::wrongtype());
        }
        let ValueType::Hash(map) = stored.value else {
            unreachable!()
        };
        let mut pairs: Vec<(Vec<u8>, Vec<u8>)> = map
            .into_iter()
            .filter(|(field, _)| {
                opts.pattern
                    .as_ref()
                    .is_none_or(|p| glob_match(p, field.as_slice()))
            })
            .collect();
        pairs.sort_by(|a, b| a.0.cmp(&b.0));

        let (next_cursor, page) = scan_util::paginate_slice(&pairs, cursor, opts.count);
        Ok(hscan_response(next_cursor, page))
    }

    pub async fn hincrbyfloat(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_args("HINCRBYFLOAT", args, 3)?;
        let delta = parse_f64_field_bytes(&args[2])?;
        if !delta.is_finite() {
            return Err(Error::Command("ERR value is not a valid float".into()));
        }
        let key = &args[0];
        let field = args[1].to_vec();
        let _lock = self.key_lock.lock(key).await;
        let mut stored = self.load_or_create_hash(db, key).await?;
        let ValueType::Hash(ref mut map) = stored.value else {
            return Err(router::wrongtype());
        };
        let current = match map.get(&field) {
            None => 0.0f64,
            Some(existing) => parse_f64_field_bytes(existing)?,
        };
        let new_val = current + delta;
        if !new_val.is_finite() {
            return Err(Error::Command(
                "ERR increment would produce NaN or Infinity".into(),
            ));
        }
        let s = format_hash_float(new_val);
        map.insert(field, s.as_bytes().to_vec());
        self.storage.set_typed(db, key, stored).await?;
        Ok(router::bulk(s.into_bytes()))
    }

    pub async fn hincrby(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_args("HINCRBY", args, 3)?;
        let delta = parse_i64_field(&args[2])?;
        let key = &args[0];
        let field = args[1].to_vec();
        let _lock = self.key_lock.lock(key).await;
        let mut stored = self.load_or_create_hash(db, key).await?;
        let ValueType::Hash(ref mut map) = stored.value else {
            return Err(router::wrongtype());
        };
        let new_val = match map.get(&field) {
            None => delta,
            Some(existing) => {
                let v = parse_i64_field(existing)?;
                v.checked_add(delta).ok_or_else(|| {
                    Error::Command("ERR increment or decrement would overflow".into())
                })?
            }
        };
        map.insert(field, new_val.to_string().into_bytes());
        self.storage.set_typed(db, key, stored).await?;
        Ok(router::integer(new_val))
    }

    async fn load_hash(&self, db: usize, key: &[u8]) -> Result<Option<HashMap<Vec<u8>, Vec<u8>>>> {
        let Some(stored) = self.storage.get_typed(db, key).await? else {
            return Ok(None);
        };
        match stored.value {
            ValueType::Hash(map) => Ok(Some(map)),
            _ => Err(router::wrongtype()),
        }
    }

    async fn load_or_create_hash(&self, db: usize, key: &[u8]) -> Result<StoredValue> {
        match self.storage.get_typed(db, key).await? {
            None => Ok(StoredValue {
                value: ValueType::Hash(HashMap::new()),
                expires_at: None,
            }),
            Some(stored) => match stored.value {
                ValueType::Hash(_) => Ok(stored),
                _ => Err(router::wrongtype()),
            },
        }
    }
}

fn array_of_bulk(items: Vec<Vec<u8>>) -> RespValue {
    RespValue::Array(Some(items.into_iter().map(router::bulk).collect()))
}

fn hscan_response(cursor: u64, page: &[(Vec<u8>, Vec<u8>)]) -> RespValue {
    let mut items = vec![RespValue::BulkString(Some(Bytes::from(cursor.to_string())))];
    let mut pairs = Vec::new();
    for (field, value) in page {
        pairs.push(router::bulk(field.clone()));
        pairs.push(router::bulk(value.clone()));
    }
    items.push(RespValue::Array(Some(pairs)));
    RespValue::Array(Some(items))
}

fn parse_i64_field(b: &[u8]) -> Result<i64> {
    let s = std::str::from_utf8(b).map_err(|_| Error::Command(WRONGTYPE.into()))?;
    s.parse::<i64>()
        .map_err(|_| Error::Command(WRONGTYPE.into()))
}

fn parse_f64_field_bytes(b: &[u8]) -> Result<f64> {
    let s = std::str::from_utf8(b)
        .map_err(|_| Error::Command("ERR hash value is not a valid float".into()))?;
    s.parse::<f64>()
        .map_err(|_| Error::Command("ERR hash value is not a valid float".into()))
}

fn format_hash_float(v: f64) -> String {
    let s = v.to_string();
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    }
}
