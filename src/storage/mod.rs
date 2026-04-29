pub mod aidb_adapter;
pub mod memory_adapter;

#[cfg(feature = "cluster")]
mod cluster_raft;

#[cfg(feature = "cluster")]
use cluster_raft::ClusterRaftEngine;

// Re-export the memory adapter as StorageAdapter for backward compatibility
// In production, you would switch to aidb_adapter::AiDbStorageAdapter
pub use memory_adapter::StorageAdapter;

// Also export the AiDb adapter
pub use aidb_adapter::AiDbStorageAdapter;

// Export the core storage types for command implementations
pub use memory_adapter::{BatchOp, SerializableStoredValue, StoredValue, ValueType};

use crate::error::Result;
use crate::observability::LockMetrics;
use bytes::Bytes;
use std::collections::HashMap;
use std::sync::Arc;

/// Unified storage engine that wraps both memory and AiDb adapters.
/// This enum allows seamless switching between storage backends via configuration.
#[derive(Clone)]
pub enum StorageEngine {
    /// In-memory storage, high performance, no persistence
    Memory(StorageAdapter),
    /// AiDb LSM-Tree persistent storage
    AiDb(AiDbStorageAdapter),
    /// Cluster mode: user data via Multi-Raft (replicated)
    #[cfg(feature = "cluster")]
    ClusterRaft(ClusterRaftEngine),
}

impl StorageEngine {
    /// Create a new memory storage engine
    pub fn new_memory(db_count: usize) -> Self {
        StorageEngine::Memory(StorageAdapter::with_db_count(db_count))
    }

    /// Attach lock contention metrics to the underlying storage adapter.
    /// No-op for the AiDb backend (which manages its own locking).
    pub fn set_lock_metrics(&mut self, metrics: Arc<LockMetrics>) {
        if let StorageEngine::Memory(adapter) = self {
            adapter.set_lock_metrics(metrics);
        }
    }

    /// Create a new AiDb storage engine
    pub fn new_aidb(path: &str, db_count: usize) -> Result<Self> {
        Ok(StorageEngine::AiDb(AiDbStorageAdapter::new(
            path, db_count,
        )?))
    }

    /// Multi-Raft backed storage (cluster mode).
    #[cfg(feature = "cluster")]
    pub fn new_cluster_raft(multi: std::sync::Arc<aidb::cluster::MultiRaftNode>, db_count: usize) -> Self {
        StorageEngine::ClusterRaft(ClusterRaftEngine::new(multi, db_count))
    }

    // ========================================================================
    // CORE STORAGE METHODS
    // ========================================================================

    /// Get a stored value by key from a specific database.
    pub fn get_value(&self, db_index: usize, key: &str) -> Result<Option<StoredValue>> {
        match self {
            StorageEngine::Memory(adapter) => adapter.get_value(db_index, key),
            StorageEngine::AiDb(adapter) => adapter.get_value(db_index, key),
            #[cfg(feature = "cluster")]
            StorageEngine::ClusterRaft(c) => c.get_value(db_index, key),
        }
    }

    /// Set a value for a key in a specific database.
    pub fn set_value(&self, db_index: usize, key: String, value: StoredValue) -> Result<()> {
        match self {
            StorageEngine::Memory(adapter) => adapter.set_value(db_index, key, value),
            StorageEngine::AiDb(adapter) => adapter.set_value(db_index, key, value),
            #[cfg(feature = "cluster")]
            StorageEngine::ClusterRaft(c) => c.set_value(db_index, key, value),
        }
    }

    /// Atomically delete a key and return its value.
    pub fn delete_and_get(&self, db_index: usize, key: &str) -> Result<Option<StoredValue>> {
        match self {
            StorageEngine::Memory(adapter) => adapter.delete_and_get(db_index, key),
            StorageEngine::AiDb(adapter) => adapter.delete_and_get(db_index, key),
            #[cfg(feature = "cluster")]
            StorageEngine::ClusterRaft(c) => c.delete_and_get(db_index, key),
        }
    }

