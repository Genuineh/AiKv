use std::collections::HashMap;
use std::time::Duration;

use crate::cluster::state::CLUSTER_STATE_MGR;
use crate::error::Error;
use crate::protocol::RespValue;
use aidb::cluster::types::ClusterError;
use aidb::Error as AidbError;
use bytes::Bytes;
use openraft::rt::watch::WatchReceiver;
use tokio::time::sleep;

use super::bytes_to_str;
use super::map_propose_error;
use super::parse_cluster_node_id;
use super::parse_int;

// ---------------------------------------------------------------------------
// CLUSTER GROUPSTATUS
// ---------------------------------------------------------------------------

/// 返回本地 data group 的 Raft 运行时指标.
///
/// 可选参数 `group-id`: 仅显示该 group; 省略则列出所有本地 group.
pub fn cluster_groupstatus(group_id: Option<u64>) -> Result<String, String> {
    let mgr = CLUSTER_STATE_MGR
        .get()
        .ok_or_else(|| "CLUSTERDOWN Cluster not initialized".to_string())?;

    let groups = mgr.multi_raft.get_groups();
    let groups_guard = groups.read();

    let iter: Vec<(u64, &aidb::cluster::OpenRaftNode)> = match group_id {
        Some(gid) => {
            let node = groups_guard
                .get(&gid)
                .ok_or_else(|| format!("ERR group {gid} not found on this node"))?;
            vec![(gid, node.as_ref())]
        }
        None => groups_guard
            .iter()
            .map(|(gid, node)| (*gid, node.as_ref()))
            .collect(),
    };

    let mut lines: Vec<String> = Vec::new();
    for (gid, node) in iter {
        let raft = node.raft();
        let current_leader = raft.metrics().borrow_watched().current_leader;
        let leader = current_leader
            .map(|id| id.to_string())
            .unwrap_or_else(|| "none".to_string());

        let members: Vec<String> = raft
            .metrics()
            .borrow_watched()
            .membership_config
            .nodes()
            .map(|(nid, _)| nid.to_string())
            .collect();

        let last_log_index_val = raft.metrics().borrow_watched().last_log_index;
        let last_log_index = last_log_index_val
            .map(|i| i.to_string())
            .unwrap_or_else(|| "0".to_string());

        let last_applied_val = raft.metrics().borrow_watched().last_applied;
        let last_applied = last_applied_val
            .map(|i| i.to_string())
            .unwrap_or_else(|| "0".to_string());

        let running_state = raft.metrics().borrow_watched().running_state.clone();
        let state = match &running_state {
            Ok(()) => "ok".to_string(),
            Err(e) => format!("fatal: {e}"),
        };

        let replication_count = raft
            .metrics()
            .borrow_watched()
            .replication
            .as_ref()
            .map(|r| r.len())
            .unwrap_or(0);

        lines.push(format!(
            "group_{gid}: \
             current_leader={leader} \
             members={} \
             last_log_index={last_log_index} \
             last_applied={last_applied} \
             running_state={state} \
             replication_count={replication_count}",
            members.join(","),
        ));
    }
    Ok(lines.join("\n"))
}
/// 创建空 data group (不分配槽位).
///
/// 用法: `CLUSTER CREATEGROUP <primary-id> [group-id]`
/// 默认 `group_id = primary_id`. 之后通过 `ADD_REPLICA` 扩展副本.
pub async fn cluster_create_group(
    primary_id: u64,
    group_id: Option<u64>,
) -> Result<String, String> {
    let mgr = CLUSTER_STATE_MGR
        .get()
        .ok_or_else(|| "CLUSTERDOWN Cluster not initialized".to_string())?;
    let meta = mgr.meta_raft.get_cluster_meta();

    if !meta.nodes.contains_key(&primary_id) {
        return Err("ERR Node not known".to_string());
    }
    if meta
        .groups
        .values()
        .any(|g| g.replicas.iter().any(|r| r.node_id == primary_id))
    {
        return Err("ERR Node already in a data group".to_string());
    }

    let gid = group_id.unwrap_or(primary_id);
    if meta.groups.contains_key(&gid) {
        return Err("ERR Group already exists".to_string());
    }

    mgr.meta_raft
        .propose(aidb::cluster::MetaRequest::CreateGroup {
            group_id: gid,
            initial_replicas: vec![(primary_id, true)],
        })
        .await
        .map_err(map_propose_error)?;

    mgr.meta_raft
        .propose(aidb::cluster::MetaRequest::ChangeNodeRole {
            node_id: primary_id,
            role: aidb::cluster::NodeRole::Voter,
        })
        .await
        .map_err(map_propose_error)?;

    let meta = mgr.meta_raft.get_cluster_meta();
    let slot_table = mgr.meta_raft.get_slot_table();
    let mut group_nodes: HashMap<u64, Vec<u64>> = HashMap::new();
    let mut node_addrs: HashMap<u64, String> = HashMap::new();
    for (group_id, group) in &meta.groups {
        group_nodes.insert(
            *group_id,
            group.replicas.iter().map(|r| r.node_id).collect(),
        );
    }
    for (nid, node) in &meta.nodes {
        let addr = node
            .client_addr
            .clone()
            .unwrap_or_else(|| node.rpc_addr.clone());
        node_addrs.insert(*nid, addr);
    }
    let group_leaders: HashMap<u64, u64> = meta
        .groups
        .iter()
        .filter_map(|(gid, g)| {
            g.replicas
                .iter()
                .find(|r| r.is_leader)
                .map(|r| (*gid, r.node_id))
        })
        .collect();
    mgr.router
        .refresh_from_data(slot_table, group_nodes, node_addrs, group_leaders);

    Ok("OK".to_string())
}

