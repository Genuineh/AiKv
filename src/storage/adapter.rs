//! StorageAdapter trait 与 KvStorage 适配层

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use bincode;

use crate::error::{Error, Result};
use crate::storage::memory::glob_match;
use crate::storage::observation::StorageObservation;
use crate::storage::types::{
  now_ms, KeyspaceStats, KvStorage, ScanResult, StorageEngineKind, StoredValue, ValueType, WriteOp,
  DB_COUNT, TTL_NO_EXPIRY, WRONGTYPE,
};
use crate::storage::AiDbEngine;

/// 底层扁平 KV 写操作 (与 `storage::WriteOp` 不同)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdapterWriteOp {
  Put { key: Vec<u8>, value: Vec<u8> },
  Delete { key: Vec<u8> },
}

#[async_trait]
pub trait StorageAdapter: Send + Sync {
  async fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>>;
  async fn set(&self, key: &[u8], value: &[u8]) -> Result<()>;
  async fn delete(&self, key: &[u8]) -> Result<bool>;
  async fn exists(&self, key: &[u8]) -> Result<bool>;
  async fn write_batch(&self, batch: Vec<AdapterWriteOp>) -> Result<()>;
  async fn scan_prefix(&self, prefix: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>>;
  async fn delete_range(&self, start: &[u8], end: &[u8]) -> Result<()>;
  async fn len(&self) -> Result<usize>;
  async fn is_empty(&self) -> Result<bool>;
  async fn clear(&self) -> Result<()>;

  async fn flush(&self) -> Result<()> {
    Ok(())
  }

  async fn create_checkpoint(&self, _dest: &Path) -> Result<PathBuf> {
    Err(Error::Command(
      "ERR Persistence not supported on memory engine".into(),
    ))
  }

  async fn close(&self) -> Result<()> {
    Ok(())
  }

  fn engine_kind(&self) -> StorageEngineKind {
    StorageEngineKind::Memory
  }

  /// 近似进程内热数据内存 (字节); 仅持久化引擎实现.
  fn approximate_memory_bytes(&self) -> Option<u64> {
    None
  }
}

/// 将 StorageAdapter 包装为命令层 KvStorage
pub struct KvStorageAdapter {
  storage: Arc<dyn StorageAdapter>,
  db_count: usize,
  observation: Option<Arc<StorageObservation>>,
}

impl KvStorageAdapter {
  pub fn new(storage: Arc<dyn StorageAdapter>) -> Arc<Self> {
    Self::with_observation(storage, None)
  }

  pub fn with_observation(
    storage: Arc<dyn StorageAdapter>,
    observation: Option<Arc<StorageObservation>>,
  ) -> Arc<Self> {
    Arc::new(Self {
      storage,
      db_count: DB_COUNT,
      observation,
    })
  }

  fn check_db(&self, db: usize) -> Result<()> {
    if db >= self.db_count {
      return Err(Error::Command(format!(
        "ERR DB index is out of range (db={db}, max={})",
        self.db_count
      )));
    }
    Ok(())
  }

  fn encode(db: usize, key: &[u8]) -> Vec<u8> {
    AiDbEngine::encode_key(db, key)
  }

  fn decode_user_key(encoded: &[u8]) -> Option<Vec<u8>> {
    AiDbEngine::decode_key(encoded).map(|(_, k)| k)
  }

  fn db_prefix(db: usize) -> Vec<u8> {
    AiDbEngine::encode_key(db, b"")
  }

  async fn get_raw(&self, db: usize, key: &[u8]) -> Result<Option<Vec<u8>>> {
    self.check_db(db)?;
    self.storage.get(&Self::encode(db, key)).await
  }

  async fn set_raw(&self, db: usize, key: &[u8], bytes: Vec<u8>) -> Result<()> {
    self.check_db(db)?;
    self.storage.set(&Self::encode(db, key), &bytes).await
  }

  async fn delete_encoded(&self, db: usize, key: &[u8]) -> Result<bool> {
    self.check_db(db)?;
    self.storage.delete(&Self::encode(db, key)).await
  }