    /// Atomically update a value using a closure.
    pub fn update_value<F>(&self, db_index: usize, key: &str, f: F) -> Result<bool>
    where
        F: FnOnce(&mut StoredValue) -> Result<()>,
    {
        match self {
            StorageEngine::Memory(adapter) => adapter.update_value(db_index, key, f),
            StorageEngine::AiDb(adapter) => adapter.update_value(db_index, key, f),
            #[cfg(feature = "cluster")]
            StorageEngine::ClusterRaft(c) => c.update_value(db_index, key, f),
        }
    }

    /// Write a batch of operations atomically.
    pub fn write_batch(&self, db_index: usize, operations: Vec<(String, BatchOp)>) -> Result<()> {
        match self {
            StorageEngine::Memory(adapter) => adapter.write_batch(db_index, operations),
            StorageEngine::AiDb(adapter) => adapter.write_batch(db_index, operations),
            #[cfg(feature = "cluster")]
            StorageEngine::ClusterRaft(c) => c.write_batch(db_index, operations),
        }
    }

    // ========================================================================
    // LEGACY METHODS (Backward compatibility)
    // ========================================================================

    /// Get a value by key from a specific database
    pub fn get_from_db(&self, db_index: usize, key: &str) -> Result<Option<Bytes>> {
        match self {
            StorageEngine::Memory(adapter) => adapter.get_from_db(db_index, key),
            StorageEngine::AiDb(adapter) => adapter.get_from_db(db_index, key),
            #[cfg(feature = "cluster")]
            StorageEngine::ClusterRaft(c) => c.get_from_db(db_index, key),
        }
    }

    /// Get a value by key (from default database 0)
    pub fn get(&self, key: &str) -> Result<Option<Bytes>> {
        match self {
            StorageEngine::Memory(adapter) => adapter.get(key),
            StorageEngine::AiDb(adapter) => adapter.get(key),
            #[cfg(feature = "cluster")]
            StorageEngine::ClusterRaft(c) => c.get(key),
        }
    }

    /// Set a value for a key in a specific database
    pub fn set_in_db(&self, db_index: usize, key: String, value: Bytes) -> Result<()> {
        match self {
            StorageEngine::Memory(adapter) => adapter.set_in_db(db_index, key, value),
            StorageEngine::AiDb(adapter) => adapter.set_in_db(db_index, key, value),
            #[cfg(feature = "cluster")]
            StorageEngine::ClusterRaft(c) => c.set_in_db(db_index, key, value),
        }
    }

    /// Set a value for a key (in default database 0)
    pub fn set(&self, key: String, value: Bytes) -> Result<()> {
        match self {
            StorageEngine::Memory(adapter) => adapter.set(key, value),
            StorageEngine::AiDb(adapter) => adapter.set(key, value),
            #[cfg(feature = "cluster")]
            StorageEngine::ClusterRaft(c) => c.set(key, value),
        }
    }

    /// Set a value with expiration time in milliseconds
    pub fn set_with_expiration_in_db(
        &self,
        db_index: usize,
        key: String,
        value: Bytes,
        expires_at: u64,
    ) -> Result<()> {
        match self {
            StorageEngine::Memory(adapter) => {
                adapter.set_with_expiration_in_db(db_index, key, value, expires_at)
            }
            StorageEngine::AiDb(adapter) => {
                adapter.set_with_expiration_in_db(db_index, key, value, expires_at)
            }
            #[cfg(feature = "cluster")]
            StorageEngine::ClusterRaft(c) => {
                c.set_with_expiration_in_db(db_index, key, value, expires_at)
            }
        }
    }

    /// Set expiration for a key in milliseconds
    pub fn set_expire_in_db(&self, db_index: usize, key: &str, expire_ms: u64) -> Result<bool> {
        match self {
            StorageEngine::Memory(adapter) => adapter.set_expire_in_db(db_index, key, expire_ms),
            StorageEngine::AiDb(adapter) => adapter.set_expire_in_db(db_index, key, expire_ms),
            #[cfg(feature = "cluster")]
            StorageEngine::ClusterRaft(c) => c.set_expire_in_db(db_index, key, expire_ms),
        }
    }

