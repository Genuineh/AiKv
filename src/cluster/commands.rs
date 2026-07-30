use std::collections::{HashMap, HashSet};
use std::time::Duration;

use crate::cluster::state::ClusterStateManager;
use crate::cluster::state::CLUSTER_STATE_MGR;
use crate::cluster::state::DEFAULT_DATA_PORT_OFFSET;
use crate::error::Error;
use crate::protocol::RespValue;
use aidb::cluster::membership_coordinator;
use aidb::cluster::meta_types::ClusterMeta;
use aidb::cluster::meta_types::NodeStatus;
use aidb::cluster::meta_types::SlotMigrationState;
use aidb::cluster::meta_types::SlotStatus;
use aidb::cluster::meta_types::SlotTable;
use aidb::cluster::meta_types::SLOT_COUNT;
use aidb::cluster::router::key_to_slot;
use aidb::cluster::types::ClusterError;
use aidb::cluster::ReplicaAllocator;
#[cfg(feature = "cluster-test-util")]
use aidb::cluster::{failpoint_registry, FailPoint};
use aidb::Error as AidbError;
use bytes::Bytes;
use tokio::time::sleep;

/// Parse an integer from bytes (used by CLUSTER command argument parsing).
pub fn parse_int<T: std::str::FromStr>(bytes: &[u8]) -> Option<T> {
    let s = std::str::from_utf8(bytes).ok()?;
    s.parse().ok()
}

/// Convert a aidb propose error into a Redis response string.
/// - NotLeader with leader address → MOVED redirect
/// - NotLeader without address → CLUSTERDOWN (leader unknown)
/// - Other errors → ERR prefix
fn map_propose_error(e: aidb::Error) -> String {
    if let aidb::Error::Cluster(aidb::cluster::types::ClusterError::NotLeader {
        leader,
        leader_addr,
        ..
    }) = e
    {
        if let Some(mgr) = CLUSTER_STATE_MGR.get() {
            if let Some(leader_id) = leader {
                if let Some(addr) = mgr.announce_resolver.redirect_addr(leader_id, &mgr.router) {
                    return format!("MOVED 0 {addr}");
                }
            }
            if let Some(addr) = leader_addr.as_deref() {
                if let Some(redirect) = mgr.announce_resolver.redirect_from_addr_str(addr) {
                    return format!("MOVED 0 {redirect}");
                }
            }
        }
        return match leader_addr {
            Some(addr) => format!("MOVED 0 {addr}"),
            None => "CLUSTERDOWN The cluster is not available".to_string(),
        };
    }
    format!("ERR {e}")
}

fn bytes_to_str(bytes: &[u8]) -> std::result::Result<&str, Error> {
    std::str::from_utf8(bytes).map_err(|_| Error::Command("ERR invalid utf8".into()))
}

// ---------------------------------------------------------------------------
// CLUSTER KEYSLOT
// ---------------------------------------------------------------------------
#[tracing::instrument(name = "cmd_cluster_keyslot", skip_all)]
pub fn cluster_keyslot(key: &[u8]) -> Result<String, String> {
    let slot = key_to_slot(key);
    Ok(slot.to_string())
}

// ---------------------------------------------------------------------------
// CLUSTER MYID
// ---------------------------------------------------------------------------
#[tracing::instrument(name = "cmd_cluster_myid", skip_all)]
pub fn cluster_myid() -> Result<String, String> {
    let mgr = CLUSTER_STATE_MGR
        .get()
        .ok_or_else(|| "CLUSTERDOWN Cluster not initialized".to_string())?;
    Ok(format!("{:040x}", mgr.node_id))
}

// ---------------------------------------------------------------------------
// CLUSTER INFO
// ---------------------------------------------------------------------------

/// 解析 group leader (CLUSTER INFO 健康判定; 不用 Router 的 first-node 回退).
fn resolve_group_leader_for_info(
    mgr: &ClusterStateManager,
    meta: &ClusterMeta,
    group_id: u64,
) -> Option<u64> {
    if let Some(group) = meta.groups.get(&group_id) {
        if let Some(replica) = group.replicas.iter().find(|r| r.is_leader) {
            return Some(replica.node_id);
        }
    }
    if let Some(node) = mgr.multi_raft.get_groups().read().get(&group_id) {
        use openraft::rt::watch::WatchReceiver;
        return node.raft().metrics().borrow_watched().current_leader;
    }
    None
}

/// 动态 `cluster_state:ok` / `fail` (对齐 oldmain: slot 满 + leader + 映射一致).
fn compute_cluster_state<F>(
    slot_table: &SlotTable,
    meta: &ClusterMeta,
    resolve_leader: F,
) -> &'static str
where
    F: Fn(u64) -> Option<u64>,
{
    let covered = slot_table
        .iter()
        .filter(|s| !matches!(s, SlotStatus::Unallocated))
        .count();
    if covered != SLOT_COUNT {
        return "fail";
    }

    let slots_map_to_known_groups = slot_table.iter().all(|status| match status {
        SlotStatus::Unallocated => true,
        SlotStatus::Assigned(gid) | SlotStatus::Migrating(gid) => meta.groups.contains_key(gid),
    });
    if !slots_map_to_known_groups {
        return "fail";
    }

    let groups_with_slots: HashSet<u64> = slot_table
        .iter()
        .filter_map(|status| match status {
            SlotStatus::Assigned(gid) | SlotStatus::Migrating(gid) => Some(*gid),
            SlotStatus::Unallocated => None,
        })
        .collect();

    if !groups_with_slots
        .iter()
        .all(|gid| resolve_leader(*gid).is_some())
    {
        return "fail";
    }

    "ok"
}

#[tracing::instrument(name = "cmd_cluster_info", skip_all)]
pub fn cluster_info() -> Result<String, String> {
    let mgr = CLUSTER_STATE_MGR
        .get()
        .ok_or_else(|| "CLUSTERDOWN Cluster not initialized".to_string())?;
    let meta = mgr.meta_raft.get_cluster_meta();
    let slot_table = mgr.meta_raft.get_slot_table();

    let assigned = slot_table
        .iter()
        .filter(|s| matches!(s, SlotStatus::Assigned(_)))
        .count();
    let migrating = slot_table
        .iter()
        .filter(|s| matches!(s, SlotStatus::Migrating(_)))
        .count();
    let ok_count = assigned;
    let known_nodes = meta.nodes.len();
    let group_count = meta.groups.len();
    let epoch = meta.version;
    let (msgs_sent, msgs_received) = mgr
        .metrics
        .as_ref()
        .map(|m| (m.cluster_messages_sent(), m.cluster_messages_received()))
        .unwrap_or((0, 0));
    let cluster_state = compute_cluster_state(&slot_table, &meta, |gid| {
        resolve_group_leader_for_info(mgr, &meta, gid)
    });

    Ok(format!(
        "cluster_state:{cluster_state}\n\
         cluster_slots_assigned:{assigned}\n\
         cluster_slots_ok:{ok_count}\n\
         cluster_slots_pfail:0\n\
         cluster_slots_fail:0\n\
         cluster_slots_migrating:{migrating}\n\
         cluster_known_nodes:{known_nodes}\n\
         cluster_size:{group_count}\n\
         cluster_current_epoch:{epoch}\n\
         cluster_my_epoch:{epoch}\n\
         cluster_stats_messages_sent:{msgs_sent}\n\
         cluster_stats_messages_received:{msgs_received}\n\
         total_cluster_links_buffer_limit_exceeded:0\n\
         cluster_slot_migration_active_tasks:0\n\
         cluster_slot_migration_active_trim_running:0\n\
         cluster_slot_migration_active_trim_current_job_keys:0\n\
         cluster_slot_migration_active_trim_current_job_trimmed:0\n\
         cluster_slot_migration_stats_active_trim_started:0\n\
         cluster_slot_migration_stats_active_trim_completed:0\n\
         cluster_slot_migration_stats_active_trim_cancelled:0\n",
    ))
}

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

    use openraft::rt::watch::WatchReceiver;

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

// ---------------------------------------------------------------------------
// CLUSTER FAILPOINT (cluster-test-util feature only)
// ---------------------------------------------------------------------------

