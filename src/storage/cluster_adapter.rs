//! 集群数据面存储适配器.
//!
//! 在 cluster 模式下, 用户数据的读写必须经过数据面 Raft group 才能在副本间
//! 复制, 从而保证 primary failover 后新 leader 能读到旧数据.
//!
//! 本适配器包裹本地引擎 (`StorageAdapter`), 当 `CLUSTER_STATE_MGR` 已初始化且
//! key 路由到一个已分配 slot 的 group 时:
//! - 写 (`set`/`delete`/`write_batch`): 通过 `propose_group` 提交到对应 group 的
//!   Raft, 由 Raft 复制并 apply 到各节点的 group 状态机.
//! - 读 (`get`/`exists`/`scan_prefix`): 直接读本地 group 状态机.
//!
//! 当集群未初始化 (单机模式) 或 slot 未分配时, 回退到本地引擎.
//! 已分配 slot 的数据 **禁止** 写本地引擎 fallback, 避免 SET 成功但 GET 读 Raft 为空.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use tokio::sync::{mpsc, oneshot};

use aidb::cluster::router::key_to_slot;
use aidb::cluster::{
    MigOp, Request, Response, ShardedStorage, SlotMigrationState, SlotStatus, ThinWriteBatch,
    ThinWriteOp, WriteBatchResponse, WriteOpResult,
};
use aidb::error::ClusterError as AidbClusterError;
use aidb::Checkpoint;

use crate::cluster::router::{
    migration_phase_for_slot, MigrationRoutePhase, TRYAGAIN_MIGRATION,
};
use crate::cluster::state::{ClusterStateManager, CLUSTER_STATE_MGR};
use crate::error::{Error, Result};
use crate::storage::adapter::{AdapterWriteOp, StorageAdapter};
use crate::storage::aidb::AiDbEngine;
use crate::storage::types::StorageEngineKind;

const GROUP_READY_MAX_ATTEMPTS: u32 = 20;
const GROUP_READY_RETRY_DELAY: Duration = Duration::from_millis(250);
const ERR_DATA_GROUP_NOT_READY: &str = "CLUSTERDOWN data group not ready";
const SET_BATCH_MAX_OPS: usize = 512;
/// 凑批等待上限. 1ms 平衡吞吐与延迟: 集群 50c 负载下 items 到达快, 等 1ms 足矣.
const SET_BATCH_MAX_DELAY: Duration = Duration::from_millis(1);

/// F-056-A1 读路由结果: `Single` 为老路径 (本地 group 直读, 无需合并);
/// `Merge` 为 Frozen/Copying 期合并读, 需依次查 target tombstone → target
/// 值 → source 值 (见 `ClusterDataAdapter::merge_read`).
enum ReadRoute {
    Single(u64),
    Merge {
        target_group: u64,
        source_group: u64,
        epoch: u64,
    },
}

/// F-056-A1 写路由结果: `Plain` 为老路径 (`Request::Put`/`Delete`/`WriteBatch`);
/// `Migration` 为 Copying (Prepare/Migrating) 期写, 必须走
/// `Request::MigrationWrite` 记录 tombstone (关闭"不存在则跳过 propose"短路).
enum WriteRoute {
    Plain(u64),
    Migration {
        gid: u64,
        source_group: u64,
        epoch: u64,
    },
}

/// 数据面感知的存储适配器.
pub struct ClusterDataAdapter {
    local: Arc<dyn StorageAdapter>,
    set_batchers: Mutex<HashMap<u64, Arc<GroupSetBatcher>>>,
    eager_flush: usize,
}

struct GroupSetBatcher {
    tx: mpsc::Sender<WriteBatchItem>,
}

struct WriteBatchItem {
    op: ThinWriteOp,
    /// ack 时返回此 op 对应的 WriteOpResult
    ack: oneshot::Sender<std::result::Result<WriteOpResult, String>>,
}

impl ClusterDataAdapter {
    /// `eager_flush` 的默认值.
    /// 值 = `SET_BATCH_MAX_OPS / 10.67`, 保持与原始 128/12 相同的比例.
    pub const DEFAULT_EAGER_FLUSH: usize = 48;

    pub fn new(local: Arc<dyn StorageAdapter>, eager_flush: usize) -> Arc<Self> {
        Arc::new(Self {
            local,
            set_batchers: Mutex::new(HashMap::new()),
            eager_flush,
        })
    }

