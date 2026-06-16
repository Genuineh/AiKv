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

    pub fn open(path: impl AsRef<Path>) -> Result<Arc<Self>> {
        let mut opts = Options::for_testing();
        opts.create_if_missing = true;
        opts.sync_wal = true;
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
    async fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        let key = key.to_vec();
        self.blocking(move |db| db.get(&key).map_err(|e| Error::Storage(e.to_string())))
            .await
    }

    async fn set(&self, key: &[u8], value: &[u8]) -> Result<()> {
        let key = key.to_vec();
        let value = value.to_vec();
        self.blocking(move |db| {
            db.put(&key, &value)
                .map_err(|e| Error::Storage(e.to_string()))
        })
        .await
    }

    async fn delete(&self, key: &[u8]) -> Result<bool> {
        let key = key.to_vec();
        self.blocking(move |db| {
            let existed = db.get(&key).ok().flatten().is_some();
            db.delete(&key).map_err(|e| Error::Storage(e.to_string()))?;
            Ok(existed)
        })
        .await
    }

    async fn exists(&self, key: &[u8]) -> Result<bool> {
        Ok(self.get(key).await?.is_some())
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

    async fn scan_prefix(&self, prefix: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        let prefix = prefix.to_vec();
        self.blocking(move |db| {
            let end = AiDbEngine::prefix_end(&prefix);
            let iter = db
                .scan(Some(prefix.as_slice()), end.as_deref())
                .map_err(|e| Error::Storage(e.to_string()))?;
            let mut out = Vec::new();
            for item in iter {
                let (k, v) = item.map_err(|e| Error::Storage(e.to_string()))?;
                out.push((k, v));
            }
            Ok(out)
        })
        .await
    }

    async fn delete_range(&self, start: &[u8], end: &[u8]) -> Result<()> {
        let start = start.to_vec();
        let end = end.to_vec();
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
