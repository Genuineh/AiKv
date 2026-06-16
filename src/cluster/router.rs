use std::collections::HashMap;

use aidb::cluster::router::key_to_slot;
use aidb::cluster::SlotStatus;

use crate::cluster::state::{ClusterStateManager, CLUSTER_STATE_MGR};

/// 集群路由决策结果
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteDecision {
    Execute,
    Moved {
        slot: u16,
        node_id: u64,
        addr: String,
    },
    Ask {
        slot: u16,
        node_id: u64,
        addr: String,
    },
    ClusterDown(String),
}

/// 命令读/写类型
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandType {
    Read,
    Write,
    Admin,
}

/// 从 Raft 状态机刷新路由缓存.
fn refresh_router_cache(mgr: &ClusterStateManager) {
    let meta = mgr.meta_raft.get_cluster_meta();
    let slot_table = mgr.meta_raft.get_slot_table();
    let mut group_nodes: HashMap<u64, Vec<u64>> = HashMap::new();
    let mut group_leaders: HashMap<u64, u64> = HashMap::new();
    let mut node_addrs: HashMap<u64, String> = HashMap::new();
    for (gid, group) in &meta.groups {
        group_nodes.insert(*gid, group.replicas.iter().map(|r| r.node_id).collect());
        if let Some(leader) = group.replicas.iter().find(|r| r.is_leader) {
            group_leaders.insert(*gid, leader.node_id);
        }
    }
    for (nid, node) in &meta.nodes {
        let addr = node
            .client_addr
            .clone()
            .unwrap_or_else(|| node.rpc_addr.clone());
        node_addrs.insert(*nid, addr);
    }
    mgr.router
        .refresh_from_data(slot_table, group_nodes, node_addrs, group_leaders);
}

/// 集群路由决策器
pub struct ClusterRouter;

impl ClusterRouter {
    /// 决定如何处理 key 的请求.
    /// 同步调用, 所有状态来自 ClusterStateManager 缓存.
    #[tracing::instrument(name = "kv_cluster_route", skip_all)]
    pub fn decide(
        key: &[u8],
        cmd_type: CommandType,
        asking: bool,
        readonly: bool,
    ) -> RouteDecision {
        let Some(mgr) = CLUSTER_STATE_MGR.get() else {
            return RouteDecision::ClusterDown("CLUSTERDOWN Cluster not initialized".into());
        };
        let slot = key_to_slot(key);
        let (group_id, status) = match mgr.router.route_key(key) {
            Ok(r) => r,
            Err(_) => {
                // Router cache may be stale (lifecycle tick hasn't caught up yet).
                // Refresh from Raft state machine and retry once.
                refresh_router_cache(mgr);
                match mgr.router.route_key(key) {
                    Ok(r) => r,
                    Err(e) => return RouteDecision::ClusterDown(e.to_string()),
                }
            }
        };

        // IMPORTING window: router cache may still show Assigned while the target
        // node tracks the slot locally via CLUSTER SETSLOT IMPORTING.
        if cmd_type == CommandType::Write && mgr.importing_slots.read().contains_key(&slot) {
            return RouteDecision::Execute;
        }

        match status {
            SlotStatus::Assigned(_) => {
                if should_execute_locally(mgr, group_id, &cmd_type, readonly) {
                    RouteDecision::Execute
                } else {
                    let has_group_meta = mgr
                        .meta_raft
                        .get_cluster_meta()
                        .groups
                        .contains_key(&group_id);
                    if has_group_meta {
                        // Leader 可能刚切换; 从 MetaRaft 刷新后再判一次.
                        mgr.refresh();
                        refresh_router_cache(mgr);
                        if should_execute_locally(mgr, group_id, &cmd_type, readonly) {
                            return RouteDecision::Execute;
                        }
                    }
                    leader_moved(mgr, group_id, slot)
                }
            }
            SlotStatus::Migrating(source_group) => {
                let mig_state = mgr.meta_raft.get_migration_state();
                let target_group = match &mig_state {
                    Some(aidb::cluster::SlotMigrationState::Prepare { target_group, .. })
                    | Some(aidb::cluster::SlotMigrationState::Migrating { target_group, .. }) => {
                        *target_group
                    }
                    None => {
                        return RouteDecision::ClusterDown(
                            "CLUSTERDOWN migration state not found".into(),
                        )
                    }
                };
                if mgr.multi_raft.is_group_local(source_group) {
                    leader_ask(mgr, target_group, slot)
                } else if mgr.multi_raft.is_group_local(target_group) {
                    // IMPORTING node: accept one-shot writes (e.g. MIGRATE RESTORE) when ASKING
                    // or local importing_slots tracking from CLUSTER SETSLOT IMPORTING.
                    let is_importing = mgr.importing_slots.read().contains_key(&slot);
                    if asking || is_importing {
                        RouteDecision::Execute
                    } else {
                        leader_moved(mgr, source_group, slot)
                    }
                } else {
                    RouteDecision::ClusterDown("CLUSTERDOWN slot migration in progress".into())
                }
            }
            SlotStatus::Unallocated => {
                // Last-resort retry: refresh and check if slot was just assigned.
                refresh_router_cache(mgr);
                match mgr.router.route_key(key) {
                    Ok((group_id, SlotStatus::Assigned(_))) => {
                        if mgr.multi_raft.is_group_local(group_id)
                            && mgr.is_local_group_leader(group_id)
                        {
                            RouteDecision::Execute
                        } else {
                            leader_moved(mgr, group_id, slot)
                        }
                    }
                    _ => RouteDecision::ClusterDown("CLUSTERDOWN The cluster is down".into()),
                }
            }
        }
    }
}

fn should_execute_locally(
    mgr: &ClusterStateManager,
    group_id: u64,
    cmd_type: &CommandType,
    readonly: bool,
) -> bool {
    mgr.multi_raft.is_group_local(group_id)
        && (mgr.is_local_group_leader(group_id) || (*cmd_type == CommandType::Read && readonly))
}

fn leader_moved(mgr: &ClusterStateManager, group_id: u64, slot: u16) -> RouteDecision {
    match mgr.router.get_group_leader(group_id) {
        Some(leader) => match mgr.announce_resolver.redirect_addr(leader, &mgr.router) {
            Some(addr) => RouteDecision::Moved {
                slot,
                node_id: leader,
                addr,
            },
            None => RouteDecision::ClusterDown("CLUSTERDOWN unknown node address".into()),
        },
        None => RouteDecision::ClusterDown("CLUSTERDOWN no leader for group".into()),
    }
}

fn leader_ask(mgr: &ClusterStateManager, group_id: u64, slot: u16) -> RouteDecision {
    match mgr.router.get_group_leader(group_id) {
        Some(leader) => match mgr.announce_resolver.redirect_addr(leader, &mgr.router) {
            Some(addr) => RouteDecision::Ask {
                slot,
                node_id: leader,
                addr,
            },
            None => RouteDecision::ClusterDown("CLUSTERDOWN unknown target address".into()),
        },
        None => RouteDecision::ClusterDown("CLUSTERDOWN no target leader".into()),
    }
}

/// 检查多个 key 是否在同一 slot
#[tracing::instrument(name = "kv_cluster_cross_slot", skip_all)]
pub fn check_cross_slot(keys: &[&[u8]]) -> Result<(), String> {
    if keys.len() <= 1 {
        return Ok(());
    }
    let first_slot = key_to_slot(keys[0]);
    if keys[1..].iter().any(|k| key_to_slot(k) != first_slot) {
        return Err("CROSSSLOT Keys in request don't hash to the same slot".into());
    }
    Ok(())
}
