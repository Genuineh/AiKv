//! Set 命令

use std::collections::HashSet;
use std::sync::Arc;

use bytes::Bytes;
use rand::seq::SliceRandom;
use tracing::instrument;

use crate::command::router::{self, KeyLock};
use crate::command::scan_util;
use crate::error::{Error, Result};
use crate::protocol::RespValue;
use crate::storage::memory::glob_match;
use crate::storage::{KvStorage, StoredValue, ValueType};

pub struct SetCommands {
    storage: Arc<dyn KvStorage>,
    key_lock: Arc<KeyLock>,
}

impl SetCommands {
    pub fn new(storage: Arc<dyn KvStorage>, key_lock: Arc<KeyLock>) -> Self {
        Self { storage, key_lock }
    }

    #[instrument(name = "cmd_set", skip(self, args), fields(cmd.name = "SADD"))]
    pub async fn sadd(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_min_args("SADD", args, 2)?;
        let key = &args[0];
        let _lock = self.key_lock.lock(key).await;
        let mut stored = self.load_or_create_set(db, key).await?;
        let set = stored.as_set_mut()?;
        let mut count = 0i64;
        for member in &args[1..] {
            if set.insert(member.to_vec()) {
                count += 1;
            }
        }
        self.storage.set_typed(db, key, stored).await?;
        Ok(router::integer(count))
    }

    pub async fn srem(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_min_args("SREM", args, 2)?;
        let key = &args[0];
        let _lock = self.key_lock.lock(key).await;
        let Some(mut stored) = self.storage.get_typed(db, key).await? else {
            return Ok(router::integer(0));
        };
        let set = stored.as_set_mut()?;
        let mut count = 0i64;
        for member in &args[1..] {
            if set.remove(member.as_ref()) {
                count += 1;
            }
        }
        if set.is_empty() {
            self.storage.delete(db, key).await?;
        } else {
            self.storage.set_typed(db, key, stored).await?;
        }
        Ok(router::integer(count))
    }

    pub async fn sismember(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_args("SISMEMBER", args, 2)?;
        let set = self.load_set(db, &args[0]).await?;
        let exists = set.is_some_and(|s| s.contains(args[1].as_ref()));
        Ok(router::integer(i64::from(exists)))
    }

    pub async fn smembers(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_args("SMEMBERS", args, 1)?;
        let set = self.load_set(db, &args[0]).await?;
        Ok(array_of_bulk(
            set.map(|s| s.iter().cloned().collect()).unwrap_or_default(),
        ))
    }

    pub async fn scard(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_args("SCARD", args, 1)?;
        let set = self.load_set(db, &args[0]).await?;
        Ok(router::integer(set.map(|s| s.len() as i64).unwrap_or(0)))
    }

    pub async fn spop(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_min_args("SPOP", args, 1)?;
        let count = if args.len() > 1 {
            Some(parse_i64(&args[1])?)
        } else {
            None
        };
        let _lock = self.key_lock.lock(&args[0]).await;
        let Some(mut stored) = self.storage.get_typed(db, &args[0]).await? else {
            return Ok(router::nil_bulk());
        };
        let set = stored.as_set_mut()?;
        let count = count.unwrap_or(1);
        if count == 0 {
            return Ok(RespValue::Array(Some(vec![])));
        }
        let mut members: Vec<Vec<u8>> = set.iter().cloned().collect();
        members.shuffle(&mut rand::thread_rng());
        let n = (count as usize).min(members.len());
        let picked: Vec<Vec<u8>> = members.into_iter().take(n).collect();
        for m in &picked {
            set.remove(m);
        }
        if set.is_empty() {
            self.storage.delete(db, &args[0]).await?;
        } else {
            self.storage.set_typed(db, &args[0], stored).await?;
        }
        if count == 1 && picked.len() == 1 {
            return Ok(router::bulk(picked.into_iter().next().unwrap()));
        }
        Ok(array_of_bulk(picked))
    }

