//! Redis 7 INFO 字段存在性 golden 测试 (Phase 4).

use super::helpers::{b, router_with_shared};

const INFO_P0_FIELDS: &str = include_str!("../../fixtures/redis88_info_p0_fields.txt");

fn info_text(resp: aikv::protocol::RespValue) -> String {
    let aikv::protocol::RespValue::BulkString(Some(text)) = resp else {
        panic!("expected bulk string");
    };
    String::from_utf8_lossy(&text).into_owned()
}

fn parse_field_list(raw: &str) -> Vec<String> {
    raw.lines()
        .filter(|line| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
        .map(|line| line.trim().to_string())
        .collect()
}

fn info_has_field(text: &str, field: &str) -> bool {
    let prefix = format!("{field}:");
    text.lines().any(|line| line.starts_with(&prefix))
}

#[tokio::test]
async fn info_redis88_all_p0_fields_present() {
    let (router, shared) = router_with_shared();
    let mut db = 0;

    router
        .execute("SET", &[b("golden"), b("v")], &mut db)
        .await
        .unwrap();
    router
        .execute("GET", &[b("golden")], &mut db)
        .await
        .unwrap();
    shared.refresh_runtime_metrics().await;

    let text = info_text(router.execute("INFO", &[b("all")], &mut db).await.unwrap());
    let fields = parse_field_list(INFO_P0_FIELDS);
    let missing: Vec<_> = fields
        .iter()
        .filter(|field| !info_has_field(&text, field))
        .cloned()
        .collect();
    assert!(
        missing.is_empty(),
        "INFO all missing Redis 8.8 P0 fields: {}",
        missing.join(", ")
    );
    assert!(text.contains("db0:keys="), "keyspace db0 line missing");
}

#[tokio::test]
async fn info_all_extra_sections_present() {
    let (router, _shared) = router_with_shared();
    let mut db = 0;
    router
        .execute("SET", &[b("k"), b("v")], &mut db)
        .await
        .unwrap();

    let text = info_text(router.execute("INFO", &[b("all")], &mut db).await.unwrap());
    for section in ["Commandstats", "Errorstats", "Threads", "Latencystats"] {
        assert!(
            text.contains(&format!("# {section}")),
            "missing section header: {section}"
        );
    }
}

#[tokio::test]
async fn info_everything_includes_modules_section() {
    let (router, _shared) = router_with_shared();
    let mut db = 0;
    let text = info_text(
        router
            .execute("INFO", &[b("everything")], &mut db)
            .await
            .unwrap(),
    );
    assert!(text.contains("# Modules"));
}

const INFO_FULL_FIELDS: &str = include_str!("../../fixtures/redis88_info_full_fields.txt");

#[tokio::test]
async fn info_full_fields_present_in_everything() {
    let (router, shared) = router_with_shared();
    let mut db = 0;
    router
        .execute("SET", &[b("golden"), b("v")], &mut db)
        .await
        .unwrap();
    shared.refresh_runtime_metrics().await;

    let text = info_text(
        router
            .execute("INFO", &[b("everything")], &mut db)
            .await
            .unwrap(),
    );
    let fields = parse_field_list(INFO_FULL_FIELDS);
    let missing: Vec<_> = fields
        .iter()
        .filter(|field| !info_has_field(&text, field))
        .cloned()
        .collect();
    assert!(
        missing.is_empty(),
        "INFO everything missing Redis 8.8 fields: {}",
        missing.join(", ")
    );
}

#[cfg(feature = "cluster")]
#[test]
fn cluster_info_redis7_p0_fields_present() {
    use std::collections::HashMap;
    use std::sync::atomic::AtomicU64;
    use std::sync::Arc;

    use aidb::cluster::meta_types::{default_slot_table, SlotStatus};
    use aidb::cluster::{MultiRaftNode, RaftServiceDispatcher, Router};
    use parking_lot::RwLock;

    use aikv::cluster::announce::AnnounceResolver;
    use aikv::cluster::cluster_info;
    use aikv::cluster::state::{
        ClusterStateManager, ReplicationRole, CLUSTER_STATE_MGR, DEFAULT_DATA_PORT_OFFSET,
    };

    const CLUSTER_P0: &str = include_str!("../../fixtures/redis7_cluster_info_p0_fields.txt");

    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let mut group_nodes = HashMap::new();
            group_nodes.insert(1u64, vec![1u64, 2u64, 3u64]);
            let mut node_addrs = HashMap::new();
            node_addrs.insert(1u64, "127.0.0.1:7001".to_string());
            let mut table = default_slot_table();
            for s in 0u16..5 {
                table[s as usize] = SlotStatus::Assigned(1);
            }
            let router = Router::new(table.clone(), group_nodes, node_addrs);
            let dispatcher = Arc::new(RaftServiceDispatcher::new());
            let multi_raft = Arc::new(MultiRaftNode::new(1, Arc::new(router.clone()), dispatcher));
            let db = aidb::DB::open(
                std::env::temp_dir().join(format!("info_golden_cluster_{}", std::process::id())),
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

            let mgr = ClusterStateManager {
                router,
                meta_raft: Arc::new(meta_raft),
                multi_raft,
                node_id: 1,
                config_epoch: AtomicU64::new(0),
                role: RwLock::new(ReplicationRole::Primary),
                local_group_leaders: RwLock::new(HashMap::from([(1u64, true)])),
                membership_coordinator: None,
                slot_migration_manager: None,
                data_dir: None,
                importing_slots: RwLock::new(HashMap::new()),
                data_port_offset: DEFAULT_DATA_PORT_OFFSET,
                announce_resolver: AnnounceResolver::default(),
                metrics: Some(Arc::new(aikv::server::metrics::ServerMetrics::default())),
                _watcher_shutdown: parking_lot::Mutex::new(None),
                _auto_save_shutdown: parking_lot::Mutex::new(None),
            };
            let _ = CLUSTER_STATE_MGR.set(Arc::new(mgr));
        });
    });

    let info = cluster_info().expect("cluster initialized");
    let fields = parse_field_list(CLUSTER_P0);
    let missing: Vec<_> = fields
        .iter()
        .filter(|field| !info_has_field(&info, field))
        .cloned()
        .collect();
    assert!(
        missing.is_empty(),
        "CLUSTER INFO missing P0 fields: {}",
        missing.join(", ")
    );
}