    /// Set expiration at absolute timestamp in milliseconds
    pub fn set_expire_at_in_db(
        &self,
        db_index: usize,
        key: &str,
        timestamp_ms: u64,
    ) -> Result<bool> {
        match self {
            StorageEngine::Memory(adapter) => {
                adapter.set_expire_at_in_db(db_index, key, timestamp_ms)
            }
            StorageEngine::AiDb(adapter) => {
                adapter.set_expire_at_in_db(db_index, key, timestamp_ms)
            }
            #[cfg(feature = "cluster")]
            StorageEngine::ClusterRaft(c) => c.set_expire_at_in_db(db_index, key, timestamp_ms),
        }
    }

    /// Get TTL in milliseconds
    pub fn get_ttl_in_db(&self, db_index: usize, key: &str) -> Result<i64> {
        match self {
            StorageEngine::Memory(adapter) => adapter.get_ttl_in_db(db_index, key),
            StorageEngine::AiDb(adapter) => adapter.get_ttl_in_db(db_index, key),
            #[cfg(feature = "cluster")]
            StorageEngine::ClusterRaft(c) => c.get_ttl_in_db(db_index, key),
        }
    }

    /// Get expiration timestamp in milliseconds
    pub fn get_expire_time_in_db(&self, db_index: usize, key: &str) -> Result<i64> {
        match self {
            StorageEngine::Memory(adapter) => adapter.get_expire_time_in_db(db_index, key),
            StorageEngine::AiDb(adapter) => adapter.get_expire_time_in_db(db_index, key),
            #[cfg(feature = "cluster")]
            StorageEngine::ClusterRaft(c) => c.get_expire_time_in_db(db_index, key),
        }
    }

    /// Remove expiration from a key
    pub fn persist_in_db(&self, db_index: usize, key: &str) -> Result<bool> {
        match self {
            StorageEngine::Memory(adapter) => adapter.persist_in_db(db_index, key),
            StorageEngine::AiDb(adapter) => adapter.persist_in_db(db_index, key),
            #[cfg(feature = "cluster")]
            StorageEngine::ClusterRaft(c) => c.persist_in_db(db_index, key),
        }
    }

    /// Delete a key from a specific database
    pub fn delete_from_db(&self, db_index: usize, key: &str) -> Result<bool> {
        match self {
            StorageEngine::Memory(adapter) => adapter.delete_from_db(db_index, key),
            StorageEngine::AiDb(adapter) => adapter.delete_from_db(db_index, key),
            #[cfg(feature = "cluster")]
            StorageEngine::ClusterRaft(c) => c.delete_from_db(db_index, key),
        }
    }

    /// Delete a key (from default database 0)
    pub fn delete(&self, key: &str) -> Result<bool> {
        match self {
            StorageEngine::Memory(adapter) => adapter.delete(key),
            StorageEngine::AiDb(adapter) => adapter.delete(key),
            #[cfg(feature = "cluster")]
            StorageEngine::ClusterRaft(c) => c.delete(key),
        }
    }

    /// Check if a key exists in a specific database
    pub fn exists_in_db(&self, db_index: usize, key: &str) -> Result<bool> {
        match self {
            StorageEngine::Memory(adapter) => adapter.exists_in_db(db_index, key),
            StorageEngine::AiDb(adapter) => adapter.exists_in_db(db_index, key),
            #[cfg(feature = "cluster")]
            StorageEngine::ClusterRaft(c) => c.exists_in_db(db_index, key),
        }
    }

    /// Like [`Self::exists_in_db`], but for RESTORE after `ASKING` during slot IMPORTING (cluster only).
    #[cfg(feature = "cluster")]
    pub fn exists_in_db_for_restore(
        &self,
        db_index: usize,
        key: &str,
        importing_after_asking: bool,
    ) -> Result<bool> {
        match self {
            StorageEngine::Memory(adapter) => adapter.exists_in_db(db_index, key),
            StorageEngine::AiDb(adapter) => adapter.exists_in_db(db_index, key),
            StorageEngine::ClusterRaft(c) => {
                if importing_after_asking {
                    c.exists_in_db_importing_restore(db_index, key)
                } else {
                    c.exists_in_db(db_index, key)
                }
            }
        }
    }

    /// Check if a key exists (in default database 0)
    pub fn exists(&self, key: &str) -> Result<bool> {
        match self {
            StorageEngine::Memory(adapter) => adapter.exists(key),
            StorageEngine::AiDb(adapter) => adapter.exists(key),
            #[cfg(feature = "cluster")]
            StorageEngine::ClusterRaft(c) => c.exists(key),
        }
    }