/// 添加副本 (从主节点调用).
///
/// 先尝试通过 MembershipCoordinator 直接操作 MultiRaft (包含 add_learner +
/// MetaRaft 元数据 + Raft 成员变更). 如果因为 group 不在本地或 group 尚未完成
/// 初始化而失败, 则退回到仅更新 MetaRaft 元数据的路径 — 后续 LifecycleManager
/// 会在 group leader 节点上检测 drift 并自动对账.
pub async fn cluster_add_replica(primary_id: u64, replica_id: u64) -> Result<String, String> {
    let mgr = CLUSTER_STATE_MGR
        .get()
        .ok_or_else(|| "CLUSTERDOWN Cluster not initialized".to_string())?;
    let meta = mgr.meta_raft.get_cluster_meta();

    let groups: Vec<u64> = meta
        .groups
        .iter()
        .filter(|(_, g)| g.replicas.iter().any(|r| r.node_id == primary_id))
        .map(|(id, _)| *id)
        .collect();
    if groups.is_empty() {
        return Err("ERR Primary is not a data group member".to_string());
    }

    // 尝试直接 MultiRaft 路径 (最快, group 必须已在本地且有 leader).
    // 最多重试 3 次 (比原来 10 次少, 因为现在有元数据回退路径).
    let coordinator = mgr
        .membership_coordinator
        .as_ref()
        .ok_or_else(|| "ERR Membership coordinator not available".to_string())?;
    let mut direct_ok = false;
    let mut delay_ms = 100u64;
    for attempt in 1u32..=3 {
        direct_ok = true;
        for &gid in &groups {
            match coordinator
                .change_group_membership(gid, vec![replica_id], vec![], None)
                .await
            {
                Ok(()) => {}
                Err(e) => {
                    let is_not_leader =
                        matches!(&e, AidbError::Cluster(ClusterError::NotLeader { .. }));
                    if is_not_leader && attempt < 3 {
                        direct_ok = false;
                        sleep(Duration::from_millis(delay_ms)).await;
                        delay_ms = delay_ms.saturating_mul(2).min(2000);
                        break;
                    }
                    // 其他错误 (如 group 未初始化, 网络不可达等) → 不重试, 直接走元数据路径.
                    direct_ok = false;
                    tracing::info!(
                      group_id = gid,
                      error = %e,
                      "direct MultiRaft path failed for ADD_REPLICA, falling back to metadata-only"
                    );
                }
            }
        }
        if direct_ok {
            break;
        }
    }

    if direct_ok {
        // 直接路径成功: 刷新 Router 缓存.
        let meta = mgr.meta_raft.get_cluster_meta();
        let slot_table = mgr.meta_raft.get_slot_table();
        let mut group_nodes: HashMap<u64, Vec<u64>> = HashMap::new();
        let mut node_addrs: HashMap<u64, String> = HashMap::new();
        for (gid, group) in &meta.groups {
            group_nodes.insert(*gid, group.replicas.iter().map(|r| r.node_id).collect());
        }
        for (nid, node) in &meta.nodes {
            let addr = node
                .client_addr
                .clone()
                .unwrap_or_else(|| node.rpc_addr.clone());
            node_addrs.insert(*nid, addr);
        }
        let group_leaders: HashMap<u64, u64> = meta
            .groups
            .iter()
            .filter_map(|(gid, g)| {
                g.replicas
                    .iter()
                    .find(|r| r.is_leader)
                    .map(|r| (*gid, r.node_id))
            })
            .collect();
        mgr.router
            .refresh_from_data(slot_table, group_nodes, node_addrs, group_leaders);
        return Ok("OK".to_string());
    }

    // 直接路径失败 → 回退到仅更新 MetaRaft 元数据.
    // 后续 LifecycleManager 在 group leader 节点上检测到 drift 后会
    // 自动执行实际的 MultiRaft 成员变更.
    tracing::info!(
        primary_id,
        replica_id,
        "ADD_REPLICA: direct path failed, falling back to metadata-only"
    );
    for &gid in &groups {
        let meta = mgr.meta_raft.get_cluster_meta();
        let group = meta
            .groups
            .get(&gid)
            .ok_or_else(|| "ERR group not found in metadata".to_string())?;
        let mut new_replicas: Vec<(u64, bool)> = group
            .replicas
            .iter()
            .map(|r| (r.node_id, r.is_leader))
            .collect();
        if !new_replicas.iter().any(|(id, _)| *id == replica_id) {
            new_replicas.push((replica_id, false));
        }
        mgr.meta_raft
            .propose(aidb::cluster::MetaRequest::ChangeGroupMembership {
                group_id: gid,
                new_replicas,
                config_version: group.config_version + 1,
            })
            .await
            .map_err(map_propose_error)?;
    }

    // 刷新 Router 缓存.
    let meta = mgr.meta_raft.get_cluster_meta();
    let slot_table = mgr.meta_raft.get_slot_table();
    let mut group_nodes: HashMap<u64, Vec<u64>> = HashMap::new();
    let mut node_addrs: HashMap<u64, String> = HashMap::new();
    for (gid, group) in &meta.groups {
        group_nodes.insert(*gid, group.replicas.iter().map(|r| r.node_id).collect());
    }
    for (nid, node) in &meta.nodes {
        let addr = node
            .client_addr
            .clone()
            .unwrap_or_else(|| node.rpc_addr.clone());
        node_addrs.insert(*nid, addr);
    }
    let group_leaders: HashMap<u64, u64> = meta
        .groups
        .iter()
        .filter_map(|(gid, g)| {
            g.replicas
                .iter()
                .find(|r| r.is_leader)
                .map(|r| (*gid, r.node_id))
        })
        .collect();
    mgr.router
        .refresh_from_data(slot_table, group_nodes, node_addrs, group_leaders);
    Ok("OK".to_string())
}

