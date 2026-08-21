//! 数据面写凑批 actor: 按 data group 将 PUT/DELETE 聚合成 `WriteBatch` 后 propose.
//! 由 `ClusterDataAdapter` 按 group 懒创建.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, oneshot};

use aidb::cluster::{Request, ThinWriteBatch};
use aidb::error::ClusterError as AidbClusterError;

use crate::cluster::state::ClusterStateManager;
use crate::error::{Error, Result};
use crate::storage::cluster_adapter::ClusterDataAdapter;

const SET_BATCH_MAX_OPS: usize = 512;
/// 凑批等待上限. 1ms 平衡吞吐与延迟: 集群 50c 负载下 items 到达快, 等 1ms 足矣.
#[allow(dead_code)]
const SET_BATCH_MAX_DELAY: Duration = Duration::from_millis(1);

pub(super) struct GroupSetBatcher {
    tx: mpsc::Sender<WriteBatchItem>,
}

pub(super) enum WriteBatchItem {
    Put {
        key: Vec<u8>,
        value: Vec<u8>,
        ack: oneshot::Sender<std::result::Result<(), String>>,
    },
    Delete {
        key: Vec<u8>,
        ack: oneshot::Sender<std::result::Result<(), String>>,
    },
}

const MIN_BATCH_TARGET: usize = 16;
const MAX_MICRO_WAIT_US: u64 = 50;

pub(super) fn get_or_spawn_set_batcher(
    batchers: &parking_lot::Mutex<HashMap<u64, Arc<GroupSetBatcher>>>,
    mgr: Arc<ClusterStateManager>,
    gid: u64,
    eager_flush: usize,
) -> Arc<GroupSetBatcher> {
    let mut batchers = batchers.lock();
    if let Some(existing) = batchers.get(&gid) {
        return existing.clone();
    }

    let (tx, rx) = mpsc::channel(4096);
    let batcher = Arc::new(GroupSetBatcher { tx });
    tokio::spawn(run_set_batcher(mgr, gid, rx, eager_flush));
    batchers.insert(gid, batcher.clone());
    batcher
}

/// 统一写批入口: 将 PUT 或 DELETE 请求发送到对应 group 的 batcher.
///
/// DELETE 会先检查 key 是否存在 (短路优化), 不存在则直接返回 `Ok(false)` 跳过 propose.
/// 存在时发送到 batcher, 与其他 DELETE/PUT 聚合后单次 propose.
pub(super) async fn submit_write_op(
    batchers: &parking_lot::Mutex<HashMap<u64, Arc<GroupSetBatcher>>>,
    eager_flush: usize,
    mgr: Arc<ClusterStateManager>,
    gid: u64,
    key: Vec<u8>,
    value: Option<Vec<u8>>,
) -> Result<bool> {
    // Plain PUT: 在 propose 前 get_local 判定 insert, 供上层计数; 失败不更新计数.
    // DELETE: key 不存在则短路跳过 propose.
    let is_put = value.is_some();
    let existed = mgr
        .multi_raft
        .get_local(gid, &key)
        .await
        .map_err(|e| {
            tracing::warn!(gid = gid, error = %e, "get_local failed in submit_write_op");
            ClusterDataAdapter::map_err(e)
        })?
        .is_some();
    if !is_put && !existed {
        return Ok(false);
    }

    let batcher = get_or_spawn_set_batcher(batchers, mgr, gid, eager_flush);
    let (ack, wait) = oneshot::channel();
    let item = match value {
        Some(v) => WriteBatchItem::Put { key, value: v, ack },
        None => WriteBatchItem::Delete { key, ack },
    };
    batcher.tx.send(item).await.map_err(|_| {
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
        })?;
    Ok(if is_put { !existed } else { true })
}

pub(super) async fn run_set_batcher(
    mgr: Arc<ClusterStateManager>,
    gid: u64,
    mut rx: mpsc::Receiver<WriteBatchItem>,
    _eager_flush: usize,
) {
    while let Some(first) = rx.recv().await {
        let t_start = Instant::now();

        let mut items = Vec::with_capacity(SET_BATCH_MAX_OPS);
        items.push(first);

        // 第一阶段:快速非阻塞拉取
        while items.len() < SET_BATCH_MAX_OPS {
            match rx.try_recv() {
                Ok(item) => items.push(item),
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => break,
            }
        }

        // 第二阶段:防单打微退避 (仅在 items 数量小于 MIN_BATCH_TARGET 时触发 50us 避震)
        if items.len() < MIN_BATCH_TARGET {
            tokio::time::sleep(Duration::from_micros(MAX_MICRO_WAIT_US)).await;
            while items.len() < SET_BATCH_MAX_OPS {
                match rx.try_recv() {
                    Ok(item) => items.push(item),
                    Err(mpsc::error::TryRecvError::Empty) => break,
                    Err(mpsc::error::TryRecvError::Disconnected) => break,
                }
            }
        }

        let wait_us = t_start.elapsed().as_micros();

        // Build ThinWriteBatch with dedup:
        // PUT 去重 (重复 key 只保留最后一个), DELETE 不去重.
        let mut tb = ThinWriteBatch::new();
        let item_count = items.len();
        let mut acks: Vec<_> = Vec::with_capacity(item_count);
        let mut seen: HashSet<Vec<u8>> = HashSet::new();

        for item in items.into_iter().rev() {
            match item {
                WriteBatchItem::Put { key, value, ack } => {
                    if seen.insert(key.clone()) {
                        tb.put(key, value);
                    }
                    acks.push(ack);
                }
                WriteBatchItem::Delete { key, ack } => {
                    // DELETE 不去重: 每条都写入, 确保删除语义正确.
                    tb.delete(key);
                    acks.push(ack);
                }
            }
        }
        acks.reverse();

        let t_propose = Instant::now();
        let result = super::cluster_adapter::ClusterDataAdapter::propose_group_with_retry(
            &mgr,
            gid,
            Request::WriteBatch(tb),
        )
        .await
        .and_then(super::cluster_adapter::ClusterDataAdapter::check_response)
        .map_err(|e| e.to_string());

        let propose_us = t_propose.elapsed().as_micros();
        let total_us = t_start.elapsed().as_micros();

        tracing::info!(
            target: "perf",
            gid = gid,
            op_count = item_count,
            wait_us,
            propose_us,
            total_us,
            "batcher_batch_done"
        );

        for ack in acks {
            let _ = ack.send(result.clone());
        }
    }
}