    /// Get all keys in a database
    pub fn get_all_keys_in_db(&self, db_index: usize) -> Result<Vec<String>> {
        match self {
            StorageEngine::Memory(adapter) => adapter.get_all_keys_in_db(db_index),
            StorageEngine::AiDb(adapter) => adapter.get_all_keys_in_db(db_index),
            #[cfg(feature = "cluster")]
            StorageEngine::ClusterRaft(c) => c.get_all_keys_in_db(db_index),
        }
    }

    /// Scan keys in a database using cursor-based pagination
    ///
    /// Returns (next_cursor, keys). If next_cursor is empty or "0", iteration is complete.
    pub fn scan_keys_in_db(&self, db_index: usize, cursor: &str, count: usize) -> Result<(String, Vec<String>)> {
        match self {
            StorageEngine::Memory(adapter) => adapter.scan_keys_in_db(db_index, cursor, count),
            StorageEngine::AiDb(adapter) => adapter.scan_keys_in_db(db_index, cursor, count),
            #[cfg(feature = "cluster")]
            StorageEngine::ClusterRaft(c) => c.scan_keys_in_db(db_index, cursor, count),
        }
    }

    /// Get database size (number of keys)
    pub fn dbsize_in_db(&self, db_index: usize) -> Result<usize> {
        match self {
            StorageEngine::Memory(adapter) => adapter.dbsize_in_db(db_index),
            StorageEngine::AiDb(adapter) => adapter.dbsize_in_db(db_index),
            #[cfg(feature = "cluster")]
            StorageEngine::ClusterRaft(c) => c.dbsize_in_db(db_index),
        }
    }

    /// Get keyspace stats for INFO keyspace: (keys, expires, avg_ttl_ms)
    pub fn keyspace_stats_in_db(&self, db_index: usize) -> Result<(usize, usize, u64)> {
        match self {
            StorageEngine::Memory(adapter) => adapter.keyspace_stats_in_db(db_index),
            StorageEngine::AiDb(adapter) => adapter.keyspace_stats_in_db(db_index),
            #[cfg(feature = "cluster")]
            StorageEngine::ClusterRaft(c) => c.keyspace_stats_in_db(db_index),
        }
    }

    /// Clear a specific database
    pub fn flush_db(&self, db_index: usize) -> Result<()> {
        match self {
            StorageEngine::Memory(adapter) => adapter.flush_db(db_index),
            StorageEngine::AiDb(adapter) => adapter.flush_db(db_index),
            #[cfg(feature = "cluster")]
            StorageEngine::ClusterRaft(c) => c.flush_db(db_index),
        }
    }

    /// Clear all databases
    pub fn flush_all(&self) -> Result<()> {
        match self {
            StorageEngine::Memory(adapter) => adapter.flush_all(),
            StorageEngine::AiDb(adapter) => adapter.flush_all(),
            #[cfg(feature = "cluster")]
            StorageEngine::ClusterRaft(c) => c.flush_all(),
        }
    }

    /// Swap two databases
    pub fn swap_db(&self, db1: usize, db2: usize) -> Result<()> {
        match self {
            StorageEngine::Memory(adapter) => adapter.swap_db(db1, db2),
            StorageEngine::AiDb(adapter) => adapter.swap_db(db1, db2),
            #[cfg(feature = "cluster")]
            StorageEngine::ClusterRaft(c) => c.swap_db(db1, db2),
        }
    }

    /// Move a key from one database to another
    pub fn move_key(&self, src_db: usize, dst_db: usize, key: &str) -> Result<bool> {
        match self {
            StorageEngine::Memory(adapter) => adapter.move_key(src_db, dst_db, key),
            StorageEngine::AiDb(adapter) => adapter.move_key(src_db, dst_db, key),
            #[cfg(feature = "cluster")]
            StorageEngine::ClusterRaft(c) => c.move_key(src_db, dst_db, key),
        }
    }

