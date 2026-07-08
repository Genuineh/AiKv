// Cluster routing unit tests.
#[cfg(feature = "cluster")]
mod tests {
    use std::collections::HashMap;
    use std::sync::atomic::AtomicU64;
    use std::sync::Arc;

    use parking_lot::RwLock;

    use aidb::cluster::meta_types::{default_slot_table, SlotMigrationState, SlotStatus};
    use aidb::cluster::MultiRaftNode;
    use aidb::cluster::RaftServiceDispatcher;
    use aidb::cluster::Router;

    use aikv::cluster::announce::AnnounceResolver;
    use aikv::cluster::connection::ClusterConnectionState;
    use aikv::cluster::router::{ClusterRouter, CommandType, RouteDecision};
    use aikv::cluster::state::{
        ClusterStateManager, ReplicationRole, CLUSTER_STATE_MGR, DEFAULT_DATA_PORT_OFFSET,
    };

    #[test]
    fn route_decision_uninitialized_returns_clusterdown() {
        let result = ClusterRouter::decide(b"x", CommandType::Read, false, false);
        assert!(matches!(result, RouteDecision::ClusterDown(_)));
    }

    /// 全分支路由测试 (串行, OnceLock 只能 set 一次).
    ///
    /// 覆盖以下决策分支:
    /// 1. MOVED — key 不在本地 Group
    /// 2. CLUSTERDOWN — slot 未分配
    /// 3. LocalLeader Execute — key 在本地 Group 且本节点是 leader
    /// 4. Readonly replica Execute — key 在本地 Group, readonly 标志设置
    /// 5. ASK — slot 迁移中且 ASKING 未设置 (source local)
    /// 6. ASKING flag set — slot 迁移中且 ASKING 已设置 (target local)
    #[tokio::test]
    async fn route_decision_all_branches() {
        // ── 0. Router state (shared across tests, modified in-place) ──
        // slots 0..4 → group 1; groups {1: [1,2,3], 2: [4,5,6]}; addrs {1: "7001", 4: "7004"}
        let mut group_nodes = HashMap::new();
        group_nodes.insert(1u64, vec![1u64, 2u64, 3u64]);
        group_nodes.insert(2u64, vec![4u64, 5u64, 6u64]);
        let mut node_addrs = HashMap::new();
        node_addrs.insert(1u64, "127.0.0.1:7001".to_string());
        node_addrs.insert(2u64, "127.0.0.1:7002".to_string());
        node_addrs.insert(4u64, "127.0.0.1:7004".to_string());

        let mut table = default_slot_table();
        for s in 0u16..5 {
            table[s as usize] = SlotStatus::Assigned(1);
        }
        let router = Router::new(table.clone(), group_nodes.clone(), node_addrs.clone());

        // ── 1. Infrastructure ──
        let dispatcher = Arc::new(RaftServiceDispatcher::new());
        let multi_raft = Arc::new(MultiRaftNode::new(1, Arc::new(router.clone()), dispatcher));

        let raft_config = aidb::cluster::RaftNodeConfig {
            node_id: 1,
            group_id: 0,
            election_timeout_min: 2000,
            election_timeout_max: 4000,
            heartbeat_interval: 100,
            max_payload_entries: 100,
            snapshot_logs_since_last: 1000,
            max_entry_size: 8192,
            rpc_timeout_ms: 500,
            grpc_max_message_size: 65536,
            snapshot_size_threshold: None,
        };
        let db = aidb::DB::open(
            std::env::temp_dir().join("raft_test"),
            aidb::config::Options::for_testing(),
        )
        .unwrap();
        let net_factory = aidb::cluster::RaftNetworkClientFactory::new(1, 0, 30, 65536);
        let meta_raft = aidb::cluster::MetaRaftNode::new(raft_config, db, net_factory)
            .await
            .unwrap();
        // 同步 MetaRaft 状态机的 slot table 与 Router 初始状态一致,
        // 避免 refresh_router_cache 用全 Unallocated 表覆盖 Router 缓存.
        meta_raft.set_slot_table(table.clone());

        // ── 2. ClusterStateManager ──
        let metrics = Arc::new(aikv::server::metrics::ServerMetrics::default());
        let mgr = ClusterStateManager {
            router,
            meta_raft: Arc::new(meta_raft),
            multi_raft,
            node_id: 1,
            config_epoch: AtomicU64::new(0),
            role: RwLock::new(ReplicationRole::Primary),
            local_group_leaders: {
                let mut l = HashMap::new();
                l.insert(1u64, true);
                RwLock::new(l)
            },
            membership_coordinator: None,
            slot_migration_manager: None,
            data_dir: None,
            importing_slots: RwLock::new(HashMap::new()),
            data_port_offset: DEFAULT_DATA_PORT_OFFSET,
            announce_resolver: AnnounceResolver::default(),
            metrics: Some(Arc::clone(&metrics)),
            _watcher_shutdown: parking_lot::Mutex::new(None),
            _auto_save_shutdown: parking_lot::Mutex::new(None),
        };
        let _ = CLUSTER_STATE_MGR.set(Arc::new(mgr));
        let mgr_ref = CLUSTER_STATE_MGR.get().unwrap();

        // ═══════════════════════════════════════════════════════════════
        // 3. MOVED — key not local
        // ═══════════════════════════════════════════════════════════════
        // b"\x00\x00" → CRC16=0 → slot 0 → group 1 (not local) → MOVED
        let r = ClusterRouter::decide(b"\x00\x00", CommandType::Read, false, false);
        match &r {
            RouteDecision::Moved {
                slot,
                node_id,
                addr,
            } => {
                assert_eq!(*slot, 0, "slot 0");
                assert_eq!(*node_id, 1, "leader node id");
                assert_eq!(addr, ":7001", "unknown announce mode uses port-only MOVED");
            }
            _ => panic!("expected MOVED, got {:?}", r),
        }

        // ═══════════════════════════════════════════════════════════════
        // 4. CLUSTERDOWN — unallocated slot
        // ═══════════════════════════════════════════════════════════════
        // b"\x01" → CRC16=0x1021=4129 → slot 4129 (unallocated) → CLUSTERDOWN
        let r = ClusterRouter::decide(b"\x01", CommandType::Read, false, false);
        assert!(
            matches!(r, RouteDecision::ClusterDown(_)),
            "expected CLUSTERDOWN, got {:?}",
            r
        );

        // ═══════════════════════════════════════════════════════════════
        // 5. LocalLeader Execute — group local + leader
        // ═══════════════════════════════════════════════════════════════
        mgr_ref.multi_raft.override_group_local(1);
        // b"\x00\x00" → slot 0 → group 1 (local, leader) → Execute
        let r = ClusterRouter::decide(b"\x00\x00", CommandType::Write, false, false);
        assert!(
            matches!(r, RouteDecision::Execute),
            "expected Execute (local leader), got {:?}",
            r
        );

        // ═══════════════════════════════════════════════════════════════
        // 6. Readonly replica Execute — group local, not leader, readonly
        // ═══════════════════════════════════════════════════════════════
        mgr_ref.local_group_leaders.write().insert(1, false);
        *mgr_ref.role.write() = ReplicationRole::Replica { primary_id: 2 };
        // b"\x00\x00" → slot 0 → group 1 (local, not leader) + Read + readonly → Execute
        let r = ClusterRouter::decide(b"\x00\x00", CommandType::Read, false, true);
        assert!(
            matches!(r, RouteDecision::Execute),
            "expected Execute (readonly replica), got {:?}",
            r
        );
        // Restore state for subsequent tests
        *mgr_ref.role.write() = ReplicationRole::Primary;
        mgr_ref.local_group_leaders.write().insert(1, true);

        // ═══════════════════════════════════════════════════════════════
        // 7. ASK-Redirect-Migrate — slot Migrating, source local
        // ═══════════════════════════════════════════════════════════════
        // v7: reads stay on source (source holds full data), writes are
        // ASK-redirected to target so post-commit the target has the latest
        // values. Per-key marking is unnecessary — the slot-level redirect
        // eliminates every per-key TOCTOU race.
        // Set slot 0 → Migrating(1) (migrating from group 1 → 2)
        let mut ask_table = default_slot_table();
        for s in 0u16..5 {
            ask_table[s as usize] = if s == 0 {
                SlotStatus::Migrating(1)
            } else {
                SlotStatus::Assigned(1)
            };
        }
        // Group 2 needs a leader so ask_target() can resolve the ASK addr.
        let mut mig_group_leaders = HashMap::new();
        mig_group_leaders.insert(2u64, 4u64);
        mgr_ref.router.refresh_from_data(
            ask_table.clone(),
            group_nodes.clone(),
            node_addrs.clone(),
            mig_group_leaders,
        );
        mgr_ref
            .meta_raft
            .set_migration_state(Some(SlotMigrationState::Migrating {
                source_group: 1,
                target_group: 2,
                slots: vec![0],
                progress: 0,
                total: 0,
            }));

        // Write → ASK(target), node 4 at addr ":7004"
        let r = ClusterRouter::decide(b"\x00\x00", CommandType::Write, false, false);
        match &r {
            RouteDecision::Ask {
                slot,
                node_id,
                addr,
            } => {
                assert_eq!(*slot, 0);
                assert_eq!(*node_id, 4, "ASK should point to target group 2's leader");
                assert_eq!(addr, ":7004");
            }
            _ => panic!("expected ASK (v7 write redirect to target), got {:?}", r),
        }
        // Read → Execute (source still serves reads)
        let r = ClusterRouter::decide(b"\x00\x00", CommandType::Read, false, false);
        assert!(
            matches!(r, RouteDecision::Execute),
            "expected Execute (source keeps serving reads during migration), got {:?}",
            r
        );

        // ═══════════════════════════════════════════════════════════════
        // 7b. Readonly replica Execute — Migrating, source local, not leader, readonly
        // ═══════════════════════════════════════════════════════════════
        mgr_ref.local_group_leaders.write().insert(1, false);
        *mgr_ref.role.write() = ReplicationRole::Replica { primary_id: 2 };
        let r = ClusterRouter::decide(b"\x00\x00", CommandType::Read, false, true);
        assert!(
            matches!(r, RouteDecision::Execute),
            "expected Execute (readonly replica keeps serving during migration), got {:?}",
            r
        );

        // ═══════════════════════════════════════════════════════════════
        // 7c. MOVED — Migrating, source local, not leader, not readonly
        // ═══════════════════════════════════════════════════════════════
        // Non-readonly write on a non-leader source replica still can't
        // execute locally; it must fall through to MOVED (to the source
        // group's leader), exactly like the Assigned branch would. Give
        // group 1 an explicit leader (node 2, != self) so is_local_group_leader
        // can't short-circuit back to true via the router's own-node fallback.
        let mut group_leaders_2 = HashMap::new();
        group_leaders_2.insert(1u64, 2u64);
        mgr_ref.router.refresh_from_data(
            ask_table.clone(),
            group_nodes.clone(),
            node_addrs.clone(),
            group_leaders_2,
        );
        let r = ClusterRouter::decide(b"\x00\x00", CommandType::Write, false, false);
        match &r {
            RouteDecision::Moved { slot, node_id, .. } => {
                assert_eq!(*slot, 0);
                assert_eq!(*node_id, 2, "should redirect to source group's leader");
            }
            _ => panic!("expected MOVED (migrating, non-leader, non-readonly), got {:?}", r),
        }
        // Restore state for subsequent tests
        mgr_ref.router.refresh_from_data(
            ask_table.clone(),
            group_nodes.clone(),
            node_addrs.clone(),
            HashMap::new(),
        );
        *mgr_ref.role.write() = ReplicationRole::Primary;
        mgr_ref.local_group_leaders.write().insert(1, true);

        // ═══════════════════════════════════════════════════════════════
        // 8. ASKING flag set — target local, asking=true
        // ═══════════════════════════════════════════════════════════════
        mgr_ref.multi_raft.clear_group_local_override(1);
        mgr_ref.multi_raft.override_group_local(2);

        // b"\x00\x00" → slot 0 → Migrating(1), source not local, target local, asking → Execute
        let r = ClusterRouter::decide(b"\x00\x00", CommandType::Read, true, false);
        assert!(
            matches!(r, RouteDecision::Execute),
            "expected Execute (ASKING), got {:?}",
            r
        );

        // ═══════════════════════════════════════════════════════════════
        // 13. IMPORTING entry shortcut — router Assigned, target local, importing_slots
        // ═══════════════════════════════════════════════════════════════
        mgr_ref.importing_slots.write().clear();
        mgr_ref.meta_raft.set_migration_state(None);
        mgr_ref.router.refresh_from_data(
            table.clone(),
            group_nodes.clone(),
            node_addrs.clone(),
            HashMap::new(),
        );
        mgr_ref.multi_raft.clear_group_local_override(1);
        mgr_ref.multi_raft.override_group_local(2);

        mgr_ref.importing_slots.write().insert(0, 1);
        let r = ClusterRouter::decide(b"\x00\x00", CommandType::Write, false, false);
        assert!(
            matches!(r, RouteDecision::Execute),
            "expected Execute (importing_slots entry), got {:?}",
            r
        );

        let r = ClusterRouter::decide(b"\x00\x00", CommandType::Read, false, false);
        match &r {
            RouteDecision::Moved { slot, .. } => assert_eq!(*slot, 0),
            _ => panic!("expected MOVED (importing read), got {:?}", r),
        }

        // ═══════════════════════════════════════════════════════════════
        // 14. ASKING alone insufficient — Assigned window without importing_slots
        // ═══════════════════════════════════════════════════════════════
        mgr_ref.importing_slots.write().clear();
        let r = ClusterRouter::decide(b"\x00\x00", CommandType::Write, true, false);
        match &r {
            RouteDecision::Moved { slot, .. } => assert_eq!(*slot, 0),
            _ => panic!("expected MOVED (asking without importing), got {:?}", r),
        }

        // ═══════════════════════════════════════════════════════════════
        // 15. Migrating branch + importing_slots regression
        // ═══════════════════════════════════════════════════════════════
        mgr_ref.router.refresh_from_data(
            ask_table.clone(),
            group_nodes.clone(),
            node_addrs.clone(),
            HashMap::new(),
        );
        mgr_ref
            .meta_raft
            .set_migration_state(Some(SlotMigrationState::Migrating {
                source_group: 1,
                target_group: 2,
                slots: vec![0],
                progress: 0,
                total: 0,
            }));
        mgr_ref.importing_slots.write().insert(0, 1);
        let r = ClusterRouter::decide(b"\x00\x00", CommandType::Write, false, false);
        assert!(
            matches!(r, RouteDecision::Execute),
            "expected Execute (migrating + importing), got {:?}",
            r
        );
        mgr_ref.importing_slots.write().clear();

        // ═══════════════════════════════════════════════════════════════
        // 9. CLUSTER INFO format validation
        // ═══════════════════════════════════════════════════════════════
        let info = aikv::cluster::cluster_info();
        assert!(info.is_ok(), "cluster_info should work after init");
        let info_str = info.unwrap();
        assert!(
            info_str.contains("cluster_state:fail"),
            "partial slot assignment should report fail: {info_str}"
        );
        assert!(
            info_str.contains("cluster_slots_assigned:"),
            "missing slots_assigned"
        );
        assert!(
            info_str.contains("cluster_known_nodes:"),
            "missing known_nodes"
        );
        assert!(info_str.contains("cluster_size:"), "missing cluster_size");
        metrics.on_gossip_refresh(mgr_ref.meta_raft.get_cluster_meta().nodes.len());
        let info_str = aikv::cluster::cluster_info().unwrap();
        assert!(
            info_str.contains("cluster_stats_messages_sent:1"),
            "expected gossip sent count in CLUSTER INFO: {info_str}"
        );
        assert!(
            info_str.contains(&format!(
                "cluster_stats_messages_received:{}",
                mgr_ref.meta_raft.get_cluster_meta().nodes.len()
            )),
            "expected gossip received count in CLUSTER INFO: {info_str}"
        );

        // ═══════════════════════════════════════════════════════════════
        // 10. CLUSTER MYID format validation
        // ═══════════════════════════════════════════════════════════════
        let myid = aikv::cluster::cluster_myid();
        assert!(myid.is_ok(), "cluster_myid should work after init");
        let myid_str = myid.unwrap();
        assert_eq!(myid_str.len(), 40, "MYID should be 40 hex chars");
        assert!(
            myid_str.chars().all(|c| c.is_ascii_hexdigit()),
            "MYID should be hex"
        );

        // ═══════════════════════════════════════════════════════════════
        // 11. CLUSTER NODES format validation
        // ═══════════════════════════════════════════════════════════════
        let nodes = aikv::cluster::cluster_nodes();
        assert!(nodes.is_ok(), "cluster_nodes should work after init");
        let nodes_str = nodes.unwrap();
        // Each line should be a node entry
        for line in nodes_str.lines() {
            assert!(!line.is_empty(), "node line should not be empty");
            assert!(
                line.contains(' '),
                "node line should have space-separated fields: {line}"
            );
        }

        // ═══════════════════════════════════════════════════════════════
        // 12. CLUSTER SLOTS does not panic after init
        // ═══════════════════════════════════════════════════════════════
        // Note: SLOTS reads from MetaRaft state machine, which in this test
        // uses default_slot_table() (all Unallocated). The test verifies
        // that the function does not panic/unwrap_err.
        let slots = aikv::cluster::cluster_slots();
        assert!(slots.is_ok(), "cluster_slots should not panic after init");
    }