    /// 本节点作为迁槽目标时的本地 data group (IMPORTING / MIGRATE RESTORE).
    fn local_migration_target_group(mgr: &ClusterStateManager) -> Option<u64> {
        if let Some(state) = mgr.meta_raft.get_migration_state() {
            let target = match &state {
                SlotMigrationState::Prepare { target_group, .. }
                | SlotMigrationState::Migrating { target_group, .. }
                | SlotMigrationState::Frozen { target_group, .. }
                | SlotMigrationState::ReadyToCommit { target_group, .. } => *target_group,
            };
            // 仅当 target group 确实在本节点 self.groups 中时才使用.
            if mgr.multi_raft.get_groups().read().contains_key(&target) {
                return Some(target);
            }
        }
        // 回退: 使用本节点的任意本地 group.
        mgr.multi_raft.local_group_ids().into_iter().next()
    }

    /// IMPORTING 窗口内写操作应落到本节点承载的目标 data group, 而非 router 仍显示的源 group.
    /// Frozen / ReadyToCommit 写冻结由 `write_frozen_err` 拦截, 此处不放行.
    fn importing_write_group(mgr: &ClusterStateManager, slot: u16) -> Option<u64> {
        if !mgr.importing_slots.read().contains_key(&slot) {
            return None;
        }
        if let Some(phase) = migration_phase_for_slot(mgr, slot) {
            if phase.writes_frozen() {
                return None;
            }
        }
        Self::local_migration_target_group(mgr)
    }

    fn write_frozen_err() -> Error {
        Error::Command(TRYAGAIN_MIGRATION.into())
    }

    /// F-056: Frozen/Ready 写冻结 — 与 `ClusterRouter::decide` 共用相位.
    fn reject_if_write_frozen(mgr: &ClusterStateManager, routing_key: &[u8]) -> Result<()> {
        let slot = key_to_slot(routing_key);
        if let Some(phase) = migration_phase_for_slot(mgr, slot) {
            if phase.writes_frozen() {
                return Err(Self::write_frozen_err());
            }
        }
        Ok(())
    }

    /// F-056-A1 读路由: ReadyToCommit → 纯 target (仅本地时走快路径, 语义不变);
    /// Frozen/Copying → 合并读 (`ReadRoute::Merge`), 不要求 target/source 本地
    /// —— `merge_read` 内部按需经 `get_key_from_group_remote` 远程读取.
    fn migration_read_route(mgr: &ClusterStateManager, slot: u16) -> Result<Option<ReadRoute>> {
        let Some(phase) = migration_phase_for_slot(mgr, slot) else {
            return Ok(None);
        };
        match phase {
            MigrationRoutePhase::ReadyToCommit { target_group, .. } => {
                Ok(mgr
                    .multi_raft
                    .get_groups()
                    .read()
                    .contains_key(&target_group)
                    .then_some(ReadRoute::Single(target_group)))
            }
            MigrationRoutePhase::Frozen {
                source_group,
                target_group,
            }
            | MigrationRoutePhase::Copying {
                source_group,
                target_group,
            } => {
                let epoch = mgr
                    .migration_epoch()
                    .ok_or_else(|| Error::Command(TRYAGAIN_MIGRATION.into()))?;
                Ok(Some(ReadRoute::Merge {
                    target_group,
                    source_group,
                    epoch,
                }))
            }
        }
    }

    /// FIX-0056-A1 合并读线性点 (算法/合并读): 先查 target 上该 epoch 内 key 的
    /// tombstone —— 最后一次是 Del 则直接返回 None (禁止 source 复活); 否则读
    /// target 值, 命中即返回 (target-wins); target miss 时才回落 source (数据
    /// 尚未拷贝到 target 的情形). 任一步 RPC 失败/超时都必须返回 TRYAGAIN,
    /// 禁止静默 fallback 到可能陈旧的本地视图 (读导向 点 3).
    async fn merge_read(
        mgr: &ClusterStateManager,
        target_group: u64,
        source_group: u64,
        epoch: u64,
        key: &[u8],
    ) -> Result<Option<Vec<u8>>> {
        let tombstone = mgr
            .multi_raft
            .get_migration_tombstone_remote(target_group, epoch, key)
            .await
            .map_err(Self::merge_read_err)?;
        if matches!(tombstone, Some(MigOp::Del)) {
            return Ok(None);
        }
        if let Some(v) = mgr
            .multi_raft
            .get_key_from_group_remote(target_group, key)
            .await
            .map_err(Self::merge_read_err)?
        {
            return Ok(Some(v));
        }
        mgr.multi_raft
            .get_key_from_group_remote(source_group, key)
            .await
            .map_err(Self::merge_read_err)
    }