    pub async fn srandmember(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_min_args("SRANDMEMBER", args, 1)?;
        let count = if args.len() > 1 {
            Some(parse_i64(&args[1])?)
        } else {
            None
        };
        let set = self.load_set(db, &args[0]).await?;
        let Some(set) = set else {
            return if count.is_some() {
                Ok(RespValue::Array(Some(vec![])))
            } else {
                Ok(router::nil_bulk())
            };
        };
        let mut members: Vec<Vec<u8>> = set.iter().cloned().collect();
        let count = count.unwrap_or(1);
        if count == 0 {
            return Ok(RespValue::Array(Some(vec![])));
        }
        let mut rng = rand::thread_rng();
        if count > 0 {
            members.shuffle(&mut rng);
            let n = (count as usize).min(members.len());
            let picked: Vec<Vec<u8>> = members.into_iter().take(n).collect();
            if args.len() == 1 {
                return Ok(router::bulk(picked.into_iter().next().unwrap()));
            }
            return Ok(array_of_bulk(picked));
        }
        let picked: Vec<Vec<u8>> = (0..(-count) as usize)
            .map(|_| members.choose(&mut rng).cloned().unwrap())
            .collect();
        Ok(array_of_bulk(picked))
    }

    pub async fn sunion(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_min_args("SUNION", args, 1)?;
        let result = self.compute_union(db, args).await?;
        Ok(array_of_bulk(result.into_iter().collect()))
    }

    pub async fn sinter(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_min_args("SINTER", args, 1)?;
        let result = self.compute_inter(db, args).await?;
        Ok(array_of_bulk(result.into_iter().collect()))
    }

    pub async fn sdiff(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_min_args("SDIFF", args, 1)?;
        let result = self.compute_diff(db, args).await?;
        Ok(array_of_bulk(result.into_iter().collect()))
    }

    pub async fn sunionstore(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_min_args("SUNIONSTORE", args, 2)?;
        let dest = &args[0];
        let result = self.compute_union(db, &args[1..]).await?;
        self.store_set(db, dest, result).await
    }

    pub async fn sinterstore(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_min_args("SINTERSTORE", args, 2)?;
        let dest = &args[0];
        let result = self.compute_inter(db, &args[1..]).await?;
        self.store_set(db, dest, result).await
    }

    pub async fn sdiffstore(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_min_args("SDIFFSTORE", args, 2)?;
        let dest = &args[0];
        let result = self.compute_diff(db, &args[1..]).await?;
        self.store_set(db, dest, result).await
    }

    #[instrument(name = "cmd_set", skip(self, args), fields(cmd.name = "SMOVE"))]
    pub async fn smove(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_args("SMOVE", args, 3)?;
        let source = &args[0];
        let dest = &args[1];
        let member = args[2].to_vec();

        let (_lock_a, _lock_b) = self.key_lock.lock_two(source, dest).await;
        let same_key = source == dest;

        let Some(mut src_stored) = self.storage.get_typed(db, source).await? else {
            return Ok(router::integer(0));
        };
        let src_set = src_stored.as_set_mut()?;
        if !src_set.remove(&member) {
            return Ok(router::integer(0));
        }

        if same_key {
            src_set.insert(member);
            self.storage.set_typed(db, source, src_stored).await?;
            return Ok(router::integer(1));
        }

        if let Some(dest_stored) = self.storage.get_typed(db, dest).await? {
            if !matches!(dest_stored.value, ValueType::Set(_)) {
                return Err(router::wrongtype());
            }
        }

        if src_set.is_empty() {
            self.storage.delete(db, source).await?;
        } else {
            self.storage.set_typed(db, source, src_stored).await?;
        }

        let mut dest_stored = match self.storage.get_typed(db, dest).await? {
            None => StoredValue::new_set(HashSet::new()),
            Some(stored) => stored,
        };
        let dest_set = dest_stored.as_set_mut()?;
        dest_set.insert(member);
        self.storage.set_typed(db, dest, dest_stored).await?;
        Ok(router::integer(1))
    }

