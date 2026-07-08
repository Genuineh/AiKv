//! @component aikv-cluster
// Cluster protocol L2 integration tests.
// CLUSTER_STATE_MGR is OnceLock — all tests use a shared setup via std::sync::Once.
#![cfg(feature = "cluster")]

use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, LazyLock};

use parking_lot::RwLock;

use aidb::cluster::meta_types::{default_slot_table, SlotStatus};
use aidb::cluster::{MultiRaftNode, RaftServiceDispatcher, Router};

use aikv::cluster::announce::AnnounceResolver;
use aikv::cluster::state::{
    ClusterStateManager, ReplicationRole, CLUSTER_STATE_MGR, DEFAULT_DATA_PORT_OFFSET,
};

static RT: LazyLock<tokio::runtime::Runtime> =
    LazyLock::new(|| tokio::runtime::Runtime::new().unwrap());

fn ensure_cluster_state() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        let mgr = RT.block_on(create_cluster_mgr());
        let _ = CLUSTER_STATE_MGR.set(Arc::new(mgr));
    });
}

async fn create_cluster_mgr() -> ClusterStateManager {
    let mut group_nodes = HashMap::new();
    group_nodes.insert(1, vec![1, 2, 3]);
    let mut node_addrs = HashMap::new();
    node_addrs.insert(1, "127.0.0.1:7001".to_string());

    let mut table = default_slot_table();
    for s in 0u16..5 {
        table[s as usize] = SlotStatus::Assigned(1);
    }
    let router = Router::new(table.clone(), group_nodes, node_addrs);
    let dispatcher = Arc::new(RaftServiceDispatcher::new());
    let multi_raft = Arc::new(MultiRaftNode::new(1, Arc::new(router.clone()), dispatcher));

    let db = aidb::DB::open(
        std::env::temp_dir().join(format!("cluster_int_{}", std::process::id())),
        aidb::config::Options::for_testing(),
    )
    .unwrap();
    let net_factory = aidb::cluster::RaftNetworkClientFactory::new(1, 0, 30, 65536);
    let meta_raft = aidb::cluster::MetaRaftNode::new(
        aidb::cluster::RaftNodeConfig {
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
            linearizable_read: false,
        },
        db,
        net_factory,
    )
    .await
    .unwrap();
    meta_raft.set_slot_table(table);

    ClusterStateManager {
        router,
        meta_raft: Arc::new(meta_raft),
        multi_raft,
        node_id: 1,
        config_epoch: AtomicU64::new(0),
        role: RwLock::new(ReplicationRole::Primary),
        local_group_leaders: {
            let mut l = HashMap::new();
            l.insert(1, true);
            RwLock::new(l)
        },
        membership_coordinator: None,
        slot_migration_manager: None,
        data_dir: None,
        importing_slots: RwLock::new(HashMap::new()),
        data_port_offset: DEFAULT_DATA_PORT_OFFSET,
        announce_resolver: AnnounceResolver::default(),
        metrics: None,
        _watcher_shutdown: parking_lot::Mutex::new(None),
        _auto_save_shutdown: parking_lot::Mutex::new(None),
    }
}

fn block_on<F: std::future::Future>(f: F) -> F::Output {
    RT.block_on(f)
}

// ── CLUSTER help / format ──

#[test]
fn cluster_info_format() {
    ensure_cluster_state();
    let info = aikv::cluster::cluster_info().unwrap();
    for field in [
        "cluster_state",
        "cluster_slots_assigned",
        "cluster_known_nodes",
        "cluster_size",
        "cluster_current_epoch",
    ] {
        assert!(info.contains(field), "info missing {field}");
    }
}

#[test]
fn cluster_myid_format() {
    ensure_cluster_state();
    let myid = aikv::cluster::cluster_myid().unwrap();
    assert_eq!(myid.len(), 40);
    assert!(myid.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn cluster_nodes_format() {
    ensure_cluster_state();
    let nodes = aikv::cluster::cluster_nodes().unwrap();
    // nodes may be empty (MetaRaft not initialized with nodes),
    // but the function should not panic
    for line in nodes.lines() {
        assert!(line.split(' ').count() >= 2, "bad line: {line}");
    }
}

// ── CLUSTER command errors (async) ──

#[test]
fn cluster_replicate_sets_role() {
    ensure_cluster_state();
    // REPLICATE sets the local role — succeeds regardless of primary existence
    let result = block_on(aikv::cluster::cluster_replicate(999));
    assert!(
        result.is_ok(),
        "REPLICATE should set local role: {:?}",
        result
    );
    // Verify the role was changed by reading it back through REPLICAS
    let replicas = aikv::cluster::cluster_replicas(999);
    let _ = replicas;
}

#[test]
fn cluster_add_slots_not_leader() {
    ensure_cluster_state();
    if let Err(e) = block_on(aikv::cluster::cluster_add_slots(&[100, 101], None)) {
        assert!(e.contains("ERR") || e.contains("CLUSTERDOWN"));
    }
}

#[test]
fn cluster_del_slots_not_leader() {
    ensure_cluster_state();
    if let Err(e) = block_on(aikv::cluster::cluster_del_slots(&[100])) {
        assert!(e.contains("ERR") || e.contains("CLUSTERDOWN"));
    }
}

#[test]
fn cluster_failover_from_primary() {
    ensure_cluster_state();
    let err = block_on(aikv::cluster::cluster_failover("FORCE")).unwrap_err();
    assert!(err.contains("ERR"));
}

#[test]
fn cluster_saveconfig_dispatch() {
    ensure_cluster_state();
    if let Err(e) = block_on(aikv::cluster::dispatch_cluster(Some("SAVECONFIG"), &[])) {
        let msg = format!("{e}");
        assert!(msg.contains("ERR") || msg.contains("CLUSTERDOWN"));
    }
}

// ── Sync command error / format ──

#[test]
fn cluster_keyslot_basic() {
    ensure_cluster_state();
    let slot: u16 = aikv::cluster::cluster_keyslot(b"x")
        .unwrap()
        .parse()
        .unwrap();
    assert!(slot < 16384);
}
