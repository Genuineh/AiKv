//! AiDb 存储引擎适配器 (StorageAdapter)

use std::path::{Path, PathBuf};
use std::sync::Arc;

use aidb::config::Options;
use aidb::{Checkpoint, WriteBatch, WriteOp as DbWriteOp, DB};
use async_trait::async_trait;
use tokio::task;

use crate::error::{Error, Result};
use crate::storage::adapter::{AdapterWriteOp, StorageAdapter};
use crate::storage::types::StorageEngineKind;

/// AiDb 扁平 KV 引擎
pub struct AiDbEngine {
    pub db: Arc<DB>,
}

impl AiDbEngine {
    /// 构造真实 key: `{db_index}:{user_key}`
    pub fn encode_key(db: usize, user_key: &[u8]) -> Vec<u8> {
        let mut key = Vec::with_capacity(8 + user_key.len());
        key.extend_from_slice(db.to_string().as_bytes());
        key.push(b':');
        key.extend_from_slice(user_key);
        key
    }

    pub fn decode_key(encoded: &[u8]) -> Option<(usize, Vec<u8>)> {
        let pos = encoded.iter().position(|&b| b == b':')?;
        let db_str = std::str::from_utf8(&encoded[..pos]).ok()?;
        let db = db_str.parse().ok()?;
        Some((db, encoded[pos + 1..].to_vec()))
    }

    pub(crate) fn prefix_end(prefix: &[u8]) -> Option<Vec<u8>> {
        let mut end = prefix.to_vec();
        while let Some(last) = end.last_mut() {
            if *last == 0xff {
                end.pop();
            } else {
                *last += 1;
                return Some(end);
            }
        }
        None
    }

    /// 生产 / CLI 默认: `Options::default()` preset.
    pub fn open(path: impl AsRef<Path>) -> Result<Arc<Self>> {
        Self::open_with_options(path, crate::storage::aidb_options::server_db_options(false))
    }

    /// 单测 preset: 小 memtable、关 background_compaction.
    pub fn open_for_testing(path: impl AsRef<Path>) -> Result<Arc<Self>> {
        Self::open_with_options(path, crate::storage::aidb_options::testing_db_options())
    }

    pub fn open_with_options(path: impl AsRef<Path>, mut opts: Options) -> Result<Arc<Self>> {
        opts.create_if_missing = true;
        opts.validate().map_err(|e| Error::Storage(e.to_string()))?;
        let db = DB::open(path, opts).map_err(|e| Error::Storage(e.to_string()))?;
        Ok(Arc::new(Self { db }))
    }

    pub fn approximate_data_memory_bytes(&self) -> u64 {
        self.db.approximate_memory_bytes()
    }

    async fn blocking<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(Arc<DB>) -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        let db = Arc::clone(&self.db);
        task::spawn_blocking(move || f(db))
            .await
            .map_err(|e| Error::Storage(e.to_string()))?
    }
}

#[async_trait]
impl StorageAdapter for AiDbEngine {
    async fn get(&self, key: Vec<u8>) -> Result<Option<Vec<u8>>> {
        self.blocking(move |db| db.get(&key).map_err(|e| Error::Storage(e.to_string())))
            .await
    }

    async fn set(&self, key: Vec<u8>, value: Vec<u8>) -> Result<()> {
        self.blocking(move |db| {
            db.put(&key, &value)
                .map_err(|e| Error::Storage(e.to_string()))
        })
        .await
    }

    async fn delete(&self, key: Vec<u8>) -> Result<bool> {
        self.blocking(move |db| {
            let existed = db.get(&key).ok().flatten().is_some();
            db.delete(&key).map_err(|e| Error::Storage(e.to_string()))?;
            Ok(existed)
        })
        .await
    }

    async fn exists(&self, key: Vec<u8>) -> Result<bool> {
        self.blocking(move |db| {
            Ok(db
                .get(&key)
                .map_err(|e| Error::Storage(e.to_string()))?
                .is_some())
        })
        .await
    }

    async fn write_batch(&self, batch: Vec<AdapterWriteOp>) -> Result<()> {
        self.blocking(move |db| {
            let mut wb = WriteBatch::new();
            for op in batch {
                match op {
                    AdapterWriteOp::Put { key, value } => wb.put(key, value),
                    AdapterWriteOp::Delete { key } => wb.delete(key),
                }
            }
            db.write(&wb).map_err(|e| Error::Storage(e.to_string()))
        })
        .await
    }

    async fn for_each_prefix(
        &self,
        prefix: Vec<u8>,
        mut f: Box<dyn FnMut(Vec<u8>, Vec<u8>) -> Result<()> + Send>,
    ) -> Result<()> {
        self.blocking(move |db| {
            let end = AiDbEngine::prefix_end(&prefix);
            let iter = db
                .scan(Some(prefix.as_slice()), end.as_deref())
                .map_err(|e| Error::Storage(e.to_string()))?;
            for item in iter {
                let (k, v) = item.map_err(|e| Error::Storage(e.to_string()))?;
                f(k, v)?;
            }
            Ok(())
        })
        .await
    }

    async fn delete_range(&self, start: Vec<u8>, end: Vec<u8>) -> Result<()> {
        self.blocking(move |db| {
            db.delete_range(&start, &end)
                .map_err(|e| Error::Storage(e.to_string()))
        })
        .await
    }

    async fn len(&self) -> Result<usize> {
        self.blocking(move |db| {
            let iter = db.iter().map_err(|e| Error::Storage(e.to_string()))?;
            let mut count = 0usize;
            for item in iter {
                let _ = item.map_err(|e| Error::Storage(e.to_string()))?;
                count += 1;
            }
            Ok(count)
        })
        .await
    }

    async fn is_empty(&self) -> Result<bool> {
        Ok(self.len().await? == 0)
    }

    async fn clear(&self) -> Result<()> {
        self.blocking(move |db| {
            let iter = db.iter().map_err(|e| Error::Storage(e.to_string()))?;
            let mut keys = Vec::new();
            for item in iter {
                let (k, _) = item.map_err(|e| Error::Storage(e.to_string()))?;
                keys.push(k);
            }
            let mut wb = WriteBatch::new();
            for key in keys {
                wb.delete(key);
            }
            if !wb.is_empty() {
                db.write(&wb).map_err(|e| Error::Storage(e.to_string()))?;
            }
            Ok(())
        })
        .await
    }

    async fn flush(&self) -> Result<()> {
        self.blocking(move |db| db.flush().map_err(|e| Error::Storage(e.to_string())))
            .await
    }

    async fn create_checkpoint(&self, dest: &Path) -> Result<PathBuf> {
        let dest = dest.to_path_buf();
        self.blocking(move |db| {
            Checkpoint::create(db.as_ref(), &dest).map_err(|e| Error::Storage(e.to_string()))
        })
        .await
    }

    async fn close(&self) -> Result<()> {
        self.blocking(move |db| db.close().map_err(|e| Error::Storage(e.to_string())))
            .await
    }

    fn engine_kind(&self) -> StorageEngineKind {
        StorageEngineKind::AiDb
    }

    fn approximate_memory_bytes(&self) -> Option<u64> {
        Some(self.db.approximate_memory_bytes())
    }
}

#[expect(dead_code)]
fn db_write_op_to_adapter(op: DbWriteOp) -> AdapterWriteOp {
    match op {
        DbWriteOp::Put { key, value } => AdapterWriteOp::Put { key, value },
        DbWriteOp::Delete { key } => AdapterWriteOp::Delete { key },
    }
}
