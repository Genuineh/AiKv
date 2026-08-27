use super::cluster_node_role_label;
use super::dispatch_cluster;
use super::topology::{apply_leader_quorum_gate, cluster_nodes_link_state, compute_cluster_state};
use crate::cluster::state::derive_cluster_ok;

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
    use super::{apply_leader_quorum_gate, compute_cluster_state};
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

    /// `apply_leader_quorum_gate`: CLUSTER INFO 报 fail 的直接驱动.
    /// 本节点作为 leader 且失去 quorum → None; 其他情况透传.
    #[test]
    fn quorum_gate_only_blocks_self_leader_lost_quorum() {
        // 本节点是 leader + 失去 quorum → 视为无 leader (fail)
        assert_eq!(apply_leader_quorum_gate(Some(7), 7, false), None);
        // 本节点是 leader + 保有 quorum → 透传
        assert_eq!(apply_leader_quorum_gate(Some(7), 7, true), Some(7));
        // 本节点非 leader (其他节点) → 不拦截 (follower 不判定)
        assert_eq!(apply_leader_quorum_gate(Some(8), 7, false), Some(8));
        // 无 leader → 透传 None (compute_cluster_state 自会判 fail)
        assert_eq!(apply_leader_quorum_gate(None, 7, false), None);
    }
}

#[cfg(test)]
mod derive_cluster_ok_tests {
    use super::derive_cluster_ok;
    use aidb::cluster::meta_types::{
        default_slot_table, ClusterMeta, GroupMeta, ReplicaInfo, SlotStatus,
    };
    use std::collections::HashMap;

    fn meta_with_group(group_id: u64) -> ClusterMeta {
        ClusterMeta {
            groups: HashMap::from([(
                group_id,
                GroupMeta {
                    group_id,
                    replicas: vec![ReplicaInfo {
                        node_id: group_id,
                        is_leader: true,
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
    fn quorum_failed_group_owns_slots_is_down() {
        let table = full_slot_table(1);
        let meta = meta_with_group(1);
        let quorum = HashMap::from([(1u64, false)]);
        assert!(!derive_cluster_ok(&quorum, &table, &meta));
    }

    #[test]
    fn quorum_ok_or_empty_is_healthy() {
        let table = full_slot_table(1);
        let meta = meta_with_group(1);
        let quorum_ok = HashMap::from([(1u64, true)]);
        assert!(derive_cluster_ok(&quorum_ok, &table, &meta));
        let quorum_empty = HashMap::new();
        assert!(derive_cluster_ok(&quorum_empty, &table, &meta));
    }

    #[test]
    fn no_slot_owning_group_is_healthy() {
        let table = default_slot_table();
        let meta = meta_with_group(1);
        let quorum = HashMap::from([(1u64, false)]);
        assert!(derive_cluster_ok(&quorum, &table, &meta));
    }

    #[test]
    fn orphan_slot_owning_group_is_down() {
        // slot 指向的 group 不在 meta.groups (孤儿) → 视为不健康,
        // 与 compute_cluster_state 的 slots_map_to_known_groups 判定对齐
        // (避免 router 门开放而 CLUSTER INFO 报 fail 的分裂).
        let table = full_slot_table(1);
        let meta = meta_with_group(2); // 仅 group 2 存在, group 1 是孤儿
        let quorum = HashMap::new();
        assert!(!derive_cluster_ok(&quorum, &table, &meta));
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