  fn deserialize(bytes: &[u8]) -> Result<StoredValue> {
    bincode::deserialize(bytes).map_err(|e| Error::Storage(format!("bincode decode: {e}")))
  }

  fn serialize(value: &StoredValue) -> Result<Vec<u8>> {
    bincode::serialize(value).map_err(|e| Error::Storage(format!("bincode encode: {e}")))
  }

  async fn load_typed(&self, db: usize, key: &[u8]) -> Result<Option<StoredValue>> {
    let Some(raw) = self.get_raw(db, key).await? else {
      return Ok(None);
    };
    let stored = Self::deserialize(&raw)?;
    if stored.is_expired() {
      if let Some(obs) = &self.observation {
        obs.record_expired_key();
      }
      let _ = self.delete_encoded(db, key).await?;
      return Ok(None);
    }
    Ok(Some(stored))
  }

  async fn keys_for_db(&self, db: usize, pattern: &[u8]) -> Result<Vec<Vec<u8>>> {
    self.check_db(db)?;
    let prefix = Self::db_prefix(db);
    let pairs = self.storage.scan_prefix(&prefix).await?;
    let mut keys = Vec::new();
    for (encoded, raw) in pairs {
      let Some(user_key) = Self::decode_user_key(&encoded) else {
        continue;
      };
      let stored = Self::deserialize(&raw)?;
      if stored.is_expired() {
        if let Some(obs) = &self.observation {
          obs.record_expired_key();
        }
        let _ = self.delete_encoded(db, &user_key).await;
        continue;
      }
      if pattern.is_empty() || glob_match(pattern, &user_key) {
        keys.push(user_key);
      }
    }
    Ok(keys)
  }

  async fn collect_db_entries(&self, db: usize) -> Result<Vec<(Vec<u8>, StoredValue)>> {
    self.check_db(db)?;
    let prefix = Self::db_prefix(db);
    let pairs = self.storage.scan_prefix(&prefix).await?;
    let mut out = Vec::new();
    for (encoded, raw) in pairs {
      let Some(user_key) = Self::decode_user_key(&encoded) else {
        continue;
      };
      let stored = Self::deserialize(&raw)?;
      if !stored.is_expired() {
        out.push((user_key, stored));
      }
    }
    Ok(out)
  }

  async fn clear_db(&self, db: usize) -> Result<()> {
    self.check_db(db)?;
    let prefix = Self::db_prefix(db);
    let end = AiDbEngine::prefix_end(&prefix).unwrap_or_else(|| {
      let mut max = prefix.clone();
      max.push(0xff);
      max
    });
    self.storage.delete_range(&prefix, &end).await
  }
}

#[async_trait]
impl KvStorage for KvStorageAdapter {
  async fn get(&self, db: usize, key: &[u8]) -> Result<Option<Vec<u8>>> {
    let stored = self.get_typed(db, key).await?;
    match stored {
      None => Ok(None),
      Some(s) => match s.value {
        ValueType::String(v) => Ok(Some(v)),
        _ => Err(Error::Command(WRONGTYPE.into())),
      },
    }
  }

  async fn set(&self, db: usize, key: &[u8], value: &[u8]) -> Result<()> {
    self
      .set_typed(db, key, StoredValue::string(value.to_vec()))
      .await
  }

  async fn set_with_ttl(
    &self,
    db: usize,
    key: &[u8],
    value: &[u8],
    expire_at_ms: u64,
  ) -> Result<()> {
    self
      .set_typed(
        db,
        key,
        StoredValue {
          value: ValueType::String(value.to_vec()),
          expires_at: Some(expire_at_ms),
        },
      )
      .await
  }

  async fn delete(&self, db: usize, key: &[u8]) -> Result<bool> {
    self.delete_encoded(db, key).await
  }

  async fn exists(&self, db: usize, key: &[u8]) -> Result<bool> {
    Ok(self.get_typed(db, key).await?.is_some())
  }