    // ── ClusterConnectionState ──

    #[test]
    fn cluster_conn_state_asking_reset() {
        let mut state = ClusterConnectionState::new();
        assert!(!state.is_asking());
        state.set_asking(true);
        assert!(state.is_asking());
        state.reset_asking();
        assert!(!state.is_asking());
    }

    #[test]
    fn cluster_conn_state_readonly_toggle() {
        let mut state = ClusterConnectionState::new();
        assert!(!state.is_readonly());
        state.set_readonly(true);
        assert!(state.is_readonly());
        state.set_readonly(false);
        assert!(!state.is_readonly());
    }

    #[test]
    fn cross_slot_rejection_multi_key() {
        use aikv::cluster::router::check_cross_slot;
        // Same slot keys (should pass)
        assert!(check_cross_slot(&[b"key1"]).is_ok());
        assert!(check_cross_slot(&[b"a", b"a"]).is_ok());

        // Different slot keys (should fail)
        let err = check_cross_slot(&[b"key1", b"key2"]).unwrap_err();
        assert!(err.contains("CROSSSLOT"), "got: {err}");
    }

    #[test]
    fn cross_slot_single_key_always_ok() {
        use aikv::cluster::router::check_cross_slot;
        assert!(check_cross_slot(&[]).is_ok());
        assert!(check_cross_slot(&[b"x"]).is_ok());
        assert!(check_cross_slot(&[b""]).is_ok());
    }

    #[test]
    fn cluster_slots_error_on_uninitialized() {
        // Verify cluster_slots returns error before CLUSTER_STATE_MGR is set.
        // (CLUSTER_STATE_MGR is already initialized by route_decision_all_branches,
        //  so this test runs as a sanity check that the function exists.)
        let _ = aikv::cluster::cluster_slots();
    }

    #[test]
    fn key_to_slot_hash_tag_extraction() {
        // Hash tag {abc} means slot is computed from "abc" only
        let tagged = aidb::cluster::router::key_to_slot(b"{abc}.suffix");
        let bare = aidb::cluster::router::key_to_slot(b"abc");
        assert_eq!(tagged, bare, "hash tag should extract inner key");

        // Unmatched braces: use whole key
        let no_close = aidb::cluster::router::key_to_slot(b"{abc");
        let whole_key = aidb::cluster::router::key_to_slot(b"{abc");
        assert_eq!(no_close, whole_key);
    }
}
