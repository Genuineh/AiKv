//! `write_batch` 成功/失败后的键计数与 [`ExpireDecrGate`] 处理.

use crate::storage::adapter::{StorageAdapter, WriteBatchStats};
use crate::storage::types::{DbKeyCounters, ExpireDecrGate, StoredValue};

/// 成功路径: Put 放开门闩并按引擎统计增减计数; Delete 对全部 op `try_claim`.
pub(crate) fn apply_successful_batch(
    counters: &DbKeyCounters,
    gate: &ExpireDecrGate,
    db: usize,
    put_keys: &[Vec<u8>],
    deleted_keys: &[Vec<u8>],
    stats: WriteBatchStats,
) {
    for encoded in put_keys {
        gate.release(encoded);
    }
    for _ in 0..stats.inserted {
        counters.incr(db);
    }
    for encoded in deleted_keys {
        let _ = gate.try_claim(encoded);
    }
    for _ in 0..stats.deleted {
        counters.decr(db);
    }
}

/// rebuild 之后, 仅当 encoded key 仍是未过期 `StoredValue` 时 `release`.
/// Put 未落盘且 SST 仍有过期值时保持门闩, 避免随后 compaction 再 `decr`.
pub(crate) async fn release_live_puts(
    storage: &dyn StorageAdapter,
    gate: &ExpireDecrGate,
    put_keys: &[Vec<u8>],
) {
    for encoded in put_keys {
        let Ok(Some(raw)) = storage.get(encoded.clone()).await else {
            continue;
        };
        let Ok(stored) = postcard::from_bytes::<StoredValue>(&raw) else {
            continue;
        };
        if !stored.is_expired() {
            gate.release(encoded);
        }
    }
}
