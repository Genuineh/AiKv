//! 存储抽象层 (Phase 9–10)

pub mod adapter;
pub mod aidb;
pub mod aidb_options;
#[cfg(feature = "cluster")]
pub mod cluster_adapter;
pub mod dump;
pub mod memory;
pub mod observation;
pub mod types;

pub use adapter::{AdapterWriteOp, KvStorageAdapter, StorageAdapter};
pub use aidb::AiDbEngine;
pub use aidb_options::{server_db_options, testing_db_options};
pub use dump::{decode as dump_decode, encode as dump_encode, DUMP_VERSION};
pub use memory::MemoryEngine;
pub use observation::StorageObservation;
pub use types::{
    is_wrongtype, now_ms, KeyspaceStats, KvStorage, ScanResult, StorageEngineKind, StoredValue,
    ValueType, WriteOp, TTL_NO_EXPIRY, WRONGTYPE,
};
