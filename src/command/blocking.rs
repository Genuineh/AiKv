//! 阻塞命令基础设施 (BLPOP/BRPOP/BLMOVE/BZPOPMIN/BZPOPMAX)
//!
//! key 上无等待者时 notify 开销为一次 DashMap 查找。

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use tokio::sync::oneshot;

use crate::protocol::RespValue;

/// 阻塞命令类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(dead_code)]
pub enum BlockingCmd {
    Blpop,
    Brpop,
    Blmove,
    Bzpopmin,
    Bzpopmax,
}

/// 挂起的阻塞请求
pub struct PendingRequest {
    pub sender: oneshot::Sender<RespValue>,
    /// 过期时间 (被 evict_expired 读取)
    #[allow(dead_code)]
    pub deadline: Instant,
}

/// 全局阻塞注册表 — key → 挂起请求列表
pub struct BlockingRegistry {
    /// key → pending blocking requests
    waiters: DashMap<Vec<u8>, Vec<PendingRequest>>,
}

impl BlockingRegistry {
    pub fn new() -> Self {
        Self {
            waiters: DashMap::new(),
        }
    }

    /// 注册阻塞等待。返回 oneshot::Receiver。
    pub fn register(&self, key: Vec<u8>, timeout: Duration) -> oneshot::Receiver<RespValue> {
        let (tx, rx) = oneshot::channel();
        let pending = PendingRequest {
            sender: tx,
            deadline: Instant::now() + timeout,
        };
        self.waiters.entry(key).or_default().push(pending);
        rx
    }

    /// 通知等待某个 key 的所有阻塞请求
    pub fn notify(&self, key: &[u8], response: RespValue) {
        let mut entry = match self.waiters.get_mut(key) {
            Some(e) => e,
            None => return,
        };
        let pending = std::mem::take(&mut *entry);
        for req in pending {
            let _ = req.sender.send(response.clone());
        }
    }

    /// 清理过期请求 (由后台定时器调用)
    #[allow(dead_code)]
    pub fn evict_expired(&self) {
        let now = Instant::now();
        self.waiters.retain(|_, waiters| {
            waiters.retain(|r| r.deadline > now);
            !waiters.is_empty()
        });
    }

    /// 全局单例
    pub fn global() -> &'static BlockingRegistry {
        static REGISTRY: OnceLock<BlockingRegistry> = OnceLock::new();
        REGISTRY.get_or_init(BlockingRegistry::new)
    }
}

/// Redis 阻塞超时返回 nil Array (not nil bulk)
pub fn nil_blocking_response() -> RespValue {
    RespValue::Array(None)
}