/// 故障注入管理.
///
/// CLUSTER FAILPOINT ARM <name> [once]
/// CLUSTER FAILPOINT RELEASE <name>
/// CLUSTER FAILPOINT STATUS
#[cfg(feature = "cluster-test-util")]
fn cluster_failpoint(args: &[Bytes]) -> Result<String, String> {
    let sub = args
        .get(1)
        .ok_or_else(|| "ERR wrong number of arguments".to_string())?;
    let sub_str = bytes_to_str(sub).map_err(|e| e.to_string())?;

    match sub_str.to_uppercase().as_str() {
        "ARM" => {
            let name = args
                .get(2)
                .ok_or_else(|| "ERR wrong number of arguments for ARM".to_string())?;
            let name_str = bytes_to_str(name).map_err(|e| e.to_string())?;
            let fp = FailPoint::from_str(name_str)
                .ok_or_else(|| format!("ERR unknown failpoint: {name_str}"))?;
            if args
                .get(3)
                .map_or(false, |a| a.eq_ignore_ascii_case(b"once"))
            {
                failpoint_registry().arm_once(fp);
                Ok(format!("armed {} (once)", fp.display_name()))
            } else {
                failpoint_registry().arm(fp);
                Ok(format!("armed {}", fp.display_name()))
            }
        }
        "RELEASE" => {
            let name = args
                .get(2)
                .ok_or_else(|| "ERR wrong number of arguments for RELEASE".to_string())?;
            let name_str = bytes_to_str(name).map_err(|e| e.to_string())?;
            let fp = FailPoint::from_str(name_str)
                .ok_or_else(|| format!("ERR unknown failpoint: {name_str}"))?;
            failpoint_registry().release(fp);
            Ok(format!("released {}", fp.display_name()))
        }
        "STATUS" => Ok(failpoint_registry().status()),
        _ => Err(format!("ERR unknown FAILPOINT subcommand: {sub_str}")),
    }
}

// ---------------------------------------------------------------------------
// CLUSTER NODES
// ---------------------------------------------------------------------------

/// Redis CLUSTER NODES link-state from MetaRaft node status.
fn cluster_nodes_link_state(status: &NodeStatus) -> &'static str {
    match status {
        NodeStatus::Offline => "disconnected",
        NodeStatus::Online | NodeStatus::Draining => "connected",
    }
}

/// Redis CLUSTER NODES 的 master/slave 标签 (与 primary_id 一致, 看分片 group 而非 MetaRaft NodeRole).
///
/// 副本在 ADD_REPLICA 后常为 Voter (参与 MultiRaft), 若用 `NodeRole::Voter` 会误显示为 master.
pub(crate) fn cluster_node_role_label(
    meta: &aidb::cluster::meta_types::ClusterMeta,
    nid: u64,
) -> &'static str {
    let in_group = meta
        .groups
        .values()
        .any(|g| g.replicas.iter().any(|r| r.node_id == nid));
    let is_leader = meta
        .groups
        .values()
        .any(|g| g.replicas.iter().any(|r| r.node_id == nid && r.is_leader));
    if is_leader || !in_group {
        "master"
    } else {
        "slave"
    }
}

#[tracing::instrument(name = "cmd_cluster_nodes", skip_all)]
pub fn cluster_nodes() -> Result<String, String> {
    let mgr = CLUSTER_STATE_MGR
        .get()
        .ok_or_else(|| "CLUSTERDOWN Cluster not initialized".to_string())?;
    let meta = mgr.meta_raft.get_cluster_meta();
    let migration_state = mgr.meta_raft.get_migration_state();
    let mut lines = Vec::new();

    for (nid, node) in &meta.nodes {
        let role = cluster_node_role_label(&meta, *nid);
        let base_flags = if *nid == mgr.node_id {
            format!("myself,{}", role)
        } else {
            role.to_string()
        };

        // Append migration flags if this node is source or target of active migration
        let migration_flags = match &migration_state {
            Some(state) => {
                let (src_gid, dst_gid) = match state {
                    aidb::cluster::meta_types::SlotMigrationState::Prepare {
                        source_group,
                        target_group,
                        ..
                    }
                    | aidb::cluster::meta_types::SlotMigrationState::Migrating {
                        source_group,
                        target_group,
                        ..
                    }
                    | aidb::cluster::meta_types::SlotMigrationState::Frozen {
                        source_group,
                        target_group,
                        ..
                    }
                    | aidb::cluster::meta_types::SlotMigrationState::ReadyToCommit {
                        source_group,
                        target_group,
                        ..
                    } => (*source_group, *target_group),
                };
                let in_source = meta
                    .groups
                    .get(&src_gid)
                    .map(|g| g.replicas.iter().any(|r| r.node_id == *nid))
                    .unwrap_or(false);
                let in_target = meta
                    .groups
                    .get(&dst_gid)
                    .map(|g| g.replicas.iter().any(|r| r.node_id == *nid))
                    .unwrap_or(false);
                match (in_source, in_target) {
                    (true, _) => ",migrating",
                    (_, true) => ",importing",
                    _ => "",
                }
            }
            None => "",
        };
        let flags = format!("{}{}", base_flags, migration_flags);

        // 从 group 元数据推导 primary_id: leader 显示 "-", replica 显示 leader 的 node_id.
        let display_primary_id = {
            let node_group = meta
                .groups
                .iter()
                .find(|(_, g)| g.replicas.iter().any(|r| r.node_id == *nid));
            match node_group {
                Some((_, group)) => {
                    let is_leader = group
                        .replicas
                        .iter()
                        .any(|r| r.node_id == *nid && r.is_leader);
                    if is_leader {
                        "-".to_string()
                    } else {
                        group
                            .replicas
                            .iter()
                            .find(|r| r.is_leader)
                            .map(|r| format!("{:040x}", r.node_id))
                            .unwrap_or_else(|| "-".to_string())
                    }
                }
                None => "-".to_string(),
            }
        };

        // 从 group 元数据推导 node 的 slot 范围 (仅 leader 显示).
        let slot_ranges_str = {
            let node_group = meta
                .groups
                .iter()
                .find(|(_, g)| g.replicas.iter().any(|r| r.node_id == *nid));
            match node_group {
                Some((_, group)) if !group.slot_ranges.is_empty() => {
                    let is_leader = group
                        .replicas
                        .iter()
                        .any(|r| r.node_id == *nid && r.is_leader);
                    if is_leader {
                        let ranges: Vec<String> = group
                            .slot_ranges
                            .iter()
                            .map(|(s, e)| format!("{s}-{e}"))
                            .collect();
                        format!(" {}", ranges.join(" "))
                    } else {
                        String::new()
                    }
                }
                _ => String::new(),
            }
        };

        let offset = CLUSTER_STATE_MGR
            .get()
            .map_or(DEFAULT_DATA_PORT_OFFSET, |m| m.data_port_offset);
        let cport = match mgr.router.get_node_addr(*nid) {
            Some(ref addr) => addr
                .rsplit(':')
                .next()
                .and_then(|port| port.parse::<u16>().ok())
                .map(|p| p + offset)
                .unwrap_or(0),
            None => 0,
        };
        // Note: rsplit(':') handles both IPv4 ("127.0.0.1:7000") and
        // IPv6 ("[::1]:7000") by taking the last colon segment as port.

        // Use client_addr for the display address (host:port that clients
        // connect to), falling back to rpc_addr if not set.  Without this,
        // CLUSTER NODES shows the gRPC/RPC port and clients that discover
        // topology via CLUSTER NODES will fail to connect.
        let addr = mgr
            .router
            .get_node_addr(*nid)
            .unwrap_or_else(|| node.rpc_addr.clone());
        let link_state = cluster_nodes_link_state(&node.status);
        let line = format!(
            "{:040x} {}@{} {} {} 0 0 {} {}{}\n",
            nid, addr, cport, flags, display_primary_id, meta.version, link_state, slot_ranges_str,
        );
        lines.push(line);
    }

    Ok(lines.join(""))
}

// ---------------------------------------------------------------------------
// CLUSTER SLOTS
// ---------------------------------------------------------------------------

/// 节点端点信息 (用于 CLUSTER SLOTS / SHARDS 响应).
#[derive(Debug, Clone)]
pub struct NodeEndpoint {
    pub host: String,
    pub port: u16,
    pub node_id: u64,
}

