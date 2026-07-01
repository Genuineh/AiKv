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

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use tokio::sync::{mpsc, oneshot};

use aidb::cluster::router::key_to_slot;
use aidb::cluster::{
    Request, Response, ShardedStorage, SlotMigrationState, SlotStatus, ThinWriteBatch,
};
use aidb::Checkpoint;

use crate::cluster::state::{ClusterStateManager, CLUSTER_STATE_MGR};
use crate::error::{Error, Result};
use crate::storage::adapter::{AdapterWriteOp, StorageAdapter};
use crate::storage::aidb::AiDbEngine;
use crate::storage::types::StorageEngineKind;

const GROUP_READY_MAX_ATTEMPTS: u32 = 20;
const GROUP_READY_RETRY_DELAY: Duration = Duration::from_millis(250);
const ERR_DATA_GROUP_NOT_READY: &str = "CLUSTERDOWN data group not ready";
const SET_BATCH_MAX_OPS: usize = 128;
/// 凑批等待上限. 1ms 平衡吞吐与延迟: 集群 50c 负载下 items 到达快, 等 1ms 足矣.
const SET_BATCH_MAX_DELAY: Duration = Duration::from_millis(1);
/// 已凑够该数量则不再等待, 立即 propose. 原值 4 → 在 50c 集群下 29% 批次仅 4 条,
/// 每 batch 2ms Raft 开销稀释严重. 提升至 12 降低小批比例.
const SET_BATCH_EAGER_FLUSH: usize = 12;

/// 数据面感知的存储适配器.
pub struct ClusterDataAdapter {
    local: Arc<dyn StorageAdapter>,
    set_batchers: Mutex<HashMap<u64, Arc<GroupSetBatcher>>>,
}

struct GroupSetBatcher {
    tx: mpsc::Sender<SetBatchItem>,
}

struct SetBatchItem {
    key: Vec<u8>,
    value: Vec<u8>,
    ack: oneshot::Sender<std::result::Result<(), String>>,
}

impl ClusterDataAdapter {
    pub fn new(local: Arc<dyn StorageAdapter>) -> Arc<Self> {
        Arc::new(Self {
            local,
            set_batchers: Mutex::new(HashMap::new()),
        })
    }

