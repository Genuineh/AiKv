//! @component aikv-cluster
//! GroupSetBatcher: Plain SET/DEL 同批合并与最后一次胜出.

#![cfg(feature = "cluster")]

use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::RwLock;

use aidb::cluster::multi_raft_node::LifecycleConfig;
use aidb::cluster::router::key_to_slot;
use aidb::cluster::{
    MetaRaftNode, MetaRequest, MultiRaftNode, RaftNetworkClientFactory, RaftNodeConfig,
    RaftServiceDispatcher, Router, SlotStatus, ThinWriteOp,
};
use aidb::config::Options;
use aidb::DB;
use tempfile::TempDir;

use aikv::cluster::announce::AnnounceResolver;
use aikv::cluster::state::{
    ClusterStateManager, ReplicationRole, CLUSTER_STATE_MGR, DEFAULT_DATA_PORT_OFFSET,
};
use aikv::storage::adapter::StorageAdapter;
use aikv::storage::aidb::AiDbEngine;
use aikv::storage::cluster_adapter::{
    coalesce_batch_ops, BatchWriteOp, ClusterDataAdapter,
};

/// 从 `ThinWriteBatch` 取某 key 的最终 op (同 key 应只出现一次).
fn final_op_for_key<'a>(
    ops: &'a [ThinWriteOp],
    key: &[u8],
) -> Option<&'a ThinWriteOp> {
    let matches: Vec<_> = ops
        .iter()
        .filter(|op| match op {
            ThinWriteOp::Put { key: k, .. } | ThinWriteOp::Delete { key: k } => k.as_slice() == key,
        })
        .collect();
    assert!(
        matches.len() <= 1,
        "coalesce must keep at most one op per key, got {}",
        matches.len()
    );
    matches.into_iter().next()
}

/// 同 key 先 SET 后 DEL (同批窗口内) 最终应为 Delete.
#[test]
fn test_batched_set_then_delete_same_key_last_wins() {
    let key = b"k1".to_vec();
    let tb = coalesce_batch_ops(&[
        (key.clone(), BatchWriteOp::Put { value: b"v1".to_vec() }),
        (key.clone(), BatchWriteOp::Delete),
    ]);
    match final_op_for_key(&tb.ops, &key) {
        Some(ThinWriteOp::Delete { .. }) => {}
        other => panic!("expected Delete last-wins, got {other:?}"),
    }
}

/// 同 key 先 DEL 后 SET, 最终应为 Put(最终 value).
#[test]
fn test_batched_delete_then_set_same_key_last_wins() {
    let key = b"k2".to_vec();
    let tb = coalesce_batch_ops(&[
        (key.clone(), BatchWriteOp::Delete),
        (key.clone(), BatchWriteOp::Put { value: b"final".to_vec() }),
    ]);
    match final_op_for_key(&tb.ops, &key) {
        Some(ThinWriteOp::Put { value, .. }) => {
            assert_eq!(value.as_slice(), b"final");
        }
        other => panic!("expected Put last-wins, got {other:?}"),
    }
}