    fn merge_read_err<E: std::fmt::Display>(e: E) -> Error {
        tracing::warn!(error = %e, "merge_read failed, mapping to TRYAGAIN");
        Error::Command(TRYAGAIN_MIGRATION.into())
    }

    /// 若 `slot` 处于 Copying (Prepare/Migrating), 返回 `(source_group, epoch)`
    /// 供写路径构造 `Request::MigrationWrite`. Frozen/ReadyToCommit 写已在
    /// `reject_if_write_frozen` 拦截, 不会到达这里. epoch 缺失属于不应发生的
    /// 内部不一致 (`BeginSlotMigration` 与 `migration_state` 同一 apply 内落地),
    /// 保守返回 TRYAGAIN 而非静默退化为非迁移写 (会漏记 tombstone).
    fn copying_migration_route(
        mgr: &ClusterStateManager,
        slot: u16,
    ) -> Result<Option<(u64, u64)>> {
        match migration_phase_for_slot(mgr, slot) {
            Some(MigrationRoutePhase::Copying { source_group, .. }) => {
                let epoch = mgr
                    .migration_epoch()
                    .ok_or_else(|| Error::Command(TRYAGAIN_MIGRATION.into()))?;
                Ok(Some((source_group, epoch)))
            }
            _ => Ok(None),
        }
    }

    /// 剥离子键后缀 (`\x01H{...}` 或 `\x01S{...}`), 返回路由用的纯 user_key.
    /// Subkey 的 slot 必须与父 key 一致, 否则集群模式下会路由到错误的 group.
    fn strip_subkey_suffix(user_key: &[u8]) -> &[u8] {
        if let Some(pos) = user_key.iter().position(|&b| b == 0x01) {
            if pos + 1 < user_key.len()
                && (user_key[pos + 1] == b'H' || user_key[pos + 1] == b'S')
            {
                return &user_key[..pos];
            }
        }
        user_key
    }

    /// 读路径: 按 router 已分配 slot 路由到本地 group, 或 F-056-A1 合并读.
    /// ReadyToCommit 读切 target (纯 target, 仅本地时走快路径); Frozen/Copying
    /// 读为合并读 (可能涉及远程 RPC, 由调用方 `merge_read` 处理).
    fn route_read(key: &[u8]) -> Result<Option<(Arc<ClusterStateManager>, ReadRoute)>> {
        let Some(mgr) = CLUSTER_STATE_MGR.get().cloned() else {
            return Ok(None);
        };
        let Some((_, user_key)) = AiDbEngine::decode_key(key) else {
            return Ok(None);
        };
        let routing_key = Self::strip_subkey_suffix(&user_key);
        let slot = key_to_slot(routing_key);
        if let Some(route) = Self::migration_read_route(&mgr, slot)? {
            return Ok(Some((mgr, route)));
        }
        Ok(match mgr.router.route_key(routing_key) {
            Ok((gid, SlotStatus::Assigned(_))) => Some((mgr, ReadRoute::Single(gid))),
            Ok((gid, SlotStatus::Migrating(_)))
                if mgr.multi_raft.get_groups().read().contains_key(&gid) =>
            {
                Some((mgr, ReadRoute::Single(gid)))
            }
            _ => None,
        })
    }

    /// 写路径: IMPORTING 时优先写入本节点目标 group.
    /// 返回 `Err(TRYAGAIN)` 当 Frozen/Ready 写冻结, 或 Copying 期 epoch 缺失;
    /// `Ok(None)` 表示无本地 group.
    fn route_write(key: &[u8]) -> Result<Option<(Arc<ClusterStateManager>, WriteRoute)>> {
        let Some(mgr) = CLUSTER_STATE_MGR.get().cloned() else {
            return Ok(None);
        };
        let Some((_, user_key)) = AiDbEngine::decode_key(key) else {
            return Ok(None);
        };
        let routing_key = Self::strip_subkey_suffix(&user_key);
        Self::reject_if_write_frozen(&mgr, routing_key)?;
        let slot = key_to_slot(routing_key);
        let migration = Self::copying_migration_route(&mgr, slot)?;
        let gid = if let Some(gid) = Self::importing_write_group(&mgr, slot) {
            Some(gid)
        } else {
            match mgr.router.route_key(routing_key) {
                Ok((gid, SlotStatus::Assigned(_))) => {
                    if mgr.multi_raft.get_groups().read().contains_key(&gid) {
                        Some(gid)
                    } else {
                        // decide() 已在 IMPORTING 窗口放行本地写, 但 router 仍指向源 group.
                        Self::local_migration_target_group(&mgr)
                    }
                }
                Ok((gid, SlotStatus::Migrating(_))) => {
                    if mgr.multi_raft.get_groups().read().contains_key(&gid) {
                        Some(gid)
                    } else {
                        Self::local_migration_target_group(&mgr)
                    }
                }
                _ => None,
            }
        };
        Ok(gid.map(|gid| {
            let route = match migration {
                Some((source_group, epoch)) => WriteRoute::Migration {
                    gid,
                    source_group,
                    epoch,
                },
                None => WriteRoute::Plain(gid),
            };
            (mgr, route)
        }))
    }