/// 移除副本 (从主节点调用).
///
/// 语义与 `cluster_add_replica` 对称: 优先走 MembershipCoordinator 直接变更,
/// 失败时回退到仅更新 MetaRaft 元数据.
pub async fn cluster_del_replica(primary_id: u64, replica_id: u64) -> Result<String, String> {
    if primary_id == replica_id {
        return Err("ERR Cannot remove primary as replica".to_string());
    }

    let mgr = CLUSTER_STATE_MGR
        .get()
        .ok_or_else(|| "CLUSTERDOWN Cluster not initialized".to_string())?;
    let meta = mgr.meta_raft.get_cluster_meta();

    let groups: Vec<u64> = meta
        .groups
        .iter()
        .filter(|(_, g)| {
            let primary_is_leader = g
                .replicas
                .iter()
                .any(|r| r.node_id == primary_id && r.is_leader);
            let has_replica = g
                .replicas
                .iter()
                .any(|r| r.node_id == replica_id && !r.is_leader);
            primary_is_leader && has_replica
        })
        .map(|(id, _)| *id)
        .collect();
    if groups.is_empty() {
        return Err("ERR Replica not found for primary".to_string());
    }

    for &gid in &groups {
        let group = meta
            .groups
            .get(&gid)
            .ok_or_else(|| "ERR group not found in metadata".to_string())?;
        if group.replicas.len() <= 1 {
            return Err("ERR Cannot remove last replica from group".to_string());
        }
    }

    let coordinator = mgr
        .membership_coordinator
        .as_ref()
        .ok_or_else(|| "ERR Membership coordinator not available".to_string())?;
    let mut direct_ok = false;
    let mut delay_ms = 100u64;
    for attempt in 1u32..=3 {
        direct_ok = true;
        for &gid in &groups {
            match coordinator
                .change_group_membership(gid, vec![], vec![replica_id], None)
                .await
            {
                Ok(()) => {}
                Err(e) => {
                    let is_not_leader =
                        matches!(&e, AidbError::Cluster(ClusterError::NotLeader { .. }));
                    if is_not_leader && attempt < 3 {
                        direct_ok = false;
                        sleep(Duration::from_millis(delay_ms)).await;
                        delay_ms = delay_ms.saturating_mul(2).min(2000);
                        break;
                    }
                    direct_ok = false;
                    tracing::info!(
                      group_id = gid,
                      error = %e,
                      "direct MultiRaft path failed for DEL_REPLICA, falling back to metadata-only"
                    );
                }
            }
        }
        if direct_ok {
            break;
        }
    }

    if !direct_ok {
        tracing::info!(
            primary_id,
            replica_id,
            "DEL_REPLICA: direct path failed, falling back to metadata-only"
        );
        for &gid in &groups {
            let meta = mgr.meta_raft.get_cluster_meta();
            let group = meta
                .groups
                .get(&gid)
                .ok_or_else(|| "ERR group not found in metadata".to_string())?;
            let new_replicas: Vec<(u64, bool)> = group
                .replicas
                .iter()
                .filter(|r| r.node_id != replica_id)
                .map(|r| (r.node_id, r.is_leader))
                .collect();
            if new_replicas.is_empty() {
                return Err("ERR Cannot remove last replica from group".to_string());
            }
            mgr.meta_raft
                .propose(aidb::cluster::MetaRequest::ChangeGroupMembership {
                    group_id: gid,
                    new_replicas,
                    config_version: group.config_version + 1,
                })
                .await
                .map_err(map_propose_error)?;
        }
    }

    let meta = mgr.meta_raft.get_cluster_meta();
    let slot_table = mgr.meta_raft.get_slot_table();
    let mut group_nodes: HashMap<u64, Vec<u64>> = HashMap::new();
    let mut node_addrs: HashMap<u64, String> = HashMap::new();
    for (gid, group) in &meta.groups {
        group_nodes.insert(*gid, group.replicas.iter().map(|r| r.node_id).collect());
    }
    for (nid, node) in &meta.nodes {
        let addr = node
            .client_addr
            .clone()
            .unwrap_or_else(|| node.rpc_addr.clone());
        node_addrs.insert(*nid, addr);
    }
    let group_leaders: HashMap<u64, u64> = meta
        .groups
        .iter()
        .filter_map(|(gid, g)| {
            g.replicas
                .iter()
                .find(|r| r.is_leader)
                .map(|r| (*gid, r.node_id))
        })
        .collect();
    mgr.router
        .refresh_from_data(slot_table, group_nodes, node_addrs, group_leaders);
    Ok("OK".to_string())
}