  async fn mget(&self, db: usize, keys: &[Vec<u8>]) -> Result<Vec<Option<Vec<u8>>>> {
    let mut out = Vec::with_capacity(keys.len());
    for key in keys {
      out.push(self.get(db, key).await?);
    }
    Ok(out)
  }

  async fn mset(&self, db: usize, pairs: &[(Vec<u8>, Vec<u8>)]) -> Result<()> {
    for (key, value) in pairs {
      self.set(db, key, value).await?;
    }
    Ok(())
  }

  async fn write_batch(&self, db: usize, ops: Vec<(Vec<u8>, WriteOp)>) -> Result<()> {
    let mut batch = Vec::with_capacity(ops.len());
    for (key, op) in ops {
      let encoded = Self::encode(db, &key);
      match op {
        WriteOp::Put(value) => {
          let stored = StoredValue::string(value);
          let bytes = Self::serialize(&stored)?;
          batch.push(AdapterWriteOp::Put {
            key: encoded,
            value: bytes,
          });
        }
        WriteOp::Delete => {
          batch.push(AdapterWriteOp::Delete { key: encoded });
        }
      }
    }
    self.storage.write_batch(batch).await
  }

  async fn keys(&self, db: usize, pattern: &[u8]) -> Result<Vec<Vec<u8>>> {
    self.keys_for_db(db, pattern).await
  }

  async fn scan(&self, db: usize, cursor: u64, pattern: &[u8], count: usize) -> Result<ScanResult> {
    let mut valid = self.keys_for_db(db, pattern).await?;
    valid.sort();
    let skip = cursor as usize;
    let end = (skip + count).min(valid.len());
    let keys = valid[skip..end].to_vec();
    let next_cursor = if end < valid.len() { end as u64 } else { 0 };
    Ok(ScanResult {
      cursor: next_cursor,
      keys,
    })
  }

  async fn len(&self, db: usize) -> Result<usize> {
    Ok(self.keys_for_db(db, b"").await?.len())
  }

  async fn keyspace_stats(&self, db: usize) -> Result<KeyspaceStats> {
    let entries = self.collect_db_entries(db).await?;
    let now = now_ms();
    let keys = entries.len();
    let mut expires = 0usize;
    let mut ttl_remaining = Vec::new();
    for (_, v) in &entries {
      if let Some(exp) = v.expires_at {
        expires += 1;
        if exp > now {
          ttl_remaining.push(exp - now);
        }
      }
    }
    let avg_ttl = crate::storage::types::compute_avg_ttl_ms(ttl_remaining.into_iter());
    Ok(KeyspaceStats {
      keys,
      expires,
      avg_ttl,
    })
  }

  async fn clear(&self, db: usize) -> Result<()> {
    self.clear_db(db).await
  }

  async fn clear_all(&self) -> Result<()> {
    for db in 0..self.db_count {
      self.clear_db(db).await?;
    }
    Ok(())
  }

  async fn expire(&self, db: usize, key: &[u8], ttl_ms: u64) -> Result<bool> {
    if ttl_ms == 0 {
      return self.delete(db, key).await;
    }
    let Some(mut stored) = self.load_typed(db, key).await? else {
      return Ok(false);
    };
    stored.expires_at = Some(now_ms().saturating_add(ttl_ms));
    self.set_typed(db, key, stored).await?;
    Ok(true)
  }

  async fn expire_at(&self, db: usize, key: &[u8], timestamp_ms: u64) -> Result<bool> {
    let now = now_ms();
    if timestamp_ms <= now {
      return self.delete(db, key).await;
    }
    let Some(mut stored) = self.load_typed(db, key).await? else {
      return Ok(false);
    };
    stored.expires_at = Some(timestamp_ms);
    self.set_typed(db, key, stored).await?;
    Ok(true)
  }

  async fn ttl(&self, db: usize, key: &[u8]) -> Result<Option<i64>> {
    let Some(stored) = self.load_typed(db, key).await? else {
      return Ok(None);
    };
    match stored.expires_at {
      None => Ok(Some(TTL_NO_EXPIRY)),
      Some(expires_at) => {
        let now = now_ms();
        if now >= expires_at {
          let _ = self.delete(db, key).await?;
          Ok(None)
        } else {
          Ok(Some((expires_at - now) as i64))
        }
      }
    }
  }

