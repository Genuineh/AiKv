//! 本节点当前被 WATCH 的 key 引用计数.
//!
//! 热路径 SET 只在 `is_watched` 为 true 时才写 watch meta (`#83`).
//! 版本仍落在存储层; 本表只回答「有没有连接在看」, 不是 #79 的版本源.

use std::sync::Arc;

use dashmap::DashMap;

#[derive(Debug, Default)]
pub struct WatchRegistry {
    counts: DashMap<(usize, Vec<u8>), usize>,
}

impl WatchRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn watch(&self, db: usize, key: &[u8]) {
        self.counts
            .entry((db, key.to_vec()))
            .and_modify(|c| *c += 1)
            .or_insert(1);
    }

    pub fn unwatch(&self, db: usize, key: &[u8]) {
        use dashmap::mapref::entry::Entry;
        match self.counts.entry((db, key.to_vec())) {
            Entry::Occupied(mut occ) => {
                *occ.get_mut() = occ.get().saturating_sub(1);
                if *occ.get() == 0 {
                    occ.remove();
                }
            }
            Entry::Vacant(_) => {}
        }
    }

    pub fn is_watched(&self, db: usize, key: &[u8]) -> bool {
        self.counts.get(&(db, key.to_vec())).is_some_and(|c| *c > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refcount_and_is_watched() {
        let reg = WatchRegistry::new();
        assert!(!reg.is_watched(0, b"k"));
        reg.watch(0, b"k");
        reg.watch(0, b"k");
        assert!(reg.is_watched(0, b"k"));
        reg.unwatch(0, b"k");
        assert!(reg.is_watched(0, b"k"));
        reg.unwatch(0, b"k");
        assert!(!reg.is_watched(0, b"k"));
        assert!(!reg.is_watched(1, b"k"));
    }
}
