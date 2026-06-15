//! 存储层类型与 KvStorage trait

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyspaceStats {
  pub keys: usize,
  pub expires: usize,
  /// 带 TTL 的 key 平均剩余生存时间 (毫秒); 无 TTL key 时为 0.
  pub avg_ttl: u64,
}

/// 从剩余 TTL 样本计算 keyspace avg_ttl (毫秒).
pub fn compute_avg_ttl_ms(remaining_ms: impl Iterator<Item = u64>) -> u64 {
  let mut sum = 0u64;
  let mut count = 0u64;
  for ttl in remaining_ms {
    sum = sum.saturating_add(ttl);
    count += 1;
  }
  if count == 0 {
    0
  } else {
    sum.checked_div(count).unwrap_or(0)
  }
}

/// 扫描结果 (内部游标, 供 Phase 10 SCAN 复用)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanResult {
  pub cursor: u64,
  pub keys: Vec<Vec<u8>>,
}

/// 批量写入操作 (命令层; 与底层 LSM WriteOp 独立)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WriteOp {
  Put(Vec<u8>),
  Delete,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ValueType {
  String(Vec<u8>),
  Hash(HashMap<Vec<u8>, Vec<u8>>),
  List(VecDeque<Vec<u8>>),
  Set(HashSet<Vec<u8>>),
  ZSet(BTreeMap<Vec<u8>, f64>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredValue {
  pub value: ValueType,
  /// Unix 毫秒时间戳; None = 永不过期
  pub expires_at: Option<u64>,
}

impl StoredValue {
  pub fn string(value: Vec<u8>) -> Self {
    Self {
      value: ValueType::String(value),
      expires_at: None,
    }
  }

  pub fn is_expired(&self) -> bool {
    let Some(expires_at) = self.expires_at else {
      return false;
    };
    now_ms() >= expires_at
  }

  pub fn type_name(&self) -> &'static str {
    match self.value {
      ValueType::String(_) => "string",
      ValueType::Hash(_) => "hash",
      ValueType::List(_) => "list",
      ValueType::Set(_) => "set",
      ValueType::ZSet(_) => "zset",
    }
  }

  /// 近似堆占用 (key 本体由调用方单独计入).
  pub fn approximate_heap_bytes(&self) -> u64 {
    match &self.value {
      ValueType::String(v) => v.len() as u64,
      ValueType::Hash(h) => h.iter().map(|(k, v)| k.len() + v.len()).sum::<usize>() as u64,
      ValueType::List(l) => l.iter().map(|v| v.len()).sum::<usize>() as u64,
      ValueType::Set(s) => s.iter().map(|v| v.len()).sum::<usize>() as u64,
      ValueType::ZSet(z) => z.keys().map(|k| k.len()).sum::<usize>() as u64,
    }
  }

  pub fn new_list(list: VecDeque<Vec<u8>>) -> Self {
    Self {
      value: ValueType::List(list),
      expires_at: None,
    }
  }

  pub fn new_set(set: HashSet<Vec<u8>>) -> Self {
    Self {
      value: ValueType::Set(set),
      expires_at: None,
    }
  }

  pub fn new_zset(zset: BTreeMap<Vec<u8>, f64>) -> Self {
    Self {
      value: ValueType::ZSet(zset),
      expires_at: None,
    }
  }

  pub fn as_hash(&self) -> Result<&HashMap<Vec<u8>, Vec<u8>>> {
    match &self.value {
      ValueType::Hash(hash) => Ok(hash),
      _ => Err(Error::Command(WRONGTYPE.into())),
    }
  }

  pub fn as_hash_mut(&mut self) -> Result<&mut HashMap<Vec<u8>, Vec<u8>>> {
    match &mut self.value {
      ValueType::Hash(hash) => Ok(hash),
      _ => Err(Error::Command(WRONGTYPE.into())),
    }
  }

  pub fn as_list(&self) -> Result<&VecDeque<Vec<u8>>> {
    match &self.value {
      ValueType::List(list) => Ok(list),
      _ => Err(Error::Command(WRONGTYPE.into())),
    }
  }

  pub fn as_list_mut(&mut self) -> Result<&mut VecDeque<Vec<u8>>> {
    match &mut self.value {
      ValueType::List(list) => Ok(list),
      _ => Err(Error::Command(WRONGTYPE.into())),
    }
  }

  pub fn as_set(&self) -> Result<&HashSet<Vec<u8>>> {
    match &self.value {
      ValueType::Set(set) => Ok(set),
      _ => Err(Error::Command(WRONGTYPE.into())),
    }
  }

  pub fn as_set_mut(&mut self) -> Result<&mut HashSet<Vec<u8>>> {
    match &mut self.value {
      ValueType::Set(set) => Ok(set),
      _ => Err(Error::Command(WRONGTYPE.into())),
    }
  }

  pub fn as_zset(&self) -> Result<&BTreeMap<Vec<u8>, f64>> {
    match &self.value {
      ValueType::ZSet(zset) => Ok(zset),
      _ => Err(Error::Command(WRONGTYPE.into())),
    }
  }

  pub fn as_zset_mut(&mut self) -> Result<&mut BTreeMap<Vec<u8>, f64>> {
    match &mut self.value {
      ValueType::ZSet(zset) => Ok(zset),
      _ => Err(Error::Command(WRONGTYPE.into())),
    }
  }
}

pub fn now_ms() -> u64 {
  use std::time::{SystemTime, UNIX_EPOCH};
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .expect("system time before UNIX epoch")
    .as_millis() as u64
}

/// 永不过期 key 在 `ttl()` 中的 sentinel (仅存储层; 命令层映射为 Redis -1)
pub const TTL_NO_EXPIRY: i64 = -1;

pub const WRONGTYPE: &str = "WRONGTYPE Operation against a key holding the wrong kind of value";

/// AiKv 支持的数据库数量 (与 Redis 默认 16 个 DB 一致)
pub const DB_COUNT: usize = 16;

/// 判断是否为 WRONGTYPE 命令错误 (不依赖 Display 字符串匹配).
pub fn is_wrongtype(err: &Error) -> bool {
  matches!(err, Error::Command(msg) if msg.starts_with("WRONGTYPE"))
}

#[async_trait]
pub trait KvStorage: Send + Sync {
  async fn get(&self, db: usize, key: &[u8]) -> Result<Option<Vec<u8>>>;
  async fn set(&self, db: usize, key: &[u8], value: &[u8]) -> Result<()>;
  async fn set_with_ttl(
    &self,
    db: usize,
    key: &[u8],
    value: &[u8],
    expire_at_ms: u64,
  ) -> Result<()>;
  async fn delete(&self, db: usize, key: &[u8]) -> Result<bool>;
  async fn exists(&self, db: usize, key: &[u8]) -> Result<bool>;

  async fn mget(&self, db: usize, keys: &[Vec<u8>]) -> Result<Vec<Option<Vec<u8>>>>;
  async fn mset(&self, db: usize, pairs: &[(Vec<u8>, Vec<u8>)]) -> Result<()>;
  async fn write_batch(&self, db: usize, ops: Vec<(Vec<u8>, WriteOp)>) -> Result<()>;

  async fn keys(&self, db: usize, pattern: &[u8]) -> Result<Vec<Vec<u8>>>;
  async fn scan(&self, db: usize, cursor: u64, pattern: &[u8], count: usize) -> Result<ScanResult>;
  async fn len(&self, db: usize) -> Result<usize>;
  async fn keyspace_stats(&self, db: usize) -> Result<KeyspaceStats>;
  async fn clear(&self, db: usize) -> Result<()>;
  async fn clear_all(&self) -> Result<()>;

  async fn expire(&self, db: usize, key: &[u8], ttl_ms: u64) -> Result<bool>;
  async fn expire_at(&self, db: usize, key: &[u8], timestamp_ms: u64) -> Result<bool>;
  /// 剩余 TTL 毫秒; None = 不存在/已过期; Some(-1) = 永不过期
  async fn ttl(&self, db: usize, key: &[u8]) -> Result<Option<i64>>;
  async fn persist(&self, db: usize, key: &[u8]) -> Result<bool>;

  async fn db_count(&self) -> Result<usize>;
  async fn swap_db(&self, a: usize, b: usize) -> Result<()>;

  async fn get_typed(&self, db: usize, key: &[u8]) -> Result<Option<StoredValue>>;
  async fn set_typed(&self, db: usize, key: &[u8], value: StoredValue) -> Result<()>;

  async fn rename_key(&self, db: usize, old_key: &[u8], new_key: &[u8]) -> Result<()>;
  async fn rename_key_nx(&self, db: usize, old_key: &[u8], new_key: &[u8]) -> Result<bool>;
  async fn copy_key(
    &self,
    src_db: usize,
    dst_db: usize,
    src_key: &[u8],
    dst_key: &[u8],
    replace: bool,
  ) -> Result<bool>;
  async fn random_key(&self, db: usize) -> Result<Option<Vec<u8>>>;

  fn engine_kind(&self) -> StorageEngineKind {
    StorageEngineKind::Memory
  }

  async fn flush_engine(&self) -> Result<()> {
    Ok(())
  }

  async fn create_checkpoint(&self, _dest: &Path) -> Result<PathBuf> {
    Err(Error::Command(
      "ERR Persistence not supported on memory engine".into(),
    ))
  }

  async fn close_engine(&self) -> Result<()> {
    Ok(())
  }

  /// 各逻辑 DB 的存活 key 数量 (index = db id).
  async fn db_key_counts(&self) -> Result<Vec<usize>> {
    let n = self.db_count().await?;
    let mut out = Vec::with_capacity(n);
    for db in 0..n {
      out.push(self.len(db).await?);
    }
    Ok(out)
  }

  /// 近似内存占用 (字节); 默认 0, 各引擎自行实现.
  async fn memory_usage_bytes(&self) -> Result<u64> {
    Ok(0)
  }
}

/// 底层存储引擎类型 (CLI / INFO / 持久化命令分支).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageEngineKind {
  Memory,
  AiDb,
}

#[cfg(test)]
mod wrongtype_tests {
  use super::*;

  #[test]
  fn test_is_wrongtype() {
    assert!(is_wrongtype(&Error::Command(WRONGTYPE.into())));
    assert!(!is_wrongtype(&Error::Command("ERR other".into())));
  }
}