  async fn persist(&self, db: usize, key: &[u8]) -> Result<bool> {
    let Some(mut stored) = self.load_typed(db, key).await? else {
      return Ok(false);
    };
    stored.expires_at = None;
    self.set_typed(db, key, stored).await?;
    Ok(true)
  }

  async fn db_count(&self) -> Result<usize> {
    Ok(self.db_count)
  }

  async fn swap_db(&self, a: usize, b: usize) -> Result<()> {
    self.check_db(a)?;
    self.check_db(b)?;
    if a == b {
      return Ok(());
    }
    let entries_a = self.collect_db_entries(a).await?;
    let entries_b = self.collect_db_entries(b).await?;
    self.clear_db(a).await?;
    self.clear_db(b).await?;
    for (key, value) in entries_b {
      self.set_typed(a, &key, value).await?;
    }
    for (key, value) in entries_a {
      self.set_typed(b, &key, value).await?;
    }
    Ok(())
  }

  async fn get_typed(&self, db: usize, key: &[u8]) -> Result<Option<StoredValue>> {
    self.load_typed(db, key).await
  }

  async fn set_typed(&self, db: usize, key: &[u8], value: StoredValue) -> Result<()> {
    let bytes = Self::serialize(&value)?;
    self.set_raw(db, key, bytes).await
  }

  async fn rename_key(&self, db: usize, old_key: &[u8], new_key: &[u8]) -> Result<()> {
    if old_key == new_key {
      return Ok(());
    }
    let Some(value) = self.load_typed(db, old_key).await? else {
      return Err(Error::Command("ERR no such key".into()));
    };
    self.delete_encoded(db, old_key).await?;
    self.set_typed(db, new_key, value).await
  }

  async fn rename_key_nx(&self, db: usize, old_key: &[u8], new_key: &[u8]) -> Result<bool> {
    if old_key == new_key {
      return Ok(true);
    }
    let Some(value) = self.load_typed(db, old_key).await? else {
      return Err(Error::Command("ERR no such key".into()));
    };
    if self.load_typed(db, new_key).await?.is_some() {
      return Ok(false);
    }
    self.delete_encoded(db, old_key).await?;
    self.set_typed(db, new_key, value).await?;
    Ok(true)
  }

  async fn copy_key(
    &self,
    src_db: usize,
    dst_db: usize,
    src_key: &[u8],
    dst_key: &[u8],
    replace: bool,
  ) -> Result<bool> {
    let Some(stored) = self.load_typed(src_db, src_key).await? else {
      return Ok(false);
    };
    if src_db == dst_db && src_key == dst_key {
      return Ok(replace);
    }
    if self.load_typed(dst_db, dst_key).await?.is_some() && !replace {
      return Ok(false);
    }
    self.set_typed(dst_db, dst_key, stored).await?;
    Ok(true)
  }

  async fn random_key(&self, db: usize) -> Result<Option<Vec<u8>>> {
    let keys = self.keys_for_db(db, b"*").await?;
    if keys.is_empty() {
      return Ok(None);
    }
    let idx = unix_nanos() as usize % keys.len();
    Ok(Some(keys[idx].clone()))
  }

  fn engine_kind(&self) -> StorageEngineKind {
    self.storage.engine_kind()
  }

  async fn flush_engine(&self) -> Result<()> {
    self.storage.flush().await
  }

  async fn create_checkpoint(&self, dest: &Path) -> Result<PathBuf> {
    self.storage.create_checkpoint(dest).await
  }

  async fn close_engine(&self) -> Result<()> {
    self.storage.close().await
  }

  async fn memory_usage_bytes(&self) -> Result<u64> {
    Ok(self.storage.approximate_memory_bytes().unwrap_or(0))
  }
}

fn unix_nanos() -> u128 {
  use std::time::{SystemTime, UNIX_EPOCH};
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .unwrap_or_default()
    .as_nanos()
}