    #[instrument(name = "cmd_set", skip(self, args), fields(cmd.name = "SSCAN"))]
    pub async fn sscan(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_min_args("SSCAN", args, 2)?;
        let cursor = scan_util::parse_u64(&args[1])?;
        let opts = scan_util::parse_scan_options("SSCAN", args, 2)?;

        let Some(stored) = self.storage.get_typed(db, &args[0]).await? else {
            return Ok(scan_util::scan_response_bulk(0, &[]));
        };
        if !matches!(stored.value, ValueType::Set(_)) {
            return Err(router::wrongtype());
        }
        let set = stored.as_set()?;
        let mut members: Vec<Vec<u8>> = set
            .iter()
            .filter(|m| {
                opts.pattern
                    .as_ref()
                    .is_none_or(|p| glob_match(p, m.as_slice()))
            })
            .cloned()
            .collect();
        members.sort();

        let (next_cursor, page) = scan_util::paginate_slice(&members, cursor, opts.count);
        Ok(scan_util::scan_response_bulk(next_cursor, page))
    }

    async fn store_set(&self, db: usize, dest: &Bytes, set: HashSet<Vec<u8>>) -> Result<RespValue> {
        let _lock = self.key_lock.lock(dest).await;
        if let Some(stored) = self.storage.get_typed(db, dest).await? {
            if !matches!(stored.value, ValueType::Set(_)) {
                return Err(router::wrongtype());
            }
        }
        let len = set.len() as i64;
        self.storage
            .set_typed(db, dest, StoredValue::new_set(set))
            .await?;
        Ok(router::integer(len))
    }

    async fn compute_union(&self, db: usize, keys: &[Bytes]) -> Result<HashSet<Vec<u8>>> {
        let mut result = HashSet::new();
        for key in keys {
            match self.storage.get_typed(db, key).await? {
                None => {}
                Some(stored) => {
                    let set = stored.as_set()?;
                    result.extend(set.iter().cloned());
                }
            }
        }
        Ok(result)
    }

    async fn compute_inter(&self, db: usize, keys: &[Bytes]) -> Result<HashSet<Vec<u8>>> {
        let mut result: Option<HashSet<Vec<u8>>> = None;
        for key in keys {
            let Some(stored) = self.storage.get_typed(db, key).await? else {
                return Ok(HashSet::new());
            };
            let set = stored.as_set()?;
            result = Some(match result {
                None => set.clone(),
                Some(mut acc) => {
                    acc.retain(|m| set.contains(m));
                    acc
                }
            });
        }
        Ok(result.unwrap_or_default())
    }

    async fn compute_diff(&self, db: usize, keys: &[Bytes]) -> Result<HashSet<Vec<u8>>> {
        let first = self.storage.get_typed(db, &keys[0]).await?;
        let mut result = match first {
            None => HashSet::new(),
            Some(stored) => stored.as_set()?.clone(),
        };
        for key in &keys[1..] {
            match self.storage.get_typed(db, key).await? {
                None => {}
                Some(stored) => {
                    let set = stored.as_set()?;
                    result.retain(|m| !set.contains(m));
                }
            }
        }
        Ok(result)
    }

    async fn load_set(&self, db: usize, key: &[u8]) -> Result<Option<HashSet<Vec<u8>>>> {
        let Some(stored) = self.storage.get_typed(db, key).await? else {
            return Ok(None);
        };
        Ok(Some(stored.as_set()?.clone()))
    }

    async fn load_or_create_set(&self, db: usize, key: &[u8]) -> Result<StoredValue> {
        match self.storage.get_typed(db, key).await? {
            None => Ok(StoredValue::new_set(HashSet::new())),
            Some(stored) => match stored.value {
                ValueType::Set(_) => Ok(stored),
                _ => Err(router::wrongtype()),
            },
        }
    }
}

fn array_of_bulk(items: Vec<Vec<u8>>) -> RespValue {
    RespValue::Array(Some(items.into_iter().map(router::bulk).collect()))
}

fn parse_i64(b: &Bytes) -> Result<i64> {
    let s =
        std::str::from_utf8(b).map_err(|_| Error::Command("ERR value is not an integer".into()))?;
    s.parse::<i64>()
        .map_err(|_| Error::Command("ERR value is not an integer".into()))
}