    fn map_err<E: std::fmt::Display>(e: E) -> Error {
        Error::Cluster(crate::error::ClusterError::Aidb(
            AidbClusterError::Internal(e.to_string()),
        ))
    }

    fn check_response(resp: Response) -> Result<()> {
        match resp {
            Response::Error(msg) => Err(Error::Cluster(crate::error::ClusterError::Aidb(
                AidbClusterError::Internal(msg),
            ))),
            _ => Ok(()),
        }
    }

    fn data_group_not_ready_err() -> Error {
        Error::Cluster(crate::error::ClusterError::Aidb(
            AidbClusterError::InvalidState(ERR_DATA_GROUP_NOT_READY.into()),
        ))
    }

    fn key_slot_status(key: &[u8]) -> Option<SlotStatus> {
        let mgr = CLUSTER_STATE_MGR.get()?;
        let (_, user_key) = AiDbEngine::decode_key(key)?;
        mgr.router
            .route_key(&user_key)
            .ok()
            .map(|(_, status)| status)
    }

    /// 仅单机或未分配 slot 时允许写本地引擎; 已分配 slot 必须走 Raft.
    fn should_use_local_engine(key: &[u8]) -> bool {
        match Self::key_slot_status(key) {
            None => true,
            Some(SlotStatus::Unallocated) => true,
            Some(_) => false,
        }
    }

    fn is_transient_cluster_err(err: &Error) -> bool {
        match err {
            Error::Cluster(crate::error::ClusterError::Aidb(ref e)) => {
                matches!(
                    e,
                    AidbClusterError::Raft(_)
                        | AidbClusterError::NotLeader { .. }
                        | AidbClusterError::Timeout(_)
                        | AidbClusterError::InvalidState(_)
                        | AidbClusterError::Internal(_)
                )
            }
            _ => false,
        }
    }

    async fn propose_group_with_retry(
        mgr: &ClusterStateManager,
        gid: u64,
        request: Request,
    ) -> Result<Response> {
        let mut last_err = Self::data_group_not_ready_err();
        for attempt in 0..GROUP_READY_MAX_ATTEMPTS {
            match mgr.multi_raft.propose_group(gid, request.clone()).await {
                Ok(resp) => return Ok(resp),
                Err(e) => {
                    last_err = Self::map_err(e);
                    if Self::is_transient_cluster_err(&last_err)
                        && attempt + 1 < GROUP_READY_MAX_ATTEMPTS
                    {
                        tokio::time::sleep(GROUP_READY_RETRY_DELAY).await;
                        continue;
                    }
                    return Err(last_err);
                }
            }
        }
        Err(last_err)
    }

    fn get_or_spawn_set_batcher(
        &self,
        mgr: Arc<ClusterStateManager>,
        gid: u64,
    ) -> Arc<GroupSetBatcher> {
        let mut batchers = self.set_batchers.lock();
        if let Some(existing) = batchers.get(&gid) {
            return existing.clone();
        }

        let (tx, rx) = mpsc::channel(4096);
        let batcher = Arc::new(GroupSetBatcher { tx });
        tokio::spawn(run_set_batcher(mgr, gid, rx, self.eager_flush));
        batchers.insert(gid, batcher.clone());
        batcher
    }

    async fn submit_write_op(
        &self,
        mgr: Arc<ClusterStateManager>,
        gid: u64,
        op: ThinWriteOp,
    ) -> Result<WriteOpResult> {
        let batcher = self.get_or_spawn_set_batcher(mgr, gid);
        let (ack, wait) = oneshot::channel();
        batcher
            .tx
            .send(WriteBatchItem { op, ack })
            .await
            .map_err(|_| {
                Error::Cluster(crate::error::ClusterError::Aidb(
                    AidbClusterError::Internal("data group write batcher stopped".into()),
                ))
            })?;
        wait.await
            .map_err(|_| {
                Error::Cluster(crate::error::ClusterError::Aidb(
                    AidbClusterError::Internal("data group write batcher dropped response".into()),
                ))
            })?
            .map_err(|e| {
                Error::Cluster(crate::error::ClusterError::Aidb(
                    AidbClusterError::Internal(e.to_string()),
                ))
            })
    }
}