/// 槽范围信息, 包含 master 和 replicas.
#[derive(Debug, Clone)]
pub struct SlotRangeInfo {
    pub start: u16,
    pub end: u16,
    pub master: NodeEndpoint,
    pub replicas: Vec<NodeEndpoint>,
}

/// 从 Router + AnnounceResolver 解析 NodeEndpoint (CLUSTER SLOTS / SHARDS).
fn resolve_endpoint(mgr: &ClusterStateManager, node_id: u64) -> Option<NodeEndpoint> {
    let (host, port) = mgr
        .announce_resolver
        .endpoint_for_node(node_id, &mgr.router)?;
    Some(NodeEndpoint {
        host,
        port,
        node_id,
    })
}

#[tracing::instrument(name = "cmd_cluster_slots", skip_all)]
pub fn cluster_slots() -> Result<Vec<SlotRangeInfo>, String> {
    let mgr = CLUSTER_STATE_MGR
        .get()
        .ok_or_else(|| "CLUSTERDOWN Cluster not initialized".to_string())?;
    let meta = mgr.meta_raft.get_cluster_meta();
    let slot_table = mgr.meta_raft.get_slot_table();
    let migration_state = mgr.meta_raft.get_migration_state();

    let mut ranges: Vec<SlotRangeInfo> = Vec::new();
    let mut i = 0usize;
    while i < slot_table.len() {
        let status = &slot_table[i];
        match status {
            SlotStatus::Assigned(gid) => {
                let start = i as u16;
                while i < slot_table.len() && slot_table[i] == SlotStatus::Assigned(*gid) {
                    i += 1;
                }
                let end = (i - 1) as u16;
                let group = meta.groups.get(gid);

                // 收集 master 和 replicas
                let mut master: Option<NodeEndpoint> = None;
                let mut replicas: Vec<NodeEndpoint> = Vec::new();
                if let Some(group) = group {
                    for r in &group.replicas {
                        if let Some(ep) = resolve_endpoint(mgr, r.node_id) {
                            if r.is_leader {
                                master = Some(ep);
                            } else {
                                replicas.push(ep);
                            }
                        }
                    }
                }

                if let Some(master) = master {
                    ranges.push(SlotRangeInfo {
                        start,
                        end,
                        master,
                        replicas,
                    });
                }
            }
            SlotStatus::Migrating(gid) => {
                // Build range for slots currently being migrated
                let start = i as u16;
                while i < slot_table.len() && slot_table[i] == SlotStatus::Migrating(*gid) {
                    i += 1;
                }
                let end = (i - 1) as u16;

                // Find source group (gid = source_group_id in Migrating status)
                let source_group = meta.groups.get(gid);
                // Get target group from migration state
                let target_gid = migration_state.as_ref().map(|ms| match ms {
                    SlotMigrationState::Prepare { target_group, .. }
                    | SlotMigrationState::Migrating { target_group, .. }
                    | SlotMigrationState::Frozen { target_group, .. }
                    | SlotMigrationState::ReadyToCommit { target_group, .. } => *target_group,
                });
                let target_group = target_gid.and_then(|tg| meta.groups.get(&tg));

                let mut source_master: Option<NodeEndpoint> = None;
                let mut target_master: Option<NodeEndpoint> = None;

                if let Some(g) = source_group {
                    for r in &g.replicas {
                        if r.is_leader {
                            source_master = resolve_endpoint(mgr, r.node_id);
                            break;
                        }
                    }
                }
                if let Some(g) = target_group {
                    for r in &g.replicas {
                        if r.is_leader {
                            target_master = resolve_endpoint(mgr, r.node_id);
                            break;
                        }
                    }
                }

                match (source_master, target_master) {
                    (Some(src), Some(dst)) => {
                        ranges.push(SlotRangeInfo {
                            start,
                            end,
                            master: src,
                            replicas: vec![NodeEndpoint {
                                host: dst.host.clone(),
                                port: dst.port,
                                node_id: dst.node_id,
                            }],
                        });
                    }
                    (Some(master), None) => {
                        ranges.push(SlotRangeInfo {
                            start,
                            end,
                            master,
                            replicas: vec![],
                        });
                    }
                    _ => {
                        // Neither node found, skip this range
                    }
                }
            }
            _ => {
                i += 1;
            }
        }
    }
    Ok(ranges)
}

// ---------------------------------------------------------------------------
// CLUSTER SHARDS (Redis 7.0+)
// ---------------------------------------------------------------------------
#[tracing::instrument(name = "cmd_cluster_shards", skip_all)]
pub fn cluster_shards() -> Result<RespValue, String> {
    let mgr = CLUSTER_STATE_MGR
        .get()
        .ok_or_else(|| "CLUSTERDOWN Cluster not initialized".to_string())?;
    let meta = mgr.meta_raft.get_cluster_meta();
    let slot_table = mgr.meta_raft.get_slot_table();

    // 按 Group 聚合槽范围
    let mut group_ranges: Vec<(u64, Vec<(u16, u16)>)> = Vec::new();
    let mut i = 0usize;
    while i < slot_table.len() {
        match slot_table[i] {
            SlotStatus::Assigned(gid) => {
                let start = i as u16;
                while i < slot_table.len() && slot_table[i] == SlotStatus::Assigned(gid) {
                    i += 1;
                }
                let end = (i - 1) as u16;
                match group_ranges.iter_mut().find(|(g, _)| *g == gid) {
                    Some((_, ranges)) => ranges.push((start, end)),
                    None => group_ranges.push((gid, vec![(start, end)])),
                }
            }
            _ => {
                i += 1;
            }
        }
    }

    // 构建 RESP 响应: 每个 shard 是一个 map { "slots": [...], "nodes": [...] }
    let mut shards: Vec<RespValue> = Vec::new();
    for (gid, ranges) in &group_ranges {
        let group = meta.groups.get(gid);
        let primary = group.and_then(|g| g.replicas.iter().find(|r| r.is_leader));
        let primary_id = primary.map(|r| r.node_id);
        let endpoint = primary_id.and_then(|nid| resolve_endpoint(mgr, nid));

        // slots 数组: [[start, end], ...]
        let slots_array: Vec<RespValue> = ranges
            .iter()
            .map(|(s, e)| {
                RespValue::Array(Some(vec![
                    RespValue::Integer(*s as i64),
                    RespValue::Integer(*e as i64),
                ]))
            })
            .collect();

        // nodes 数组: 每个 node 是一个 map
        let mut nodes: Vec<RespValue> = Vec::new();
        if let (Some(id), Some(ep)) = (primary_id, endpoint) {
            let node_id_hex = format!("{:040x}", id);
            nodes.push(RespValue::Array(Some(vec![
                RespValue::BulkString(Some(Bytes::from("id"))),
                RespValue::BulkString(Some(Bytes::from(node_id_hex))),
                RespValue::BulkString(Some(Bytes::from("port"))),
                RespValue::Integer(ep.port as i64),
                RespValue::BulkString(Some(Bytes::from("ip"))),
                RespValue::BulkString(Some(Bytes::from(ep.host.clone()))),
                RespValue::BulkString(Some(Bytes::from("endpoint"))),
                RespValue::BulkString(Some(Bytes::from(ep.host))),
                RespValue::BulkString(Some(Bytes::from("role"))),
                RespValue::BulkString(Some(Bytes::from("master"))),
                RespValue::BulkString(Some(Bytes::from("replication-offset"))),
                RespValue::Integer(0),
                RespValue::BulkString(Some(Bytes::from("health"))),
                RespValue::BulkString(Some(Bytes::from("online"))),
            ])));
        }

        // shard map: "slots" → slot_ranges, "nodes" → nodes
        shards.push(RespValue::Array(Some(vec![
            RespValue::BulkString(Some(Bytes::from("slots"))),
            RespValue::Array(Some(slots_array)),
            RespValue::BulkString(Some(Bytes::from("nodes"))),
            RespValue::Array(Some(nodes)),
        ])));
    }

    Ok(RespValue::Array(Some(shards)))
}

// ---------------------------------------------------------------------------
// CLUSTER MYSHARDID
// ---------------------------------------------------------------------------
#[tracing::instrument(name = "cmd_cluster_myshardid", skip_all)]
pub fn cluster_myshardid() -> Result<String, String> {
    let mgr = CLUSTER_STATE_MGR
        .get()
        .ok_or_else(|| "CLUSTERDOWN Cluster not initialized".to_string())?;
    let meta = mgr.meta_raft.get_cluster_meta();
    for (gid, group) in &meta.groups {
        if group.replicas.iter().any(|r| r.node_id == mgr.node_id) {
            return Ok(format!("{:040x}", gid));
        }
    }
    Err("CLUSTERDOWN Node not in any group".to_string())
}