/// 搭建单节点、单真实 data group 的最小集群 (Plain 写路径 smoke).
async fn build_plain_mgr() -> Arc<ClusterStateManager> {
    let key = b"plain_del{b}";
    let slot = key_to_slot(key);

    let meta_dir = TempDir::new().unwrap();
    let meta_db = DB::open(meta_dir.path().join("meta"), Options::for_testing()).unwrap();
    let meta_factory =
        RaftNetworkClientFactory::new(1, aidb::cluster::METARAFT_GROUP_ID, 30, 65 * 1024 * 1024);
    let meta_cfg = RaftNodeConfig {
        node_id: 1,
        group_id: aidb::cluster::METARAFT_GROUP_ID,
        election_timeout_min: 150,
        election_timeout_max: 300,
        heartbeat_interval: 30,
        rpc_timeout_ms: 30,
        snapshot_logs_since_last: 200,
        ..Default::default()
    };
    let meta_raft = Arc::new(MetaRaftNode::new(meta_cfg, meta_db, meta_factory).await.unwrap());
    meta_raft
        .initialize(vec![(1, "http://127.0.0.1:1".into())])
        .await
        .unwrap();
    for _ in 0..50 {
        if meta_raft.is_leader().await {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(meta_raft.is_leader().await, "meta raft should elect itself leader");

    meta_raft
        .propose(MetaRequest::RegisterNode {
            node_id: 1,
            rpc_addr: "http://127.0.0.1:19200".into(),
            client_addr: None,
            tags: HashMap::new(),
        })
        .await
        .unwrap();
    meta_raft
        .propose(MetaRequest::CreateGroup {
            group_id: 1,
            initial_replicas: vec![(1, true)],
        })
        .await
        .unwrap();
    meta_raft
        .propose(MetaRequest::AssignSlots {
            group_id: 1,
            slots: vec![slot],
        })
        .await
        .unwrap();

    let mut group_nodes = HashMap::new();
    group_nodes.insert(1u64, vec![1u64]);
    let mut node_addrs = HashMap::new();
    node_addrs.insert(1u64, "127.0.0.1:17011".to_string());
    let mut table = aidb::cluster::default_slot_table();
    table[slot as usize] = SlotStatus::Assigned(1);
    let router = Arc::new(Router::new(table, group_nodes, node_addrs));
    let dispatcher = Arc::new(RaftServiceDispatcher::new());
    let lifecycle = aidb::cluster::LifecycleManager::new(
        1,
        router.clone(),
        Arc::new(MetaRaftProv(meta_raft.clone())),
    )
    .with_tick_interval(Duration::from_millis(30));
    let multi_raft = Arc::new(MultiRaftNode::new_with_lifecycle(
        1,
        router.clone(),
        dispatcher,
        lifecycle,
    ));

    let data_dir = TempDir::new().unwrap();
    let _shutdown_rx = multi_raft.start_lifecycle_with_data(LifecycleConfig {
        data_dir: data_dir.path().to_path_buf(),
        raft_node_config: RaftNodeConfig {
            node_id: 1,
            election_timeout_min: 150,
            election_timeout_max: 300,
            heartbeat_interval: 30,
            rpc_timeout_ms: 30,
            snapshot_logs_since_last: 200,
            ..Default::default()
        },
        options: Options::for_testing(),
        compaction_filter: None,
    });

    for _ in 0..100 {
        let ids = multi_raft.local_group_ids();
        if !ids.is_empty() && multi_raft.is_elected_leader_sync(1) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        multi_raft.local_group_ids().contains(&1),
        "group 1 must be created locally"
    );
    assert!(
        multi_raft.is_elected_leader_sync(1),
        "group 1 must elect itself leader"
    );

    let mgr = ClusterStateManager {
        router: (*router).clone(),
        meta_raft,
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
        metrics: None,
        _watcher_shutdown: parking_lot::Mutex::new(None),
        _auto_save_shutdown: parking_lot::Mutex::new(None),
    };
    let _ = CLUSTER_STATE_MGR.set(Arc::new(mgr));
    std::mem::forget((meta_dir, data_dir));
    CLUSTER_STATE_MGR.get().unwrap().clone()
}

struct MetaRaftProv(Arc<MetaRaftNode>);
impl aidb::cluster::lifecycle_manager::MetaRaftProvider for MetaRaftProv {
    fn get_cluster_meta(&self) -> aidb::cluster::ClusterMeta {
        self.0.get_cluster_meta()
    }
    fn get_slot_table(&self) -> aidb::cluster::SlotTable {
        self.0.get_slot_table()
    }
    fn get_migration_state(&self) -> Option<aidb::cluster::SlotMigrationState> {
        self.0.get_migration_state()
    }
}

/// Plain DEL: key 存在时返回 true 且之后 GET 为 None; 不存在返回 false 且不失败.
#[tokio::test]
async fn test_plain_delete_return_matches_pre_read_existed() {
    let _mgr = build_plain_mgr().await;
    let local = AiDbEngine::open_for_testing(
        std::env::temp_dir().join(format!("aikv_group_batcher_local_{}", std::process::id())),
    )
    .unwrap();
    let adapter = ClusterDataAdapter::new(local, ClusterDataAdapter::DEFAULT_EAGER_FLUSH);

    let key = AiDbEngine::encode_key(0, b"plain_del{b}");
    let missing = AiDbEngine::encode_key(0, b"missing_del{b}");

    // 不存在: 返回 false, 不报错.
    let deleted = adapter.delete(missing.clone()).await.expect("missing delete must Ok");
    assert!(!deleted, "delete missing key must return false");
    assert_eq!(adapter.get(missing).await.unwrap(), None);

    // 存在: SET 后 DEL 返回 true, GET 为 None.
    adapter
        .set(key.clone(), b"present".to_vec())
        .await
        .expect("plain set must succeed");
    assert_eq!(
        adapter.get(key.clone()).await.unwrap(),
        Some(b"present".to_vec())
    );
    let deleted = adapter.delete(key.clone()).await.expect("existing delete must Ok");
    assert!(deleted, "delete existing key must return true");
    assert_eq!(
        adapter.get(key).await.unwrap(),
        None,
        "after delete GET must be None"
    );
}