pub(super) fn handle_groupstatus(
    args: &[bytes::Bytes],
) -> crate::error::Result<crate::protocol::RespValue> {
    let gid: Option<u64> = args.get(1).and_then(|a| parse_int(a));
    let info = cluster_groupstatus(gid).map_err(Error::Command)?;
    Ok(RespValue::BulkString(Some(Bytes::from(info))))
}

pub(super) async fn handle_creategroup(
    args: &[bytes::Bytes],
) -> crate::error::Result<crate::protocol::RespValue> {
    // 用法: CLUSTER CREATEGROUP <primary-id> [group-id]
    let primary_raw = args
        .get(1)
        .map(|a| bytes_to_str(a))
        .ok_or_else(|| Error::Command("ERR wrong number of arguments".into()))??;
    let primary_id = parse_cluster_node_id(primary_raw).map_err(Error::Command)?;
    let group_id = match args.get(2) {
        Some(raw) => Some(parse_cluster_node_id(bytes_to_str(raw)?).map_err(Error::Command)?),
        None => None,
    };
    let msg = cluster_create_group(primary_id, group_id)
        .await
        .map_err(Error::Command)?;
    Ok(RespValue::SimpleString(msg))
}

pub(super) async fn handle_add_replica(
    args: &[bytes::Bytes],
) -> crate::error::Result<crate::protocol::RespValue> {
    // ── 内部命令: 从主节点添加副本 ──
    // 用法: CLUSTER ADD_REPLICA <primary_id> <replica_id>
    let hex_primary = args
        .get(1)
        .map(|a| bytes_to_str(a))
        .ok_or_else(|| Error::Command("ERR wrong number of arguments".into()))??;
    let hex_replica = args
        .get(2)
        .map(|a| bytes_to_str(a))
        .ok_or_else(|| Error::Command("ERR wrong number of arguments".into()))??;
    let primary_id = parse_cluster_node_id(hex_primary).map_err(Error::Command)?;
    let replica_id = parse_cluster_node_id(hex_replica).map_err(Error::Command)?;
    let msg = cluster_add_replica(primary_id, replica_id)
        .await
        .map_err(Error::Command)?;
    Ok(RespValue::SimpleString(msg))
}

pub(super) async fn handle_del_replica(
    args: &[bytes::Bytes],
) -> crate::error::Result<crate::protocol::RespValue> {
    // 用法: CLUSTER DEL_REPLICA <primary_id> <replica_id>
    let primary_raw = args
        .get(1)
        .map(|a| bytes_to_str(a))
        .ok_or_else(|| Error::Command("ERR wrong number of arguments".into()))??;
    let replica_raw = args
        .get(2)
        .map(|a| bytes_to_str(a))
        .ok_or_else(|| Error::Command("ERR wrong number of arguments".into()))??;
    let primary_id = parse_cluster_node_id(primary_raw).map_err(Error::Command)?;
    let replica_id = parse_cluster_node_id(replica_raw).map_err(Error::Command)?;
    let msg = cluster_del_replica(primary_id, replica_id)
        .await
        .map_err(Error::Command)?;
    Ok(RespValue::SimpleString(msg))
}
