use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::OnceLock;

use parking_lot::RwLock;

use crate::cluster::announce::AnnounceResolver;
use aidb::cluster::slot_migration::SlotMigrationManager;
use aidb::cluster::{MembershipCoordinator, MetaRaftNode, MultiRaftNode, Router};

/// 数据面端口偏移默认值 (与 Redis Cluster @cport 约定一致).
pub const DEFAULT_DATA_PORT_OFFSET: u16 = 10000;

/// 本节点角色
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplicationRole {
    Primary,
    Replica { primary_id: u64 },
}

/// 集群状态管理器 — 全局单例, 缓存路由信息供同步的 decide() 使用.
///
/// Router 是值类型但内部使用 Arc<RwLock<...>> (AiDb Router 实现了 Clone,
/// 克隆只拷贝内部 Arc 指针, 与 LifecycleManager 等共享同一份状态).
pub struct ClusterStateManager {
    pub router: Router,
    pub meta_raft: std::sync::Arc<MetaRaftNode>,
    pub multi_raft: std::sync::Arc<MultiRaftNode>,
    pub node_id: u64,
    pub config_epoch: AtomicU64,
    pub role: RwLock<ReplicationRole>,
    /// 本地 Group Leader 缓存 {group_id → is_leader}
    pub local_group_leaders: RwLock<HashMap<u64, bool>>,
    /// 成员协调器 (CLUSTER MEET/FORGET/REPLICATE 使用)
    pub membership_coordinator: Option<std::sync::Arc<MembershipCoordinator>>,
    /// 槽迁移管理器 (CLUSTER SETSLOT 使用)
    pub slot_migration_manager: Option<std::sync::Arc<SlotMigrationManager>>,
    /// 本地导入中的 slots (用于 `redis-cli --cluster check` 兼容).
    /// maps slot → source_node_id that initiated the import.
    pub importing_slots: parking_lot::RwLock<std::collections::HashMap<u16, u64>>,
    /// 数据目录 (用于 CONFIG REWRITE 持久化到 nodes.conf)
    pub data_dir: Option<std::path::PathBuf>,
    /// 数据面总线端口偏移.
    pub data_port_offset: u16,
    /// 客户端地址通告 (CLUSTER SLOTS / MOVED; 仅进程 env, 不持久化).
    pub announce_resolver: AnnounceResolver,
    /// Metrics (注入用于 gossip/failover 计数, 初始化时可 None).
    pub metrics: Option<std::sync::Arc<crate::server::metrics::ServerMetrics>>,
    /// Shutdown signal for LeaderChangeWatcher (held to keep sender alive).
    pub _watcher_shutdown: parking_lot::Mutex<Option<tokio::sync::watch::Sender<bool>>>,
    /// Shutdown signal for ConfigAutoSave (held to keep sender alive).
    pub _auto_save_shutdown: parking_lot::Mutex<Option<tokio::sync::watch::Sender<bool>>>,
}

impl ClusterStateManager {
    pub fn new(
        router: Router,
        meta_raft: std::sync::Arc<MetaRaftNode>,
        multi_raft: std::sync::Arc<MultiRaftNode>,
        node_id: u64,
    ) -> Self {
        let mgr = Self {
            router,
            meta_raft,
            multi_raft,
            node_id,
            config_epoch: AtomicU64::new(0),
            role: RwLock::new(ReplicationRole::Primary),
            local_group_leaders: RwLock::new(HashMap::new()),
            membership_coordinator: None,
            slot_migration_manager: None,
            data_dir: None,
            data_port_offset: DEFAULT_DATA_PORT_OFFSET,
            announce_resolver: AnnounceResolver::from_env(),
            importing_slots: parking_lot::RwLock::new(std::collections::HashMap::new()),
            metrics: None,
            _watcher_shutdown: parking_lot::Mutex::new(None),
            _auto_save_shutdown: parking_lot::Mutex::new(None),
        };
        mgr.refresh();
        mgr
    }

    /// 注入 MembershipCoordinator (在集群初始化完成后调用).
    pub fn set_membership_coordinator(&mut self, mc: std::sync::Arc<MembershipCoordinator>) {
        self.membership_coordinator = Some(mc);
    }

    /// 注入 SlotMigrationManager (在集群初始化完成后调用).
    pub fn set_slot_migration_manager(&mut self, sm: std::sync::Arc<SlotMigrationManager>) {
        self.slot_migration_manager = Some(sm);
    }

    /// 从 MetaRaft 刷新本地 Leader 缓存.
    pub fn refresh(&self) {
        let meta = self.meta_raft.get_cluster_meta();
        if meta.groups.is_empty() {
            return;
        }
        let mut leaders = self.local_group_leaders.write();
        leaders.clear();
        for (gid, group) in &meta.groups {
            let is_leader = group
                .replicas
                .iter()
                .any(|r| r.node_id == self.node_id && r.is_leader);
            leaders.insert(*gid, is_leader);
        }
    }

    pub fn is_local_group_leader(&self, group_id: u64) -> bool {
        if self.router.get_group_leader(group_id) == Some(self.node_id)
            && self.multi_raft.is_group_local(group_id)
        {
            return true;
        }
        self.local_group_leaders
            .read()
            .get(&group_id)
            .copied()
            .unwrap_or(false)
    }

    /// 获取当前迁移目标 group 的 leader 信息, 用于 ASK 重定向.
    ///
    /// 返回 `(target_group, leader_node_id, client_addr)` 或 None.
    /// 覆盖 Prepare / Migrating / Frozen / ReadyToCommit 全部活跃相位.
    pub fn migration_target_leader(&self) -> Option<(u64, u64, String)> {
        let mig_state = self.meta_raft.get_migration_state();
        let target_group = match &mig_state {
            Some(aidb::cluster::SlotMigrationState::Prepare { target_group, .. })
            | Some(aidb::cluster::SlotMigrationState::Migrating { target_group, .. })
            | Some(aidb::cluster::SlotMigrationState::Frozen { target_group, .. })
            | Some(aidb::cluster::SlotMigrationState::ReadyToCommit { target_group, .. }) => {
                *target_group
            }
            None => return None,
        };
        let leader = self.router.get_group_leader(target_group)?;
        let addr = self.announce_resolver.redirect_addr(leader, &self.router)?;
        Some((target_group, leader, addr))
    }

    /// FIX-0056-A1: 当前活跃迁移的 epoch (= `BeginSlotMigration` 时的
    /// `cluster_meta.version`), 用于 `Request::MigrationWrite` /
    /// `get_migration_tombstone_remote` 定位 target group 上的 oplog 前缀.
    /// 无活跃迁移时为 `None`.
    pub fn migration_epoch(&self) -> Option<u64> {
        self.meta_raft.get_migration_epoch()
    }

    /// 应用本地 MultiRaft 观测到的 group leader, 立即刷新路由缓存.
    pub fn apply_observed_group_leader(&self, group_id: u64, leader_id: u64) {
        self.router.update_group_leader(group_id, leader_id);
        let is_local = leader_id == self.node_id;
        self.local_group_leaders.write().insert(group_id, is_local);
    }
}

/// 全局集群状态管理器
pub static CLUSTER_STATE_MGR: OnceLock<std::sync::Arc<ClusterStateManager>> = OnceLock::new();