// ---------------------------------------------------------------------------
// CLUSTER COUNTKEYSINSLOT
// ---------------------------------------------------------------------------
#[tracing::instrument(name = "cmd_cluster_count_keys_in_slot", skip_all)]
pub fn cluster_count_keys_in_slot(slot: u16) -> Result<i64, String> {
    let mgr = match CLUSTER_STATE_MGR.get() {
        Some(m) => m,
        None => return Err("CLUSTERDOWN Cluster not initialized".to_string()),
    };
    let handle = match tokio::runtime::Handle::try_current() {
        Ok(h) => h,
        Err(_) => return Err("ERR no async runtime".to_string()),
    };
    let mut total = 0i64;
    for gid in mgr.multi_raft.get_groups().read().keys() {
        if let Ok(keys) = handle.block_on(mgr.multi_raft.scan_keys(*gid, None)) {
            total += keys
                .iter()
                .filter(|k| aidb::cluster::router::key_to_slot(k) == slot)
                .count() as i64;
        }
    }
    Ok(total)
}

// ---------------------------------------------------------------------------
// CLUSTER GETKEYSINSLOT
// ---------------------------------------------------------------------------
#[tracing::instrument(name = "cmd_cluster_get_keys_in_slot", skip_all)]
pub fn cluster_get_keys_in_slot(slot: u16, count: usize) -> Result<Vec<Vec<u8>>, String> {
    let mgr = match CLUSTER_STATE_MGR.get() {
        Some(m) => m,
        None => return Err("CLUSTERDOWN Cluster not initialized".to_string()),
    };
    let handle = match tokio::runtime::Handle::try_current() {
        Ok(h) => h,
        Err(_) => return Err("ERR no async runtime".to_string()),
    };
    let mut keys = Vec::new();
    for gid in mgr.multi_raft.get_groups().read().keys() {
        if keys.len() >= count {
            break;
        }
        if let Ok(found) = handle.block_on(mgr.multi_raft.scan_keys(*gid, None)) {
            for k in found {
                if aidb::cluster::router::key_to_slot(&k) == slot {
                    keys.push(k);
                    if keys.len() >= count {
                        break;
                    }
                }
            }
        }
    }
    Ok(keys)
}

// ---------------------------------------------------------------------------
// Helper: parse 40-char hex node_id into u64
// ---------------------------------------------------------------------------
pub fn parse_hex_node_id(hex: &str) -> Result<u64, String> {
    let hex = hex.trim_start_matches("0x");
    u64::from_str_radix(hex, 16).map_err(|_| "ERR invalid node id".to_string())
}

/// Accept decimal node ids (factory/bootstrap) or 40-char hex ids (redis-cli style).
pub fn parse_cluster_node_id(raw: &str) -> Result<u64, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("ERR invalid node id".to_string());
    }
    if raw.chars().all(|c| c.is_ascii_digit()) {
        raw.parse::<u64>()
            .map_err(|_| "ERR invalid node id".to_string())
    } else {
        parse_hex_node_id(raw)
    }
}

// ---------------------------------------------------------------------------
// CLUSTER MEET
// ---------------------------------------------------------------------------
#[tracing::instrument(name = "cmd_cluster_meet", skip_all)]
pub async fn cluster_meet(
    addr: &str,
    port: u16,
    client_port: Option<u16>,
    client_host: Option<&str>,
) -> Result<String, String> {
    let mut last_err = String::new();
    let mut delay_ms = 100u64;
    for attempt in 1u32..=10 {
        let mgr = CLUSTER_STATE_MGR
            .get()
            .ok_or_else(|| "CLUSTERDOWN Cluster not initialized".to_string())?;
        let coordinator = mgr
            .membership_coordinator
            .as_ref()
            .ok_or_else(|| "ERR Membership coordinator not available".to_string())?;
        let meta = mgr.meta_raft.get_cluster_meta();
        let next_id = meta.nodes.keys().max().unwrap_or(&0) + 1;
        // client_addr 使用独立的外部可达 host, 与内部 RPC 地址分离
        let client_host = client_host.unwrap_or(addr);
        let client_addr = client_port.map(|cp| format!("{client_host}:{cp}"));
        match coordinator
            .add_node(membership_coordinator::NodeJoinContext {
                node_id: next_id,
                rpc_addr: format!("{addr}:{port}"),
                client_addr,
                join_method: membership_coordinator::JoinMethod::Empty,
            })
            .await
        {
            Ok(()) => return Ok("OK".to_string()),
            Err(e) => {
                let is_not_leader =
                    matches!(&e, AidbError::Cluster(ClusterError::NotLeader { .. }));
                last_err = map_propose_error(e);
                if is_not_leader && attempt < 10 {
                    sleep(Duration::from_millis(delay_ms)).await;
                    delay_ms = delay_ms.saturating_mul(2).min(2000);
                    continue;
                }
                return Err(last_err);
            }
        }
    }
    Err(last_err)
}

// ---------------------------------------------------------------------------
// CLUSTER FORGET
// ---------------------------------------------------------------------------
#[tracing::instrument(name = "cmd_cluster_forget", skip_all)]
pub async fn cluster_forget(hex_node_id: &str, force: bool) -> Result<String, String> {
    let mgr = CLUSTER_STATE_MGR
        .get()
        .ok_or_else(|| "CLUSTERDOWN Cluster not initialized".to_string())?;
    let coordinator = mgr
        .membership_coordinator
        .as_ref()
        .ok_or_else(|| "ERR Membership coordinator not available".to_string())?;
    let node_id = parse_hex_node_id(hex_node_id)?;
    coordinator
        .remove_node(membership_coordinator::NodeLeaveContext { node_id, force })
        .await
        .map_err(map_propose_error)?;
    Ok("OK".to_string())
}