    /// Rename a key
    pub fn rename_in_db(&self, db_index: usize, old_key: &str, new_key: &str) -> Result<bool> {
        match self {
            StorageEngine::Memory(adapter) => adapter.rename_in_db(db_index, old_key, new_key),
            StorageEngine::AiDb(adapter) => adapter.rename_in_db(db_index, old_key, new_key),
            #[cfg(feature = "cluster")]
            StorageEngine::ClusterRaft(c) => c.rename_in_db(db_index, old_key, new_key),
        }
    }

    /// Rename a key only if new key doesn't exist
    pub fn rename_nx_in_db(&self, db_index: usize, old_key: &str, new_key: &str) -> Result<bool> {
        match self {
            StorageEngine::Memory(adapter) => adapter.rename_nx_in_db(db_index, old_key, new_key),
            StorageEngine::AiDb(adapter) => adapter.rename_nx_in_db(db_index, old_key, new_key),
            #[cfg(feature = "cluster")]
            StorageEngine::ClusterRaft(c) => c.rename_nx_in_db(db_index, old_key, new_key),
        }
    }

    /// Copy a key
    pub fn copy_in_db(
        &self,
        src_db: usize,
        dst_db: usize,
        src_key: &str,
        dst_key: &str,
        replace: bool,
    ) -> Result<bool> {
        match self {
            StorageEngine::Memory(adapter) => {
                adapter.copy_in_db(src_db, dst_db, src_key, dst_key, replace)
            }
            StorageEngine::AiDb(adapter) => {
                adapter.copy_in_db(src_db, dst_db, src_key, dst_key, replace)
            }
            #[cfg(feature = "cluster")]
            StorageEngine::ClusterRaft(c) => {
                c.copy_in_db(src_db, dst_db, src_key, dst_key, replace)
            }
        }
    }

    /// Get a random key from a database
    pub fn random_key_in_db(&self, db_index: usize) -> Result<Option<String>> {
        match self {
            StorageEngine::Memory(adapter) => adapter.random_key_in_db(db_index),
            StorageEngine::AiDb(adapter) => adapter.random_key_in_db(db_index),
            #[cfg(feature = "cluster")]
            StorageEngine::ClusterRaft(c) => c.random_key_in_db(db_index),
        }
    }

    /// Export all databases as StoredValue maps (for persistence)
    pub fn export_all_databases(&self) -> Result<Vec<HashMap<String, StoredValue>>> {
        match self {
            StorageEngine::Memory(adapter) => adapter.export_all_databases(),
            StorageEngine::AiDb(adapter) => adapter.export_all_databases(),
            #[cfg(feature = "cluster")]
            StorageEngine::ClusterRaft(c) => c.export_all_databases(),
        }
    }

    /// Get AiDb storage statistics (MemTable, WAL, Block Cache).
    ///
    /// Returns aggregated statistics from all AiDb instances:
    /// - memtable_bytes: Total size of all MemTables
    /// - wal_bytes: Total WAL file size
    /// - block_cache_bytes: Current block cache usage
    /// - block_cache_capacity_bytes: Block cache capacity
    ///
    /// For Memory storage, returns (0, 0, 0, 0).
    pub fn get_aidb_stats(&self) -> (u64, u64, u64, u64) {
        match self {
            StorageEngine::Memory(_) => (0, 0, 0, 0),
            StorageEngine::AiDb(adapter) => adapter.get_storage_stats(),
            #[cfg(feature = "cluster")]
            StorageEngine::ClusterRaft(c) => c.get_aidb_stats(),
        }
    }

    /// Create a backup of the underlying storage.
    ///
    /// For AiDb: flushes MemTable and copies SSTable + WAL files to `backup_dir`.
    /// For ClusterRaft: backs up all local Raft group databases.
    /// Memory engine does not support backup and returns an error.
    pub fn create_backup(&self, backup_dir: &std::path::Path) -> Result<Vec<String>> {
        match self {
            StorageEngine::Memory(_) => Err(crate::error::AikvError::Storage(
                "Memory engine does not support SAVE (no persistent data)".to_string(),
            )),
            StorageEngine::AiDb(adapter) => adapter.create_backup(backup_dir),
            #[cfg(feature = "cluster")]
            StorageEngine::ClusterRaft(c) => c.create_backup(backup_dir),
        }
    }

    /// Check if this is an AiDb storage engine.
    pub fn is_aidb(&self) -> bool {
        matches!(self, StorageEngine::AiDb(_))
    }
}
