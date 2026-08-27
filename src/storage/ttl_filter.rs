//! TTL 过期 key 的 compaction 过滤器与键计数移除监听.
//!
//! - [`TtlExpireFilter`]: 纯决策, 无副作用.
//! - [`DbKeyCounterRemovalListener`]: Version 安装后回调; 仅当 `get==None` 且
//!   [`ExpireDecrGate`] 赢得单飞时 `decr`.

use std::sync::{Arc, Weak};

use aidb::engine::compaction::{CompactionFilter, CompactionRemovalListener, FilterDecision};
use aidb::DB;

use crate::storage::types::{DbKeyCounters, ExpireDecrGate, StoredValue};

/// TTL 过期的 key-value entry 在 compaction 时自动丢弃.
pub struct TtlExpireFilter;

impl CompactionFilter for TtlExpireFilter {
    fn filter(&self, _level: usize, _key: &[u8], value: &[u8]) -> FilterDecision {
        let Ok(stored) = postcard::from_bytes::<StoredValue>(value) else {
            return FilterDecision::Keep;
        };
        if stored.is_expired() {
            FilterDecision::Remove
        } else {
            FilterDecision::Keep
        }
    }
}

/// Compaction Version 安装后, 对已从读视图消失的过期 key 扣减计数.
pub struct DbKeyCounterRemovalListener {
    counters: Arc<DbKeyCounters>,
    gate: Arc<ExpireDecrGate>,
    db: Weak<DB>,
}

impl DbKeyCounterRemovalListener {
    pub fn new(counters: Arc<DbKeyCounters>, gate: Arc<ExpireDecrGate>, db: Arc<DB>) -> Self {
        Self {
            counters,
            gate,
            db: Arc::downgrade(&db),
        }
    }
}

impl CompactionRemovalListener for DbKeyCounterRemovalListener {
    fn on_latest_put_removed(&self, user_key: &[u8]) {
        let payload = strip_optional_sm_prefix(user_key);
        if crate::storage::subkey::decode_subkey(payload).is_some() {
            return;
        }
        let Some(db_id) = logical_db_from_payload(payload) else {
            return;
        };
        let Some(db) = self.db.upgrade() else {
            return;
        };
        match db.get(user_key) {
            Ok(None) => {}
            Ok(Some(_)) => return,
            Err(e) => {
                tracing::warn!(error = %e, "compaction removal listener get failed");
                return;
            }
        }
        if !self.gate.try_claim(payload) {
            return;
        }
        self.counters.decr(db_id);
    }
}

fn logical_db_from_payload(payload: &[u8]) -> Option<usize> {
    let pos = payload.iter().position(|&b| b == b':')?;
    let db_str = std::str::from_utf8(&payload[..pos]).ok()?;
    db_str.parse().ok()
}

fn strip_optional_sm_prefix(raw: &[u8]) -> &[u8] {
    const HEAD: &[u8] = b"\x01sm/";
    if let Some(rest) = raw.strip_prefix(HEAD) {
        if rest.len() > 9 && rest[8] == b'/' {
            return &rest[9..];
        }
    }
    raw
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_logical_db_from_plain_and_sm_key() {
        assert_eq!(logical_db_from_payload(b"3:foo"), Some(3));
        let mut sm = b"\x01sm/".to_vec();
        sm.extend_from_slice(&7u64.to_be_bytes());
        sm.push(b'/');
        sm.extend_from_slice(b"2:bar");
        assert_eq!(
            logical_db_from_payload(strip_optional_sm_prefix(&sm)),
            Some(2)
        );
        assert_eq!(
            logical_db_from_payload(strip_optional_sm_prefix(b"\x00raft/nope")),
            None
        );
        assert_eq!(logical_db_from_payload(b"1:hash\x01Hfield"), Some(1));
    }
}
