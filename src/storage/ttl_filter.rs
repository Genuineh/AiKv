//! TTL 过期 key 的 compaction 过滤器.
//!
//! 在 compaction 合并 SST 时, 尝试反序列化 value 为 `StoredValue`,
//! 检查 `is_expired()`, 对已过期的 entry 返回 `Remove` 以丢弃.

use crate::storage::types::StoredValue;
use aidb::engine::compaction::{CompactionFilter, FilterDecision};

/// TTL 过期的 key-value entry 在 compaction 时自动丢弃.
///
/// 仅对能反序列化为 `StoredValue` 格式的 entry 做过期判断.
/// 反序列化失败 (系统 key / subkey entry / 损毁数据) 时保守保留.
pub struct TtlExpireFilter;

impl CompactionFilter for TtlExpireFilter {
    fn filter(&self, _level: usize, _key: &[u8], value: &[u8]) -> FilterDecision {
        let Ok(stored) = postcard::from_bytes::<StoredValue>(value) else {
            // 非 StoredValue 格式 — 可能是 meta key / subkey entry / 损毁数据,
            // 保守保留.
            return FilterDecision::Keep;
        };
        if stored.is_expired() {
            FilterDecision::Remove
        } else {
            FilterDecision::Keep
        }
    }
}