    /// 本节点作为迁槽目标时的本地 data group (IMPORTING / MIGRATE RESTORE).
    fn local_migration_target_group(mgr: &ClusterStateManager) -> Option<u64> {
        if let Some(state) = mgr.meta_raft.get_migration_state() {
            let target = match &state {
                SlotMigrationState::Prepare { target_group, .. }
                | SlotMigrationState::Migrating { target_group, .. } => *target_group,
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
    fn importing_write_group(mgr: &ClusterStateManager, slot: u16) -> Option<u64> {
        if !mgr.importing_slots.read().contains_key(&slot) {
            return None;
        }
        Self::local_migration_target_group(mgr)
    }

    /// 读路径: 按 router 已分配 slot 路由到本地 group.
    /// 对于 Migrating 状态的 slot，如果源 group 确实在本地 self.groups 中
    /// （而非仅 is_group_local 返回 true 的竞态窗口），则从本地读。
    fn route_read(key: &[u8]) -> Option<(Arc<ClusterStateManager>, u64)> {
        let mgr = CLUSTER_STATE_MGR.get()?.clone();
        let (_, user_key) = AiDbEngine::decode_key(key)?;
        match mgr.router.route_key(&user_key) {
            Ok((gid, SlotStatus::Assigned(_))) => Some((mgr, gid)),
            Ok((gid, SlotStatus::Migrating(_)))
                if mgr.multi_raft.get_groups().read().contains_key(&gid) =>
            {
                Some((mgr, gid))
            }
            _ => None,
        }
    }

    /// 写路径: IMPORTING 时优先写入本节点目标 group.
    fn route_write(key: &[u8]) -> Option<(Arc<ClusterStateManager>, u64)> {
        let mgr = CLUSTER_STATE_MGR.get()?.clone();
        let (_, user_key) = AiDbEngine::decode_key(key)?;
        let slot = key_to_slot(&user_key);
        if let Some(gid) = Self::importing_write_group(&mgr, slot) {
            return Some((mgr, gid));
        }
        match mgr.router.route_key(&user_key) {
            Ok((gid, SlotStatus::Assigned(_))) => {
                if mgr.multi_raft.get_groups().read().contains_key(&gid) {
                    Some((mgr, gid))
                } else {
                    // decide() 已在 IMPORTING 窗口放行本地写, 但 router 仍指向源 group.
                    Self::local_migration_target_group(&mgr).map(|local_gid| (mgr, local_gid))
                }
            }
            Ok((gid, SlotStatus::Migrating(_))) => {
                if mgr.multi_raft.get_groups().read().contains_key(&gid) {
                    Some((mgr, gid))
                } else {
                    Self::local_migration_target_group(&mgr).map(|local_gid| (mgr, local_gid))
                }
            }
            _ => None,
        }
    }

    fn map_err<E: std::fmt::Display>(e: E) -> Error {
        Error::Cluster(e.to_string())
    }

    fn check_response(resp: Response) -> Result<()> {
        match resp {
            Response::Error(msg) => Err(Error::Cluster(msg)),
            _ => Ok(()),
        }
    }

    fn data_group_not_ready_err() -> Error {
        Error::Cluster(ERR_DATA_GROUP_NOT_READY.into())
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
            Error::Cluster(msg) => {
                msg.contains("not found locally")
                    || msg.contains("data group not ready")
                    || msg.contains("当前 Leader: None")
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
        tokio::spawn(run_set_batcher(mgr, gid, rx));
        batchers.insert(gid, batcher.clone());
        batcher
    }

    async fn submit_batched_set(
        &self,
        mgr: Arc<ClusterStateManager>,
        gid: u64,
        key: Vec<u8>,
        value: Vec<u8>,
    ) -> Result<()> {
        let batcher = self.get_or_spawn_set_batcher(mgr, gid);
        let (ack, wait) = oneshot::channel();
        batcher
            .tx
            .send(SetBatchItem {
                key,
                value,
                ack,
            })
            .await
            .map_err(|_| Error::Cluster("data group write batcher stopped".into()))?;
        wait.await
            .map_err(|_| Error::Cluster("data group write batcher dropped response".into()))?
            .map_err(Error::Cluster)
    }
}

async fn run_set_batcher(
    mgr: Arc<ClusterStateManager>,
    gid: u64,
    mut rx: mpsc::Receiver<SetBatchItem>,
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
            if items.len() >= SET_BATCH_MAX_OPS || items.len() >= SET_BATCH_EAGER_FLUSH {
                break;
            }
            match tokio::time::timeout(SET_BATCH_MAX_DELAY, rx.recv()).await {
                Ok(Some(item)) => items.push(item),
                Ok(None) | Err(_) => break,
            }
        }

        let mut tb = ThinWriteBatch::new();
        for item in &items {
            tb.put(item.key.clone(), item.value.clone());
        }

        let result = ClusterDataAdapter::propose_group_with_retry(&mgr, gid, Request::WriteBatch(tb))
            .await
            .and_then(ClusterDataAdapter::check_response)
            .map_err(|e| e.to_string());

        for item in items {
            let _ = item.ack.send(result.clone());
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
    async fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        match Self::route_read(key) {
            Some((mgr, gid)) => mgr.multi_raft.get_local(gid, key).await.map_err(|e| {
                tracing::warn!(gid = gid, error = %e, "get_local failed");
                Self::map_err(e)
            }),
            None if Self::should_use_local_engine(key) => self.local.get(key).await,
            None => Err(Self::data_group_not_ready_err()),
        }
    }

    async fn set(&self, key: &[u8], value: &[u8]) -> Result<()> {
        match Self::route_write(key) {
            Some((mgr, gid)) => {
                self.submit_batched_set(mgr, gid, key.to_vec(), value.to_vec())
                    .await
            }
            None if Self::should_use_local_engine(key) => self.local.set(key, value).await,
            None => Err(Self::data_group_not_ready_err()),
        }
    }

    async fn delete(&self, key: &[u8]) -> Result<bool> {
        match Self::route_write(key) {
            Some((mgr, gid)) => {
                let existed = mgr
                    .multi_raft
                    .get_local(gid, key)
                    .await
                    .map_err(|e| {
                        tracing::warn!(gid = gid, error = %e, "get_local failed in delete");
                        Self::map_err(e)
                    })?
                    .is_some();
                if existed {
                    let resp = Self::propose_group_with_retry(
                        &mgr,
                        gid,
                        Request::Delete { key: key.to_vec() },
                    )
                    .await?;
                    Self::check_response(resp)?;
                }
                Ok(existed)
            }
            None if Self::should_use_local_engine(key) => self.local.delete(key).await,
            None => Err(Self::data_group_not_ready_err()),
        }
    }

    async fn exists(&self, key: &[u8]) -> Result<bool> {
        Ok(self.get(key).await?.is_some())
    }

    async fn write_batch(&self, batch: Vec<AdapterWriteOp>) -> Result<()> {
        // 按 group 分组后逐组提交, 避免跨 group 的单次提案.
        let mut by_group: HashMap<u64, (Arc<ClusterStateManager>, ThinWriteBatch)> = HashMap::new();
        let mut local_ops: Vec<AdapterWriteOp> = Vec::new();
        for op in batch {
            let key = match &op {
                AdapterWriteOp::Put { key, .. } => key.as_slice(),
                AdapterWriteOp::Delete { key } => key.as_slice(),
            };
            match Self::route_write(key) {
                Some((mgr, gid)) => {
                    let entry = by_group
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
        for (gid, (mgr, tb)) in by_group {
            let resp = Self::propose_group_with_retry(&mgr, gid, Request::WriteBatch(tb)).await?;
            Self::check_response(resp)?;
        }
        if !local_ops.is_empty() {
            self.local.write_batch(local_ops).await?;
        }
        Ok(())
    }

    async fn scan_prefix(&self, prefix: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        match CLUSTER_STATE_MGR.get() {
            Some(mgr) => {
                let mut out = Vec::new();
                for gid in mgr.multi_raft.local_group_ids() {
                    let pairs = mgr
                        .multi_raft
                        .scan_local_pairs(gid)
                        .await
                        .map_err(Self::map_err)?;
                    for (k, v) in pairs {
                        if k.starts_with(prefix) {
                            out.push((k, v));
                        }
                    }
                }
                Ok(out)
            }
            None => self.local.scan_prefix(prefix).await,
        }
    }

    async fn delete_range(&self, start: &[u8], end: &[u8]) -> Result<()> {
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
                        if k.as_slice() >= start && k.as_slice() < end {
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