async fn run_set_batcher(
    mgr: Arc<ClusterStateManager>,
    gid: u64,
    mut rx: mpsc::Receiver<WriteBatchItem>,
    eager_flush: usize,
) {
    while let Some(first) = rx.recv().await {

        let mut items = Vec::with_capacity(SET_BATCH_MAX_OPS);
        items.push(first);

        while items.len() < SET_BATCH_MAX_OPS {
            while items.len() < SET_BATCH_MAX_OPS {
                match rx.try_recv() {
                    Ok(item) => items.push(item),
                    Err(mpsc::error::TryRecvError::Empty) => break,
                    Err(mpsc::error::TryRecvError::Disconnected) => break,
                }
            }
            if items.len() >= SET_BATCH_MAX_OPS || items.len() >= eager_flush {
                break;
            }
            match tokio::time::timeout(SET_BATCH_MAX_DELAY, rx.recv()).await {
                Ok(Some(item)) => items.push(item),
                Ok(None) | Err(_) => break,
            }
        }

        let mut tb = ThinWriteBatch::new();
        let mut ack_indices: Vec<(usize, oneshot::Sender<std::result::Result<WriteOpResult, String>>)> = Vec::with_capacity(items.len());
        let mut seen: HashSet<Vec<u8>> = HashSet::new();

        let mut op_idx = 0usize;
        for item in items.into_iter().rev() {
            match &item.op {
                ThinWriteOp::Put { key, value } => {
                    if !seen.insert(key.clone()) {
                        // 重复 key: 已由后面的操作覆盖, 直接回复
                        let _ = item.ack.send(Ok(WriteOpResult::Ok));
                        continue;
                    }
                    tb.put(key.clone(), value.clone());
                    ack_indices.push((op_idx, item.ack));
                    op_idx += 1;
                }
                ThinWriteOp::Delete { key } => {
                    if !seen.insert(key.clone()) {
                        // 重复 key: 已由后面的操作覆盖
                        let _ = item.ack.send(Ok(WriteOpResult::Deleted));
                        continue;
                    }
                    tb.delete(key.clone());
                    ack_indices.push((op_idx, item.ack));
                    op_idx += 1;
                }
            }
        }
        ack_indices.reverse();

        // #region debug-agent log
        static BATCH_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let batch_sz = op_idx;
        let t_propose = std::time::Instant::now();
        // #endregion
        let result = ClusterDataAdapter::propose_group_with_retry(&mgr, gid, Request::WriteBatch(tb))
            .await;
        // #region debug-agent log
        let propose_us = t_propose.elapsed().as_micros();
        let seq = BATCH_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if seq % 200 == 0 || (seq < 100 && batch_sz > 0) {
            let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis();
            eprintln!(r#"{{"sessionId":"aa9275","hypothesisId":"C","location":"cluster_adapter.rs:batcher","message":"batch_propose","data":{{"seq":{},"gid":{},"batch_sz":{},"propose_us":{}}},"timestamp":{}}}"#, seq, gid, batch_sz, propose_us, ts);
        }
        // #endregion

        let per_op_results: Vec<std::result::Result<WriteOpResult, String>> = match result {
            Ok(Response::Value(Some(data))) => {
                match rmp_serde::from_slice::<WriteBatchResponse>(&data) {
                    Ok(resp) => resp.results.into_iter()
                        .map(Ok)
                        .collect(),
                    Err(e) => vec![Err(format!("decode WriteBatchResponse: {e}")); op_idx],
                }
            }
            Ok(other) => vec![Err(format!("unexpected response: {other:?}")); op_idx],
            Err(e) => vec![Err(e.to_string()); op_idx],
        };

        for (idx, ack) in ack_indices {
            let r = per_op_results.get(idx)
                .cloned()
                .unwrap_or(Err("missing result".into()));
            let _ = ack.send(r);
        }
    }
}

/// Checkpoint every local data-group DB under `dest/group_{id}/`.
pub(crate) fn checkpoint_group_storages(
    storages: &HashMap<u64, ShardedStorage>,
    dest: &Path,
) -> Result<PathBuf> {
    if storages.is_empty() {
        return Err(Error::Command(
            "ERR no local data groups to checkpoint".into(),
        ));
    }
    std::fs::create_dir_all(dest).map_err(|e| Error::Storage(e.to_string()))?;
    for (gid, storage) in storages {
        storage
            .db()
            .flush()
            .map_err(|e| Error::Storage(e.to_string()))?;
        let group_dest = dest.join(format!("group_{gid}"));
        Checkpoint::create(storage.db().as_ref(), &group_dest)
            .map_err(|e| Error::Storage(e.to_string()))?;
    }
    Ok(dest.to_path_buf())
}

fn flush_group_storages(storages: &HashMap<u64, ShardedStorage>) -> Result<()> {
    for storage in storages.values() {
        storage
            .db()
            .flush()
            .map_err(|e| Error::Storage(e.to_string()))?;
    }
    Ok(())
}

#[async_trait]
impl StorageAdapter for ClusterDataAdapter {
    async fn get(&self, key: Vec<u8>) -> Result<Option<Vec<u8>>> {
        match Self::route_read(&key)? {
            Some((mgr, ReadRoute::Single(gid))) => {
                mgr.multi_raft.get_local(gid, &key).await.map_err(|e| {
                    tracing::warn!(gid = gid, error = %e, "get_local failed");
                    Self::map_err(e)
                })
            }
            Some((
                mgr,
                ReadRoute::Merge {
                    target_group,
                    source_group,
                    epoch,
                },
            )) => Self::merge_read(&mgr, target_group, source_group, epoch, &key).await,
            None if Self::should_use_local_engine(&key) => self.local.get(key).await,
            None => Err(Self::data_group_not_ready_err()),
        }
    }

    async fn set(&self, key: Vec<u8>, value: Vec<u8>) -> Result<()> {
        match Self::route_write(&key)? {
            Some((mgr, WriteRoute::Migration { gid, epoch, .. })) => {
                let mut ops = ThinWriteBatch::new();
                ops.put(key, value);
                let resp =
                    Self::propose_group_with_retry(&mgr, gid, Request::MigrationWrite { epoch, ops })
                        .await?;
                Self::check_response(resp)
            }
            Some((mgr, WriteRoute::Plain(gid))) => {
                self.submit_write_op(mgr, gid, ThinWriteOp::Put { key, value }).await?;
                Ok(())
            }
            None if Self::should_use_local_engine(&key) => self.local.set(key, value).await,
            None => Err(Self::data_group_not_ready_err()),
        }
    }

    async fn delete(&self, key: Vec<u8>) -> Result<bool> {
        match Self::route_write(&key)? {
            Some((
                mgr,
                WriteRoute::Migration {
                    gid,
                    source_group,
                    epoch,
                },
            )) => {
                // FIX-0056-A1: 迁移期一律 propose (禁止"不存在则跳过"短路),
                // existed 仅用于 DEL 返回值, 经合并读判断 (target 或 source 命中即算存在).
                let existed = Self::merge_read(&mgr, gid, source_group, epoch, &key)
                    .await?
                    .is_some();
                let mut ops = ThinWriteBatch::new();
                ops.delete(key);
                let resp =
                    Self::propose_group_with_retry(&mgr, gid, Request::MigrationWrite { epoch, ops })
                        .await?;
                Self::check_response(resp)?;
                Ok(existed)
            }
            Some((mgr, WriteRoute::Plain(gid))) => {
                let result = self.submit_write_op(mgr, gid, ThinWriteOp::Delete { key }).await?;
                Ok(result == WriteOpResult::Deleted)
            }
            None if Self::should_use_local_engine(&key) => self.local.delete(key).await,
            None => Err(Self::data_group_not_ready_err()),
        }
    }

    async fn exists(&self, key: Vec<u8>) -> Result<bool> {
        match Self::route_read(&key)? {
            Some((mgr, ReadRoute::Single(gid))) => {
                let v = mgr.multi_raft.get_local(gid, &key).await.map_err(|e| {
                    tracing::warn!(gid = gid, error = %e, "get_local failed in exists");
                    Self::map_err(e)
                })?;
                Ok(v.is_some())
            }
            Some((
                mgr,
                ReadRoute::Merge {
                    target_group,
                    source_group,
                    epoch,
                },
            )) => Ok(Self::merge_read(&mgr, target_group, source_group, epoch, &key)
                .await?
                .is_some()),
            None if Self::should_use_local_engine(&key) => self.local.exists(key).await,
            None => Err(Self::data_group_not_ready_err()),
        }
    }

    async fn write_batch(&self, batch: Vec<AdapterWriteOp>) -> Result<()> {
        // 按 group 分组后逐组提交, 避免跨 group 的单次提案.
        // F-056-A1: Copying 期写额外按 (gid, epoch) 分组走 MigrationWrite.
        let mut plain_by_group: HashMap<u64, (Arc<ClusterStateManager>, ThinWriteBatch)> =
            HashMap::new();
        let mut migration_by_group: HashMap<u64, (Arc<ClusterStateManager>, u64, ThinWriteBatch)> =
            HashMap::new();
        let mut local_ops: Vec<AdapterWriteOp> = Vec::new();
        for op in batch {
            let key = match &op {
                AdapterWriteOp::Put { key, .. } => key.as_slice(),
                AdapterWriteOp::Delete { key } => key.as_slice(),
            };
            match Self::route_write(key)? {
                Some((mgr, WriteRoute::Migration { gid, epoch, .. })) => {
                    let entry = migration_by_group
                        .entry(gid)
                        .or_insert_with(|| (mgr, epoch, ThinWriteBatch::new()));
                    match op {
                        AdapterWriteOp::Put { key, value } => entry.2.put(key, value),
                        AdapterWriteOp::Delete { key } => entry.2.delete(key),
                    }
                }
                Some((mgr, WriteRoute::Plain(gid))) => {
                    let entry = plain_by_group
                        .entry(gid)
                        .or_insert_with(|| (mgr, ThinWriteBatch::new()));
                    match op {
                        AdapterWriteOp::Put { key, value } => entry.1.put(key, value),
                        AdapterWriteOp::Delete { key } => entry.1.delete(key),
                    }
                }
                None if Self::should_use_local_engine(key) => local_ops.push(op),
                None => return Err(Self::data_group_not_ready_err()),
            }
        }
        for (gid, (mgr, tb)) in plain_by_group {
            let resp = Self::propose_group_with_retry(&mgr, gid, Request::WriteBatch(tb)).await?;
            Self::check_response(resp)?;
        }
        for (gid, (mgr, epoch, ops)) in migration_by_group {
            let resp =
                Self::propose_group_with_retry(&mgr, gid, Request::MigrationWrite { epoch, ops })
                    .await?;
            Self::check_response(resp)?;
        }
        if !local_ops.is_empty() {
            self.local.write_batch(local_ops).await?;
        }
        Ok(())
    }

    async fn for_each_prefix(
        &self,
        prefix: Vec<u8>,
        mut f: Box<dyn FnMut(Vec<u8>, Vec<u8>) -> Result<()> + Send>,
    ) -> Result<()> {
        match CLUSTER_STATE_MGR.get() {
            Some(mgr) => {
                for gid in mgr.multi_raft.local_group_ids() {
                    let pairs = mgr
                        .multi_raft
                        .scan_local_pairs(gid)
                        .await
                        .map_err(Self::map_err)?;
                    for (k, v) in pairs {
                        if k.starts_with(&prefix) {
                            f(k, v)?;
                        }
                    }
                }
                Ok(())
            }
            None => self.local.for_each_prefix(prefix, f).await,
        }
    }

    async fn delete_range(&self, start: Vec<u8>, end: Vec<u8>) -> Result<()> {
        match CLUSTER_STATE_MGR.get() {
            Some(mgr) => {
                for gid in mgr.multi_raft.local_group_ids() {
                    let pairs = mgr
                        .multi_raft
                        .scan_local_pairs(gid)
                        .await
                        .map_err(Self::map_err)?;
                    let mut tb = ThinWriteBatch::new();
                    for (k, _) in pairs {
                        if k.as_slice() >= start.as_slice() && k.as_slice() < end.as_slice() {
                            tb.delete(k);
                        }
                    }
                    if !tb.ops.is_empty() {
                        let resp = mgr
                            .multi_raft
                            .propose_group(gid, Request::WriteBatch(tb))
                            .await
                            .map_err(Self::map_err)?;
                        Self::check_response(resp)?;
                    }
                }
                Ok(())
            }
            None => self.local.delete_range(start, end).await,
        }
    }

    async fn len(&self) -> Result<usize> {
        match CLUSTER_STATE_MGR.get() {
            Some(mgr) => {
                let mut total = 0usize;
                for gid in mgr.multi_raft.local_group_ids() {
                    total += mgr
                        .multi_raft
                        .scan_local_pairs(gid)
                        .await
                        .map_err(Self::map_err)?
                        .len();
                }
                Ok(total)
            }
            None => self.local.len().await,
        }
    }

    async fn is_empty(&self) -> Result<bool> {
        Ok(self.len().await? == 0)
    }

    async fn clear(&self) -> Result<()> {
        match CLUSTER_STATE_MGR.get() {
            Some(mgr) => {
                for gid in mgr.multi_raft.local_group_ids() {
                    let pairs = mgr
                        .multi_raft
                        .scan_local_pairs(gid)
                        .await
                        .map_err(Self::map_err)?;
                    let mut tb = ThinWriteBatch::new();
                    for (k, _) in pairs {
                        tb.delete(k);
                    }
                    if !tb.ops.is_empty() {
                        let resp = mgr
                            .multi_raft
                            .propose_group(gid, Request::WriteBatch(tb))
                            .await
                            .map_err(Self::map_err)?;
                        Self::check_response(resp)?;
                    }
                }
                Ok(())
            }
            None => self.local.clear().await,
        }
    }

    async fn flush(&self) -> Result<()> {
        if let Some(mgr) = CLUSTER_STATE_MGR.get() {
            let storages = mgr.multi_raft.get_storages().read().clone();
            if !storages.is_empty() {
                let storages = storages;
                tokio::task::spawn_blocking(move || flush_group_storages(&storages))
                    .await
                    .map_err(|e| Error::Storage(e.to_string()))??;
            }
        }
        self.local.flush().await
    }

    async fn create_checkpoint(&self, dest: &Path) -> Result<PathBuf> {
        match CLUSTER_STATE_MGR.get() {
            Some(mgr) => {
                let storages = mgr.multi_raft.get_storages().read().clone();
                if storages.is_empty() {
                    return self.local.create_checkpoint(dest).await;
                }
                let dest = dest.to_path_buf();
                tokio::task::spawn_blocking(move || checkpoint_group_storages(&storages, &dest))
                    .await
                    .map_err(|e| Error::Storage(e.to_string()))?
            }
            None => self.local.create_checkpoint(dest).await,
        }
    }

    async fn close(&self) -> Result<()> {
        self.local.close().await
    }

    fn engine_kind(&self) -> StorageEngineKind {
        self.local.engine_kind()
    }

    fn approximate_memory_bytes(&self) -> Option<u64> {
        self.local.approximate_memory_bytes()
    }

    fn allow_lazy_expire_delete(&self, key: &[u8]) -> bool {
        match Self::route_read(key) {
            // 只有本节点是该 group 的 Raft leader 时, 惰性过期才值得发起 propose;
            // 副本上 propose 必然以 NotLeader 失败 (见 node.rs::propose 不做网络转发),
            // 交给 leader 自己的读路径或后续写连接去清理即可.
            Ok(Some((mgr, ReadRoute::Single(gid)))) => mgr.is_local_group_leader(gid),
            // 合并读期 (Frozen/Copying): 迁移期写一律落 target group, 惰性过期同理.
            Ok(Some((mgr, ReadRoute::Merge { target_group, .. }))) => {
                mgr.is_local_group_leader(target_group)
            }
            // 未路由到 data group (单机模式或 slot 未分配): 走本地引擎, 始终允许.
            Ok(None) => true,
            // 路由本身失败 (如迁移 epoch 缺失): 保守放弃, 交由下次读/写连接清理.
            Err(_) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aidb::config::Options;
    use tempfile::tempdir;

    #[test]
    fn checkpoint_group_storages_roundtrip() {
        let dir = tempdir().expect("tempdir");
        let data_dir = dir.path().join("data");
        let backup_dir = dir.path().join("backup");
        std::fs::create_dir_all(&data_dir).expect("data dir");

        let storage =
            ShardedStorage::open(&data_dir, 1, Options::for_testing()).expect("open group storage");
        storage.db().put(b"k1", b"v1").expect("put");
        storage.db().flush().expect("flush");

        let mut storages = HashMap::new();
        storages.insert(1, storage);

        let path = checkpoint_group_storages(&storages, &backup_dir).expect("checkpoint");
        assert_eq!(path, backup_dir);
        assert!(backup_dir.join("group_1").join("CURRENT").exists());

        let restored =
            aidb::DB::open(backup_dir.join("group_1"), Options::for_testing()).expect("restore");
        assert_eq!(restored.get(b"k1").expect("get"), Some(b"v1".to_vec()));
    }
}
