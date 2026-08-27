//! 存储层可观测性事件 (与 server metrics 解耦).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// 跨存储引擎共享的观测计数器.
#[derive(Debug, Default)]
pub struct StorageObservation {
    expired_keys: AtomicU64,
}

impl StorageObservation {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn record_expired_key(&self) {
        self.expired_keys.fetch_add(1, Ordering::Relaxed);
    }

    pub fn drain_expired_keys(&self) -> u64 {
        self.expired_keys.swap(0, Ordering::Relaxed)
    }
}