/// Parse optional `FORCE` flag for `CLUSTER FORGET <node-id> [FORCE]`.
pub fn parse_forget_force(arg: Option<&[u8]>) -> Result<bool, String> {
    match arg {
        None => Ok(false),
        Some(raw) => {
            let flag = std::str::from_utf8(raw)
                .map_err(|_| "ERR invalid FORGET option".to_string())?
                .trim();
            if flag.eq_ignore_ascii_case("force") {
                Ok(true)
            } else {
                Err(format!("ERR unknown FORGET option '{flag}'"))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// CLUSTER SAVECONFIG — 持久化集群配置到文件
// ---------------------------------------------------------------------------

/// Persist the current cluster config to nodes.conf.
/// Returns Ok(()) on success, or an error string.
pub fn save_nodes_conf(
    meta_raft: &aidb::cluster::MetaRaftNode,
    data_dir: &std::path::Path,
) -> Result<(), String> {
    std::fs::create_dir_all(data_dir).map_err(|e| format!("ERR cannot create data dir: {e}"))?;

    let meta = meta_raft.get_cluster_meta();
    let slot_table = meta_raft.get_slot_table();

    let config_path = data_dir.join("nodes.conf");
    // Serialize cluster config as JSON for inspectability
    let config_json = serde_json::to_string_pretty(&serde_json::json!({
      "version": meta.version,
      "nodes": meta.nodes.iter().map(|(id, n)| serde_json::json!({
        "id": id,
        "rpc_addr": n.rpc_addr,
        "client_addr": n.client_addr,
        "status": format!("{:?}", n.status),
        "role": format!("{:?}", n.role),
      })).collect::<Vec<_>>(),
      "groups": meta.groups.iter().map(|(gid, g)| serde_json::json!({
        "group_id": gid,
        "config_version": g.config_version,
        "replicas": g.replicas.iter().map(|r| serde_json::json!({
          "node_id": r.node_id,
          "is_leader": r.is_leader,
        })).collect::<Vec<_>>(),
      })).collect::<Vec<_>>(),
      "slot_table": slot_table.iter().enumerate().filter_map(|(i, s)| {
        match s {
          aidb::cluster::meta_types::SlotStatus::Assigned(gid) => {
            Some(serde_json::json!({"slot": i, "group_id": gid}))
          }
          _ => None,
        }
      }).collect::<Vec<_>>(),
    }))
    .map_err(|e| format!("ERR serialization: {e}"))?;

    // Atomic write: write to temp -> fsync -> rename
    let tmp_path = config_path.with_extension("tmp");
    std::fs::write(&tmp_path, &config_json).map_err(|e| format!("ERR write config: {e}"))?;
    std::fs::rename(&tmp_path, &config_path).map_err(|e| format!("ERR rename config: {e}"))?;
    // fsync the directory to ensure the rename is durable
    if let Some(parent) = config_path.parent() {
        if let Ok(dir) = std::fs::File::open(parent) {
            let _ = dir.sync_all();
        }
    }
    tracing::info!(path = %config_path.display(), "cluster config saved");
    Ok(())
}

#[tracing::instrument(name = "cmd_cluster_saveconfig", skip_all)]
fn cluster_saveconfig() -> Result<String, String> {
    let mgr = CLUSTER_STATE_MGR
        .get()
        .ok_or_else(|| "CLUSTERDOWN Cluster not initialized".to_string())?;
    let dir = mgr
        .data_dir
        .as_ref()
        .ok_or_else(|| "ERR data_dir not set".to_string())?;
    save_nodes_conf(&mgr.meta_raft, dir)?;
    Ok("OK".to_string())
}

// ---------------------------------------------------------------------------
// CLUSTER ADDSLOTS
// ---------------------------------------------------------------------------
#[tracing::instrument(name = "cmd_cluster_add_slots", skip_all)]
pub async fn cluster_add_slots(
    slots: &[u16],
    target_node_id: Option<u64>,
) -> Result<String, String> {
    let mgr = CLUSTER_STATE_MGR
        .get()
        .ok_or_else(|| "CLUSTERDOWN Cluster not initialized".to_string())?;

    let effective_node = target_node_id.unwrap_or(mgr.node_id);
    let group_id = {
        let meta = mgr.meta_raft.get_cluster_meta();
        match meta
            .groups
            .iter()
            .find(|(_, g)| g.replicas.iter().any(|r| r.node_id == effective_node))
            .map(|(id, _)| *id)
        {
            Some(gid) => gid,
            None => {
                // 自动创建分片组: node_id 作为 group_id
                let gid = effective_node;
                mgr.meta_raft
                    .propose(aidb::cluster::MetaRequest::CreateGroup {
                        group_id: gid,
                        initial_replicas: vec![(effective_node, true)],
                    })
                    .await
                    .map_err(map_propose_error)?;
                gid
            }
        }
    };

    mgr.meta_raft
        .propose(aidb::cluster::MetaRequest::AssignSlots {
            group_id,
            slots: slots.to_vec(),
        })
        .await
        .map_err(map_propose_error)?;

    // Promote target node to Voter so CLUSTER NODES shows correct
    // "master" role instead of "slave" (nodes join as Learner via MEET).
    mgr.meta_raft
        .propose(aidb::cluster::MetaRequest::ChangeNodeRole {
            node_id: effective_node,
            role: aidb::cluster::NodeRole::Voter,
        })
        .await
        .map_err(map_propose_error)?;

    // Immediately refresh router cache so subsequent routing decisions
    // use the updated slot table instead of stale Unallocated entries.
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

// ---------------------------------------------------------------------------
// CLUSTER DELSLOTS
// ---------------------------------------------------------------------------
#[tracing::instrument(name = "cmd_cluster_del_slots", skip_all)]
pub async fn cluster_del_slots(slots: &[u16]) -> Result<String, String> {
    let mgr = CLUSTER_STATE_MGR
        .get()
        .ok_or_else(|| "CLUSTERDOWN Cluster not initialized".to_string())?;
    mgr.meta_raft
        .propose(aidb::cluster::MetaRequest::UnassignSlots {
            slots: slots.to_vec(),
        })
        .await
        .map_err(map_propose_error)?;
    Ok("OK".to_string())
}

// ---------------------------------------------------------------------------
// CLUSTER SETSLOT
// ---------------------------------------------------------------------------
#[tracing::instrument(name = "cmd_cluster_set_slot", skip_all)]
pub async fn cluster_set_slot(
    slot: u16,
    sub: &str,
    node_id: Option<u64>,
) -> Result<String, String> {
    let mgr = CLUSTER_STATE_MGR
        .get()
        .ok_or_else(|| "CLUSTERDOWN Cluster not initialized".to_string())?;
    match sub.to_uppercase().as_str() {
        "MIGRATING" => {
            let target_nid = node_id.ok_or_else(|| "ERR wrong number of arguments".to_string())?;
            let (source_gid, _) = mgr
                .router
                .route_slot(slot)
                .map_err(|e| format!("ERR {e}"))?;
            let meta = mgr.meta_raft.get_cluster_meta();
            let target_gid = meta
                .groups
                .iter()
                .find(|(_, g)| g.replicas.iter().any(|r| r.node_id == target_nid))
                .map(|(id, _)| *id)
                .ok_or_else(|| "ERR Target node not in any group".to_string())?;
            let sm = mgr
                .slot_migration_manager
                .as_ref()
                .ok_or_else(|| "ERR SlotMigrationManager not initialized".to_string())?;
            sm.start_migration(source_gid, target_gid, vec![slot])
                .await
                .map_err(|e| format!("ERR {e}"))?;
            Ok("OK".to_string())
        }
        "IMPORTING" => {
            let source_nid = node_id.ok_or_else(|| "ERR wrong number of arguments".to_string())?;
            mgr.importing_slots.write().insert(slot, source_nid);
            Ok("OK".to_string())
        }
        "NODE" => {
            let target_nid = node_id.ok_or_else(|| "ERR wrong number of arguments".to_string())?;
            let meta = mgr.meta_raft.get_cluster_meta();
            let target_gid = meta
                .groups
                .iter()
                .find(|(_, g)| g.replicas.iter().any(|r| r.node_id == target_nid))
                .map(|(id, _)| *id)
                .ok_or_else(|| "ERR Target node not in any group".to_string())?;
            mgr.meta_raft
                .propose(aidb::cluster::MetaRequest::AssignSlots {
                    group_id: target_gid,
                    slots: vec![slot],
                })
                .await
                .map_err(map_propose_error)?;
            Ok("OK".to_string())
        }
        "STABLE" => {
            // Clear local importing slot tracking
            mgr.importing_slots.write().remove(&slot);
            // F-056: 走完整收尾链 (freeze → quiesce → final_verify → mark_ready → commit).
            // 无活跃迁移时 finish_migration 会失败; 仅清 importing 仍返回 OK.
            if mgr.meta_raft.get_migration_state().is_some() {
                let sm = mgr
                    .slot_migration_manager
                    .as_ref()
                    .ok_or_else(|| "ERR SlotMigrationManager not initialized".to_string())?;
                sm.finish_migration()
                    .await
                    .map_err(|e| format!("ERR finish_migration: {e}"))?;
            }
            Ok("OK".to_string())
        }
        _ => Err("ERR Unknown SETSLOT subcommand".to_string()),
    }
}

// ---------------------------------------------------------------------------
// CLUSTER FAILOVER
// ---------------------------------------------------------------------------
#[tracing::instrument(name = "cmd_cluster_failover", skip_all)]
pub async fn cluster_failover(mode: &str) -> Result<String, String> {
    let mgr = CLUSTER_STATE_MGR
        .get()
        .ok_or_else(|| "CLUSTERDOWN Cluster not initialized".to_string())?;
    match mode.to_uppercase().as_str() {
        "FORCE" | "TAKEOVER" => {
            // Read role into local before any .await (parking_lot lock held across await is risky).
            let is_primary = matches!(
                *mgr.role.read(),
                crate::cluster::state::ReplicationRole::Primary
            );
            if is_primary {
                return Err("ERR You should be a replica".to_string());
            }
            let meta = mgr.meta_raft.get_cluster_meta();
            for (gid, group) in &meta.groups {
                if group.replicas.iter().any(|r| r.node_id == mgr.node_id) {
                    mgr.multi_raft
                        .change_group_membership(*gid, vec![mgr.node_id])
                        .await
                        .map_err(|e| format!("ERR {e}"))?;
                    *mgr.role.write() = crate::cluster::state::ReplicationRole::Primary;
                    if let Some(m) = &mgr.metrics {
                        m.on_failover();
                    }
                    return Ok("OK".to_string());
                }
            }
            Err("ERR Node not found in any group".to_string())
        }
        _ => Err("ERR Unknown FAILOVER mode".to_string()),
    }
}

// ---------------------------------------------------------------------------
// CLUSTER REPLICATE
// ---------------------------------------------------------------------------
//
// 设置本节点为指定 primary 的副本 (仅元数据层面).
// 实际的 MultiRaft 成员变更受限于 group 在本地才可操作,
// 当前版本 replicas 不服务数据读取, 这是已知限制.
#[tracing::instrument(name = "cmd_cluster_replicate", skip_all)]
pub async fn cluster_replicate(primary_id: u64) -> Result<String, String> {
    let mgr = CLUSTER_STATE_MGR
        .get()
        .ok_or_else(|| "CLUSTERDOWN Cluster not initialized".to_string())?;
    *mgr.role.write() = crate::cluster::state::ReplicationRole::Replica { primary_id };
    Ok("OK".to_string())
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

// ---------------------------------------------------------------------------
// CLUSTER REPLICAS
// ---------------------------------------------------------------------------
#[tracing::instrument(name = "cmd_cluster_replicas", skip_all)]
pub fn cluster_replicas(node_id: u64) -> Result<Vec<String>, String> {
    let mgr = CLUSTER_STATE_MGR
        .get()
        .ok_or_else(|| "CLUSTERDOWN Cluster not initialized".to_string())?;
    let meta = mgr.meta_raft.get_cluster_meta();
    let replicas: Vec<String> = meta
        .groups
        .iter()
        .filter(|(_, g)| {
            g.replicas
                .iter()
                .any(|r| r.node_id == node_id && r.is_leader)
        })
        .flat_map(|(_, g)| &g.replicas)
        .filter(|r| !r.is_leader)
        .map(|r| format!("{:040x}", r.node_id))
        .collect();
    Ok(replicas)
}

// ---------------------------------------------------------------------------
// CLUSTER REBALANCE
// ---------------------------------------------------------------------------
#[tracing::instrument(name = "cmd_cluster_rebalance", skip_all)]
pub async fn cluster_rebalance() -> Result<String, String> {
    let mgr = CLUSTER_STATE_MGR
        .get()
        .ok_or_else(|| "CLUSTERDOWN Cluster not initialized".to_string())?;

    let slot_table = mgr.meta_raft.get_slot_table();

    // Count slots per group (only assigned slots)
    let mut group_slots: std::collections::HashMap<u64, Vec<u16>> =
        std::collections::HashMap::new();
    for (slot_idx, status) in slot_table.iter().enumerate() {
        if let aidb::cluster::meta_types::SlotStatus::Assigned(gid) = status {
            group_slots.entry(*gid).or_default().push(slot_idx as u16);
        }
    }

    let group_count = group_slots.len();
    if group_count <= 1 {
        return Ok("No rebalance needed (0 or 1 groups)".to_string());
    }

    // Check no migration in progress
    if mgr.meta_raft.get_migration_state().is_some() {
        return Err("ERR migration already in progress".to_string());
    }

    // Compute ideal slots per group (from ReplicaAllocator)
    let ideal_ranges = ReplicaAllocator::suggest_slot_allocation(group_count);
    let ideal_counts: Vec<usize> = ideal_ranges
        .iter()
        .map(|ranges| {
            ranges
                .iter()
                .map(|(start, end)| (end - start + 1) as usize)
                .sum()
        })
        .collect();

    // Map group_ids to a sorted list for matching against ideal_counts
    let mut sorted_gids: Vec<u64> = group_slots.keys().copied().collect();
    sorted_gids.sort();

    // Build surplus/deficit lists by comparing current vs ideal
    let per_group = 16384 / group_count;
    let mut deficits: Vec<(u64, usize)> = Vec::new();
    let mut surpluses: Vec<(u64, Vec<u16>)> = Vec::new();

    for (i, &gid) in sorted_gids.iter().enumerate() {
        let current = group_slots.get(&gid).map(|s| s.len()).unwrap_or(0);
        let ideal = ideal_counts.get(i).copied().unwrap_or(per_group);
        if current > ideal {
            let mut slots = group_slots[&gid].clone();
            slots.sort();
            let excess = current - ideal;
            surpluses.push((gid, slots[slots.len() - excess..].to_vec()));
        } else if current < ideal {
            deficits.push((gid, ideal - current));
        }
    }

    // Execute migrations greedily
    let sm = mgr
        .slot_migration_manager
        .as_ref()
        .ok_or_else(|| "ERR SlotMigrationManager not initialized".to_string())?;

    let mut total_migrated: u64 = 0;

    for (target_gid, mut needed) in deficits {
        while needed > 0 && !surpluses.is_empty() {
            let (src_gid, ref mut surplus_slots) = surpluses[0];
            let take = needed.min(surplus_slots.len());
            let migrate_slots: Vec<u16> = surplus_slots.drain(..take).collect();
            needed -= take;

            // Execute migration
            let migration_id = sm
                .start_migration(src_gid, target_gid, migrate_slots.clone())
                .await
                .map_err(|e| format!("ERR start_migration: {e}"))?;

            // Build ActiveMigration from migration state
            let migration_state = mgr
                .meta_raft
                .get_migration_state()
                .ok_or_else(|| "ERR migration state lost".to_string())?;
            let (src, dst, slots) = match &migration_state {
                aidb::cluster::meta_types::SlotMigrationState::Prepare {
                    source_group,
                    target_group,
                    slots,
                    ..
                } => (*source_group, *target_group, slots.clone()),
                _ => return Err("ERR unexpected migration state".to_string()),
            };

            let active = aidb::cluster::slot_migration::ActiveMigration {
                migration_id,
                source_group: src,
                target_group: dst,
                slots,
                checkpoint: Vec::new(),
            };

            let result = sm
                .run_pending_migration(active)
                .await
                .map_err(|e| format!("ERR run_migration: {e}"))?;

            if !result.is_completed {
                return Err(format!(
                    "ERR migration incomplete: {} keys migrated",
                    result.migrated_count
                ));
            }

            // F-056: 完整收尾链, 失败返回 ERR (不得静默跳过).
            sm.finish_migration()
                .await
                .map_err(|e| format!("ERR finish_migration: {e}"))?;

            total_migrated += migrate_slots.len() as u64;

            if surplus_slots.is_empty() {
                surpluses.remove(0);
            }
        }
    }

    Ok(format!(
        "OK {} slots rebalanced across {} groups",
        total_migrated, group_count
    ))
}

// ---------------------------------------------------------------------------
// CLUSTER subcommand dispatch
// ---------------------------------------------------------------------------
/// CLUSTER 子命令分发器. 所有 CLUSTER 子命令统一入口,
/// 避免 execute_inner 的 match 进一步膨胀.
#[tracing::instrument(name = "cmd_cluster", skip_all, fields(sub = sub))]
pub async fn dispatch_cluster(
    sub: Option<&str>,
    args: &[Bytes],
) -> crate::error::Result<RespValue> {
    // Read-only subcommands
    match sub {
        Some("keyslot") | Some("KEYSLOT") => {
            let key = args
                .get(1)
                .ok_or_else(|| Error::Command("ERR wrong number of arguments".into()))?;
            let slot = cluster_keyslot(key).map_err(Error::Command)?;
            Ok(RespValue::Integer(
                slot.parse::<i64>()
                    .map_err(|_| Error::Command("ERR invalid slot".into()))?,
            ))
        }
        Some("myid") | Some("MYID") => {
            let id = cluster_myid().map_err(Error::Command)?;
            Ok(RespValue::BulkString(Some(Bytes::from(id))))
        }
        Some("groupstatus") | Some("GROUPSTATUS") => {
            let gid: Option<u64> = args.get(1).and_then(|a| parse_int(a));
            let info = cluster_groupstatus(gid).map_err(Error::Command)?;
            Ok(RespValue::BulkString(Some(Bytes::from(info))))
        }
        Some("info") | Some("INFO") => {
            let info = cluster_info().map_err(Error::Command)?;
            Ok(RespValue::BulkString(Some(Bytes::from(info))))
        }
        Some("nodes") | Some("NODES") => {
            let nodes = cluster_nodes().map_err(Error::Command)?;
            Ok(RespValue::BulkString(Some(Bytes::from(nodes))))
        }
        Some("slots") | Some("SLOTS") => {
            let slots = cluster_slots().map_err(Error::Command)?;
            let array: Vec<RespValue> = slots
                .into_iter()
                .map(|range| {
                    let mut entry = vec![
                        RespValue::Integer(range.start as i64),
                        RespValue::Integer(range.end as i64),
                        RespValue::Array(Some(vec![
                            RespValue::BulkString(Some(Bytes::from(range.master.host))),
                            RespValue::Integer(range.master.port as i64),
                            RespValue::BulkString(Some(Bytes::from(format!(
                                "{:040x}",
                                range.master.node_id
                            )))),
                        ])),
                    ];
                    // 追加 replica 条目
                    for replica in &range.replicas {
                        entry.push(RespValue::Array(Some(vec![
                            RespValue::BulkString(Some(Bytes::from(replica.host.clone()))),
                            RespValue::Integer(replica.port as i64),
                            RespValue::BulkString(Some(Bytes::from(format!(
                                "{:040x}",
                                replica.node_id
                            )))),
                        ])));
                    }
                    RespValue::Array(Some(entry))
                })
                .collect();
            Ok(RespValue::Array(Some(array)))
        }
        Some("shards") | Some("SHARDS") => cluster_shards().map_err(Error::Command),
        Some("myshardid") | Some("MYSHARDID") => {
            let id = cluster_myshardid().map_err(Error::Command)?;
            Ok(RespValue::BulkString(Some(Bytes::from(id))))
        }
        Some("countkeysinslot") | Some("COUNTKEYSINSLOT") => {
            let slot: u16 = args
                .get(1)
                .and_then(|a| parse_int(a))
                .ok_or_else(|| Error::Command("ERR invalid slot".into()))?;
            let count = cluster_count_keys_in_slot(slot).map_err(Error::Command)?;
            Ok(RespValue::Integer(count))
        }
        Some("getkeysinslot") | Some("GETKEYSINSLOT") => {
            let slot: u16 = args
                .get(1)
                .and_then(|a| parse_int(a))
                .ok_or_else(|| Error::Command("ERR invalid slot".into()))?;
            let count: usize = args
                .get(2)
                .and_then(|a| parse_int(a))
                .ok_or_else(|| Error::Command("ERR invalid count".into()))?;
            let keys = cluster_get_keys_in_slot(slot, count).map_err(Error::Command)?;
            Ok(RespValue::Array(Some(
                keys.into_iter()
                    .map(|k| RespValue::BulkString(Some(Bytes::from(k))))
                    .collect(),
            )))
        }
        // Write commands
        Some("meet") | Some("MEET") => {
            // Redis 标准: CLUSTER MEET <host> <client_port> [rpc_port] [client_host]
            // - Arg 2: client_port (Redis 标准, 用于 MOVED 重定向)
            // - Arg 3 (可选): rpc_port; 未提供时自动推导为 client_port + 10000
            // - Arg 4 (可选): client_host (Docker 等场景的外部可达地址)
            let addr = args
                .get(1)
                .ok_or_else(|| Error::Command("ERR wrong number of arguments".into()))?;
            let client_port_str = args
                .get(2)
                .ok_or_else(|| Error::Command("ERR wrong number of arguments".into()))?;
            let client_port: u16 = parse_int(client_port_str)
                .ok_or_else(|| Error::Command("ERR invalid client port".into()))?;
            let addr_str = bytes_to_str(addr)?;
            // 可选第 3 参数: RPC 端口; 默认 client_port + 10000 (Redis Cluster 惯例)
            let rpc_port: u16 = args
                .get(3)
                .and_then(|a| parse_int(a))
                .unwrap_or(client_port + 10000);
            // 可选第 4 参数: 客户端可达 host (Docker/容器场景)
            let client_host: Option<&str> = args.get(4).and_then(|a| bytes_to_str(a).ok());
            let msg = cluster_meet(addr_str, rpc_port, Some(client_port), client_host)
                .await
                .map_err(Error::Command)?;
            Ok(RespValue::SimpleString(msg))
        }
        Some("forget") | Some("FORGET") => {
            let hex_id = args
                .get(1)
                .map(|a| bytes_to_str(a))
                .ok_or_else(|| Error::Command("ERR wrong number of arguments".into()))??;
            let force =
                parse_forget_force(args.get(2).map(|a| a.as_ref())).map_err(Error::Command)?;
            let msg = cluster_forget(hex_id, force)
                .await
                .map_err(Error::Command)?;
            Ok(RespValue::SimpleString(msg))
        }
        Some("addslots") | Some("ADDSLOTS") => {
            // Optional "NODE <id>" prefix for assigning slots to a specific node.
            let (target_node_id, slot_start) = if args.len() > 2 {
                match bytes_to_str(&args[1]) {
                    Ok(s) if s.eq_ignore_ascii_case("node") => {
                        let nid = bytes_to_str(&args[2])
                            .ok()
                            .and_then(|s| s.parse::<u64>().ok());
                        (nid, 3usize)
                    }
                    _ => (None, 1usize),
                }
            } else {
                (None, 1usize)
            };
            let slots: Vec<u16> = args[slot_start..]
                .iter()
                .filter_map(|a| parse_int(a))
                .collect();
            let msg = cluster_add_slots(&slots, target_node_id)
                .await
                .map_err(Error::Command)?;
            Ok(RespValue::SimpleString(msg))
        }
        Some("delslots") | Some("DELSLOTS") => {
            let slots: Vec<u16> = args[1..].iter().filter_map(|a| parse_int(a)).collect();
            let msg = cluster_del_slots(&slots).await.map_err(Error::Command)?;
            Ok(RespValue::SimpleString(msg))
        }
        Some("setslot") | Some("SETSLOT") => {
            let slot: u16 = args
                .get(1)
                .and_then(|a| parse_int(a))
                .ok_or_else(|| Error::Command("ERR invalid slot".into()))?;
            let sub = match args.get(2).map(|a| bytes_to_str(a)) {
                Some(Ok(s)) => s,
                _ => "",
            };
            let node_id = args.get(3).and_then(|a| parse_int::<u64>(a));
            let msg = cluster_set_slot(slot, sub, node_id)
                .await
                .map_err(Error::Command)?;
            Ok(RespValue::SimpleString(msg))
        }
        Some("failover") | Some("FAILOVER") => {
            let mode = match args.get(1).map(|a| bytes_to_str(a)) {
                Some(Ok(s)) => s,
                _ => "FORCE",
            };
            let msg = cluster_failover(mode).await.map_err(Error::Command)?;
            Ok(RespValue::SimpleString(msg))
        }
        Some("replicate") | Some("REPLICATE") => {
            let hex_id = args
                .get(1)
                .map(|a| bytes_to_str(a))
                .ok_or_else(|| Error::Command("ERR wrong number of arguments".into()))??;
            let primary_id = parse_hex_node_id(hex_id).map_err(Error::Command)?;
            let msg = cluster_replicate(primary_id)
                .await
                .map_err(Error::Command)?;
            Ok(RespValue::SimpleString(msg))
        }
        // 用法: CLUSTER CREATEGROUP <primary-id> [group-id]
        Some("creategroup") | Some("CREATEGROUP") => {
            let primary_raw = args
                .get(1)
                .map(|a| bytes_to_str(a))
                .ok_or_else(|| Error::Command("ERR wrong number of arguments".into()))??;
            let primary_id = parse_cluster_node_id(primary_raw).map_err(Error::Command)?;
            let group_id = match args.get(2) {
                Some(raw) => {
                    Some(parse_cluster_node_id(bytes_to_str(raw)?).map_err(Error::Command)?)
                }
                None => None,
            };
            let msg = cluster_create_group(primary_id, group_id)
                .await
                .map_err(Error::Command)?;
            Ok(RespValue::SimpleString(msg))
        }
        // ── 内部命令: 从主节点添加副本 ──
        // 用法: CLUSTER ADD_REPLICA <primary_id> <replica_id>
        Some("add_replica") | Some("ADD_REPLICA") => {
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
        // 用法: CLUSTER DEL_REPLICA <primary_id> <replica_id>
        Some("del_replica") | Some("DEL_REPLICA") => {
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
        Some("replicas") | Some("REPLICAS") => {
            let hex_id = args
                .get(1)
                .map(|a| bytes_to_str(a))
                .ok_or_else(|| Error::Command("ERR wrong number of arguments".into()))??;
            let node_id = parse_hex_node_id(hex_id).map_err(Error::Command)?;
            let replicas = cluster_replicas(node_id).map_err(Error::Command)?;
            Ok(RespValue::Array(Some(
                replicas
                    .into_iter()
                    .map(|r| RespValue::BulkString(Some(Bytes::from(r))))
                    .collect(),
            )))
        }
        Some("saveconfig") | Some("SAVECONFIG") => {
            cluster_saveconfig().map_err(Error::Command)?;
            Ok(RespValue::SimpleString("OK".into()))
        }
        Some("bumpepoch") | Some("BUMPEPOCH") => {
            let mgr = CLUSTER_STATE_MGR
                .get()
                .ok_or_else(|| Error::Command("CLUSTERDOWN Cluster not initialized".into()))?;
            // Propose BumpEpoch through MetaRaft consensus
            use aidb::cluster::meta_types::MetaRequest;
            let _response = mgr
                .meta_raft
                .propose(MetaRequest::BumpEpoch)
                .await
                .map_err(|e| Error::Command(format!("ERR {e}")))?;
            // After consensus, read the new epoch from the state machine
            let new_epoch = mgr.meta_raft.get_cluster_meta().version;
            Ok(RespValue::BulkString(Some(Bytes::from(
                new_epoch.to_string(),
            ))))
        }
        Some("set-config-epoch") | Some("SET-CONFIG-EPOCH") => {
            Ok(RespValue::SimpleString("OK".into()))
        }
        Some("count-failure-reports") | Some("COUNT-FAILURE-REPORTS") => Ok(RespValue::Integer(0)),
        Some("rebalance") | Some("REBALANCE") => {
            let msg = cluster_rebalance().await.map_err(Error::Command)?;
            Ok(RespValue::BulkString(Some(Bytes::from(msg))))
        }
        Some("reset") | Some("RESET") => {
            if let Some(mode) = args.get(1) {
                let mode_str = bytes_to_str(mode)?;
                if !mode_str.eq_ignore_ascii_case("soft") && !mode_str.eq_ignore_ascii_case("hard")
                {
                    return Err(Error::Command(format!(
                        "ERR unknown RESET option '{mode_str}'"
                    )));
                }
            }
            Err(Error::Command(
                "ERR CLUSTER RESET is not supported; stop the node and clear data_dir (see docs/modules/cluster.md)".into(),
            ))
        }
        #[cfg(feature = "cluster-test-util")]
        Some("failpoint") | Some("FAILPOINT") => {
            let result = cluster_failpoint(args).map_err(Error::Command)?;
            Ok(RespValue::BulkString(Some(Bytes::from(result))))
        }
        _ => Err(Error::Command("ERR unknown CLUSTER subcommand".into())),
    }
}

#[cfg(test)]
mod cluster_nodes_role_tests {
    use super::cluster_node_role_label;
    use aidb::cluster::meta_types::{
        ClusterMeta, GroupMeta, NodeInfo, NodeRole, NodeStatus, ReplicaInfo,
    };
    use std::collections::HashMap;

    fn node(id: u64) -> NodeInfo {
        NodeInfo {
            node_id: id,
            rpc_addr: format!("h:{id}"),
            client_addr: None,
            role: NodeRole::Voter,
            status: NodeStatus::Online,
            registered_at: 0,
            tags: HashMap::new(),
        }
    }

    #[test]
    fn cluster_nodes_link_state_from_meta_status() {
        assert_eq!(
            super::cluster_nodes_link_state(&NodeStatus::Online),
            "connected"
        );
        assert_eq!(
            super::cluster_nodes_link_state(&NodeStatus::Draining),
            "connected"
        );
        assert_eq!(
            super::cluster_nodes_link_state(&NodeStatus::Offline),
            "disconnected"
        );
    }

    #[test]
    fn replica_voter_still_shows_slave() {
        let meta = ClusterMeta {
            groups: HashMap::from([(
                1,
                GroupMeta {
                    group_id: 1,
                    replicas: vec![
                        ReplicaInfo {
                            node_id: 1,
                            is_leader: true,
                        },
                        ReplicaInfo {
                            node_id: 2,
                            is_leader: false,
                        },
                    ],
                    slot_ranges: vec![(0, 100)],
                    config_version: 1,
                },
            )]),
            nodes: HashMap::from([(1, node(1)), (2, node(2))]),
            ..ClusterMeta::default()
        };
        assert_eq!(cluster_node_role_label(&meta, 1), "master");
        assert_eq!(cluster_node_role_label(&meta, 2), "slave");
    }
}

#[cfg(test)]
mod cluster_info_state_tests {
    use super::compute_cluster_state;
    use aidb::cluster::meta_types::{
        default_slot_table, ClusterMeta, GroupMeta, ReplicaInfo, SlotStatus,
    };
    use std::collections::HashMap;

    fn meta_with_group(group_id: u64, has_leader: bool) -> ClusterMeta {
        ClusterMeta {
            groups: HashMap::from([(
                group_id,
                GroupMeta {
                    group_id,
                    replicas: vec![ReplicaInfo {
                        node_id: group_id,
                        is_leader: has_leader,
                    }],
                    slot_ranges: vec![],
                    config_version: 1,
                },
            )]),
            ..ClusterMeta::default()
        }
    }

    fn full_slot_table(group_id: u64) -> Vec<SlotStatus> {
        let mut table = default_slot_table();
        for slot in table.iter_mut() {
            *slot = SlotStatus::Assigned(group_id);
        }
        table
    }

    #[test]
    fn cluster_state_fail_partial_slots() {
        let mut table = default_slot_table();
        for slot in &mut table[0..5] {
            *slot = SlotStatus::Assigned(1);
        }
        let meta = meta_with_group(1, true);
        assert_eq!(compute_cluster_state(&table, &meta, |_| Some(1)), "fail");
    }

    #[test]
    fn cluster_state_fail_no_leader() {
        let table = full_slot_table(1);
        let meta = meta_with_group(1, false);
        assert_eq!(compute_cluster_state(&table, &meta, |_| None), "fail");
    }

    #[test]
    fn cluster_state_ok_healthy() {
        let table = full_slot_table(1);
        let meta = meta_with_group(1, true);
        assert_eq!(compute_cluster_state(&table, &meta, |_| Some(1)), "ok");
    }

    #[test]
    fn cluster_state_fail_orphan_slot() {
        let table = full_slot_table(99);
        let meta = meta_with_group(1, true);
        assert_eq!(compute_cluster_state(&table, &meta, |_| Some(99)), "fail");
    }

    #[test]
    fn cluster_state_ok_counts_migrating_as_covered() {
        let mut table = full_slot_table(1);
        table[0] = SlotStatus::Migrating(1);
        let meta = meta_with_group(1, true);
        assert_eq!(compute_cluster_state(&table, &meta, |_| Some(1)), "ok");
    }
}

#[cfg(test)]
mod cluster_reset_dispatch_tests {
    use super::dispatch_cluster;
    use bytes::Bytes;

    fn b(s: &str) -> Bytes {
        Bytes::from(s.to_owned())
    }

    #[tokio::test]
    async fn reset_soft_returns_explicit_err() {
        let err = dispatch_cluster(Some("RESET"), &[b("RESET"), b("SOFT")])
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("not supported"));
        assert!(msg.contains("data_dir"));
    }

    #[tokio::test]
    async fn reset_default_mode_returns_explicit_err() {
        let err = dispatch_cluster(Some("RESET"), &[b("RESET")])
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not supported"));
    }

    #[tokio::test]
    async fn reset_hard_returns_explicit_err() {
        let err = dispatch_cluster(Some("RESET"), &[b("RESET"), b("HARD")])
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not supported"));
    }

    #[tokio::test]
    async fn reset_unknown_option() {
        let err = dispatch_cluster(Some("RESET"), &[b("RESET"), b("WTF")])
            .await
            .unwrap_err();
        assert!(err.to_string().contains("unknown RESET option"));
    }
}
