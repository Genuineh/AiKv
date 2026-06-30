//! aikv 入口: 加载配置, 初始化 tracing, 启动 TCP 服务.
#![recursion_limit = "256"]

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

#[cfg(feature = "cluster")]
#[allow(unused_imports)]
use std::collections::HashMap;
#[cfg(feature = "cluster")]
#[allow(unused_imports)]
use std::path::Path;

#[cfg(feature = "cluster")]
use aikv::cluster::state::DEFAULT_DATA_PORT_OFFSET;
use aikv::command::blocking;
use aikv::server::{ConnectionConfig, Server, ServerSharedState};
use aikv::storage::{
    server_db_options, AiDbEngine, KvStorage, KvStorageAdapter, MemoryEngine, StorageEngineKind,
    StorageObservation,
};
use clap::{Parser as ClapParser, ValueEnum};
use tracing_subscriber::{fmt, prelude::*, EnvFilter, Registry};

#[derive(Clone, Copy, Debug, ValueEnum)]
enum EngineKind {
    Memory,
    #[value(name = "aidb")]
    AiDb,
}

#[derive(ClapParser, Debug)]
#[command(name = "aikv", about = "Redis RESP compatible KV server")]
struct Args {
    /// 监听地址 (host:port)
    #[arg(long, default_value = "127.0.0.1:6379")]
    bind: SocketAddr,

    /// 存储引擎: memory (默认) 或 aidb
    #[arg(long, value_enum, default_value_t = EngineKind::Memory)]
    engine: EngineKind,

    /// AiDb 数据目录 (--engine aidb 时必填)
    #[arg(long)]
    data_dir: Option<PathBuf>,

    /// 每条写后 fsync WAL (强持久, 低吞吐; 默认 false)
    #[arg(long, default_value_t = false)]
    sync_wal: bool,

    /// BGSAVE 目标目录 (默认 {data_dir}/backup/)
    #[arg(long)]
    backup_dir: Option<PathBuf>,

    /// 集群模式节点 ID (启用 cluster feature 时有效)
    #[cfg(feature = "cluster")]
    #[arg(long)]
    cluster_node_id: Option<u64>,

    /// 集群模式 RPC 地址 (启用 cluster feature 时有效)
    #[cfg(feature = "cluster")]
    #[arg(long)]
    cluster_rpc_addr: Option<String>,

    /// 集群模式已有节点地址列表 (逗号分隔, 首节点不填)
    #[cfg(feature = "cluster")]
    #[arg(long, value_delimiter = ',')]
    cluster_peers: Vec<String>,

    /// Raft election timeout min (ms), default 1000
    #[cfg(feature = "cluster")]
    #[arg(long, default_value = "1000")]
    raft_election_timeout_min: u64,

    /// Raft election timeout max (ms), default 2000
    #[cfg(feature = "cluster")]
    #[arg(long, default_value = "2000")]
    raft_election_timeout_max: u64,

    /// Raft RPC (AppendEntries/Vote) 超时 (ms), default 500. 必须 < election_timeout_min
    #[cfg(feature = "cluster")]
    #[arg(long, default_value = "500", verbatim_doc_comment)]
    raft_rpc_timeout_ms: u64,

    /// Raft heartbeat 间隔 (ms), default 300. 必须 < election_timeout_min
    #[cfg(feature = "cluster")]
    #[arg(long, default_value = "300")]
    raft_heartbeat_interval: u64,

    /// Metrics HTTP 服务端口 (默认 9191)
    #[arg(long, default_value = "9191")]
    metrics_port: u16,

    /// Metrics HTTP 服务监听地址 (默认 127.0.0.1)
    #[arg(long, default_value = "127.0.0.1")]
    metrics_addr: String,

    /// 最大并发客户端连接数 (0 表示不限制, 默认 10000)
    #[arg(long, default_value = "10000")]
    max_clients: usize,

    /// 生命周期管理 tick 间隔 (毫秒, 默认 1000)
    #[cfg(feature = "cluster")]
    #[arg(long, default_value = "1000")]
    lifecycle_tick_ms: u64,

    /// Gossip 后台刷新间隔 (秒, 默认 1)
    #[cfg(feature = "cluster")]
    #[arg(long, default_value = "1")]
    gossip_interval: u64,

    /// 集群配置自动保存轮询间隔 (毫秒, 默认 2000)
    #[cfg(feature = "cluster")]
    #[arg(long, default_value = "2000")]
    config_auto_save_ms: u64,

    /// 数据面总线端口偏移 (默认 10000, 与 Redis Cluster @cport 约定一致).
    /// data_port = rpc_port + 此偏移值.
    #[cfg(feature = "cluster")]
    #[arg(long, default_value_t = DEFAULT_DATA_PORT_OFFSET)]
    cluster_data_port_offset: u16,
}

/// Storage 构建结果: (storage, engine_kind, data_dir, 可选的 DB 引用)
type StorageBuildResult = Result<
    (
        Arc<dyn KvStorage>,
        StorageEngineKind,
        Option<PathBuf>,
        Option<Arc<aidb::DB>>,
    ),
    String,
>;

fn build_storage(args: &Args, observation: Arc<StorageObservation>) -> StorageBuildResult {
    match args.engine {
        EngineKind::Memory => Ok((
            MemoryEngine::with_observation(16, Some(observation)),
            StorageEngineKind::Memory,
            None,
            None,
        )),
        EngineKind::AiDb => {
            let path = args
                .data_dir
                .as_ref()
                .ok_or_else(|| "--data-dir required for aidb engine".to_string())?;
            let engine = AiDbEngine::open_with_options(path, server_db_options(args.sync_wal))
                .map_err(|e| e.to_string())?;
            let db = engine.db.clone();
            let adapter: Arc<dyn aikv::storage::StorageAdapter> = engine;
            #[cfg(feature = "cluster")]
            let adapter: Arc<dyn aikv::storage::StorageAdapter> =
                aikv::storage::cluster_adapter::ClusterDataAdapter::new(adapter);
            Ok((
                KvStorageAdapter::with_observation(adapter, Some(observation)),
                StorageEngineKind::AiDb,
                Some(path.clone()),
                Some(db),
            ))
        }
    }
}

#[cfg(feature = "cluster")]
fn spawn_client_addr_sync(
    meta_raft: std::sync::Arc<aidb::cluster::MetaRaftNode>,
    node_id: u64,
    external_client_addr: String,
) {
    use aidb::cluster::meta_types::MetaRequest;

    tokio::spawn(async move {
        for attempt in 1u32..=20 {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            let meta = meta_raft.get_cluster_meta();
            match meta.nodes.get(&node_id) {
                Some(node) if node.client_addr.as_deref() != Some(&external_client_addr) => {
                    match meta_raft
                        .propose(MetaRequest::UpdateNodeClientAddr {
                            node_id,
                            client_addr: Some(external_client_addr.clone()),
                        })
                        .await
                    {
                        Ok(_) => {
                            tracing::info!(
                              client_addr = %external_client_addr,
                              attempt,
                              "client_addr updated in MetaRaft"
                            );
                            break;
                        }
                        Err(e) => {
                            tracing::warn!(
                              attempt,
                              error = %e,
                              "failed to update client_addr, will retry"
                            );
                        }
                    }
                }
                Some(_) => {
                    tracing::info!("client_addr already correct, skipping update");
                    break;
                }
                None => {
                    // bootstrap 或尚未 MEET 注册, 继续等待.
                }
            }
        }
    });
}

#[cfg(feature = "cluster")]
#[allow(clippy::too_many_arguments)]
async fn init_cluster(
    node_id: u64,
    rpc_addr: &str,
    peers: &[String],
    data_dir: &Path,
    bind_addr: SocketAddr,
    cluster_db: Option<Arc<aidb::DB>>,
    raft_election_timeout_min: u64,
    raft_election_timeout_max: u64,
    raft_rpc_timeout_ms: u64,
    raft_heartbeat_interval: u64,
    lifecycle_tick_ms: u64,
    gossip_interval: u64,
    config_auto_save_ms: u64,
    cluster_data_port_offset: u16,
    sync_wal: bool,
    metrics: Arc<aikv::server::metrics::ServerMetrics>,
) -> Result<(), Box<dyn std::error::Error>> {
    use std::net::SocketAddr as NetAddr;

    use aidb::cluster::{
        membership_coordinator::MembershipCoordinator,
        meta_types::{default_slot_table, ClusterMeta, SlotMigrationState, SlotTable},
        slot_migration::SlotMigrationManager,
        LeaderChangeWatcher, LifecycleManager, MetaRaftNode, MultiRaftNode,
        RaftNetworkClientFactory, RaftNodeConfig, RaftServiceDispatcher, Router,
    };
    use aidb::config::MigrationConfig;

    // 1. 打开/重用 DB 引擎 (WAL 必须启用)
    let db = match cluster_db {
        Some(db) => db,
        None => {
            let opts = server_db_options(sync_wal);
            aidb::DB::open(data_dir, opts)?
        }
    };

    // 2. 创建 RaftNetworkClientFactory
    let net_factory = RaftNetworkClientFactory::new(node_id, 0, 30, 64 * 1024 * 1024);
    let net_factory = Arc::new(parking_lot::RwLock::new(net_factory));

    // 3. 创建 MetaRaftNode (控制平面, group_id=0)
    let raft_config = RaftNodeConfig {
        node_id,
        group_id: 0,
        election_timeout_min: raft_election_timeout_min,
        election_timeout_max: raft_election_timeout_max,
        heartbeat_interval: raft_heartbeat_interval,
        max_payload_entries: 100,
        snapshot_logs_since_last: 256,
        max_entry_size: 8 * 1024 * 1024,
        rpc_timeout_ms: raft_rpc_timeout_ms,
        grpc_max_message_size: 64 * 1024 * 1024,
    };
    let factory = net_factory.read().clone(); // drop read lock before .await
    let meta_raft = Arc::new(MetaRaftNode::new(raft_config.clone(), db.clone(), factory).await?);

    // 4. 注册 peer 地址到 net_factory
    for peer_addr in peers {
        if peer_addr == rpc_addr {
            continue;
        }
        net_factory.write().add_node(0, peer_addr.clone());
    }

    // 5. 确定本节点的 client_addr (外部可达地址).
    // bootstrap 节点通过 initialize_with_client 设置; 加入节点通过后台 task
    // 等待 MEET 注册后通过 MetaRaft proposal 更新.
    let external_client_addr = std::env::var("AIKV_CLIENT_ADDR").unwrap_or_else(|_| {
        let rpc_host = rpc_addr
            .rsplit_once(':')
            .map(|(h, _)| h.to_string())
            .unwrap_or_default();
        format!("{rpc_host}:{}", bind_addr.port())
    });

    // 6. 首节点 bootstrap / 从节点自动发现
    if peers.is_empty() {
        meta_raft
            .initialize_with_client(vec![(
                node_id,
                rpc_addr.to_string(),
                Some(external_client_addr.clone()),
            )])
            .await?;
        tracing::info!("meta raft initialized as bootstrap node");
    } else {
        tracing::info!("meta raft joining via peers (gRPC auto-discovery)");
    }

    // 6. 创建共享 RaftServiceDispatcher (MetaRaft + MultiRaft 共用).
    // MetaRaft gRPC 使用共享 dispatcher 使其端口也能路由数据 group 的 Raft 消息,
    // 解决 add_learner_to_group 等操作使用 MetaRaft RPC 地址导致的端口错位.
    let dispatcher: Arc<RaftServiceDispatcher> = Arc::new(RaftServiceDispatcher::new());

    // 7. 启动 MetaRaft gRPC (独立端口, 使用共享 dispatcher).
    // 使用 lookup_host 支持 Docker hostname 解析 (SocketAddr::parse 仅支持 IP).
    let rpc_socket: NetAddr = tokio::net::lookup_host(rpc_addr)
        .await?
        .next()
        .ok_or_else(|| format!("cannot resolve RPC address: {rpc_addr}"))?;
    let meta_raft_clone = Arc::clone(&meta_raft);
    let dispatcher_for_meta = Arc::clone(&dispatcher);
    tokio::spawn(async move {
        if let Err(e) = meta_raft_clone
            .start_server_with_dispatcher(rpc_socket, 64 * 1024 * 1024, dispatcher_for_meta)
            .await
        {
            tracing::error!(error = %e, "meta raft gRPC server failed");
        }
    });
    tracing::info!(rpc_addr = %rpc_addr, "meta raft gRPC server started");

    // 6a. 所有节点后台同步 AIKV_CLIENT_ADDR → MetaRaft (含 bootstrap).
    spawn_client_addr_sync(
        Arc::clone(&meta_raft),
        node_id,
        external_client_addr.clone(),
    );

    // 7. 推算 MultiRaft 总线端口 (rpc_port + cluster_data_port_offset, 与 @cport 约定一致)
    let rpc_port_u16: u16 = rpc_socket.port();
    if rpc_port_u16 > 65535u16 - cluster_data_port_offset {
        return Err(format!(
            "RPC port {} is too large: rpc_port + {} = {} exceeds u16::MAX (65535). \
       Use a port <= {}.",
            rpc_port_u16,
            cluster_data_port_offset,
            rpc_port_u16 as u32 + cluster_data_port_offset as u32,
            65535u16 - cluster_data_port_offset,
        )
        .into());
    }
    let data_port = rpc_port_u16 + cluster_data_port_offset;
    let data_addr = format!("{}:{}", rpc_socket.ip(), data_port);

    // 8. 创建 Router (初始空表, 由 LifecycleManager 刷新)
    let router = Arc::new(Router::new(
        default_slot_table(),
        HashMap::new(),
        HashMap::new(),
    ));

    // 9. MetaRaftProvider 包装 (MetaRaftNode 未实现 trait, 需手动包装)
    struct MetaRaftProv(Arc<MetaRaftNode>);
    impl aidb::cluster::lifecycle_manager::MetaRaftProvider for MetaRaftProv {
        fn get_cluster_meta(&self) -> ClusterMeta {
            self.0.get_cluster_meta()
        }
        fn get_slot_table(&self) -> SlotTable {
            self.0.get_slot_table()
        }
        fn get_migration_state(&self) -> Option<SlotMigrationState> {
            self.0.get_migration_state()
        }
    }

    // 10. 创建 LifecycleManager + MultiRaftNode (复用共享 dispatcher).
    let meta_raft_prov = Arc::new(MetaRaftProv(Arc::clone(&meta_raft)));
    let lifecycle = LifecycleManager::new(node_id, router.clone(), meta_raft_prov)
        .with_tick_interval(std::time::Duration::from_millis(lifecycle_tick_ms));
    let multi_raft =
        MultiRaftNode::new_with_lifecycle(node_id, router.clone(), dispatcher.clone(), lifecycle);
    let multi_raft = Arc::new(multi_raft);

    // 11. 启动 Lifecycle (数据 Group 自动创建/销毁)
    let lifecycle_cfg = aidb::cluster::multi_raft_node::LifecycleConfig {
        data_dir: data_dir.to_path_buf(),
        raft_node_config: raft_config,
        options: server_db_options(sync_wal),
    };
    let _lifecycle_shutdown = multi_raft.start_lifecycle_with_data(lifecycle_cfg);

    // 12. 启动数据面 gRPC server (spawn 后台任务)
    let data_socket: NetAddr = data_addr.parse()?;
    let mr_for_grpc = Arc::clone(&multi_raft);
    tokio::spawn(async move {
        if let Err(e) = mr_for_grpc.start(data_socket, 64 * 1024 * 1024).await {
            tracing::error!(error = %e, "multi raft gRPC server failed");
        }
    });
    tracing::info!(data_addr = %data_addr, "multi raft gRPC server starting");

    // 14. 创建 MembershipCoordinator
    let membership = Arc::new(MembershipCoordinator::new(
        meta_raft.clone(),
        multi_raft.clone(),
        router.clone(),
        node_id,
    ));

    // 15. 创建 ClusterStateManager, 注入 membership, 设置全局单例
    let mut state_mgr = aikv::cluster::state::ClusterStateManager::new(
        router.as_ref().clone(),
        meta_raft.clone(),
        multi_raft.clone(),
        node_id,
    );
    state_mgr.set_membership_coordinator(membership.clone());
    let slot_migration = Arc::new(SlotMigrationManager::new(
        meta_raft.clone(),
        multi_raft.clone(),
        router.clone(),
        node_id,
        data_dir.join("slot_migration"),
        MigrationConfig::default(),
    ));
    state_mgr.set_slot_migration_manager(slot_migration);
    state_mgr.data_dir = Some(data_dir.to_path_buf());
    state_mgr.data_port_offset = cluster_data_port_offset;
    state_mgr.metrics = Some(metrics);
    tracing::info!(
      announce_mode = ?state_mgr.announce_resolver.mode(),
      client_addr = %external_client_addr,
      "cluster announce resolver ready"
    );
    let state_mgr = Arc::new(state_mgr);
    let _ = aikv::cluster::state::CLUSTER_STATE_MGR.set(state_mgr);

    // 16. 启动拓扑后台刷新 (leader 缓存 + gossip metrics)
    let _topology_refresh_handle = aikv::cluster::gossip::start_background_refresh(
        aikv::cluster::state::CLUSTER_STATE_MGR
            .get()
            .unwrap()
            .clone(),
        gossip_interval,
    );

    // 17. 启动 LeaderChangeWatcher (tick 后同步观测 leader 到路由缓存)
    let tick_ms = (raft_election_timeout_min / 2).max(100);
    let leader_watcher = std::sync::Arc::new(LeaderChangeWatcher::new(
        node_id,
        multi_raft.clone(),
        meta_raft.clone(),
        std::time::Duration::from_millis(tick_ms),
    ));
    let (watcher_tx, watcher_rx) = tokio::sync::watch::channel(false);
    let watcher_for_task = leader_watcher.clone();
    let tick_duration = std::time::Duration::from_millis(tick_ms);
    tokio::spawn(async move {
        let mut shutdown_rx = watcher_rx;
        loop {
            tokio::select! {
              _ = tokio::time::sleep(tick_duration) => {
                let changed = watcher_for_task.tick().await;
                if let Some(mgr) = aikv::cluster::state::CLUSTER_STATE_MGR.get() {
                  let cache = watcher_for_task.leader_cache().read();
                  for group_id in changed {
                    if let Some(leader_id) = cache.get(&group_id).and_then(|v| *v) {
                      mgr.apply_observed_group_leader(group_id, leader_id);
                    }
                  }
                }
              }
              _ = shutdown_rx.changed() => {
                tracing::info!("LeaderChangeWatcher shutting down");
                break;
              }
            }
        }
    });
    // Store tx in global state manager so the sender outlives init_cluster().
    // Previously the sender was dropped on return, causing the watcher
    // to shut down immediately.
    if let Some(mgr) = aikv::cluster::state::CLUSTER_STATE_MGR.get() {
        *mgr._watcher_shutdown.lock() = Some(watcher_tx);
    }
    tracing::info!(tick_ms, "LeaderChangeWatcher started");

    // 18. 启动 ConfigAutoSave
    let auto_save = aikv::cluster::config_auto_save::ConfigAutoSave::new_with_interval(
        meta_raft.clone(),
        data_dir.to_path_buf(),
        std::time::Duration::from_millis(config_auto_save_ms),
    );
    let (auto_save_tx, auto_save_rx) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        auto_save.run(auto_save_rx).await;
    });
    // Same pattern as watcher — store in global state manager.
    if let Some(mgr) = aikv::cluster::state::CLUSTER_STATE_MGR.get() {
        *mgr._auto_save_shutdown.lock() = Some(auto_save_tx);
    }
    tracing::info!("ConfigAutoSave started");

    tracing::info!(node_id, rpc_addr, "cluster initialized successfully");
    Ok(())
}

#[allow(unused_variables)]
fn init_logging(
    tcp_port: u16,
    #[cfg(feature = "cluster")] cluster_node_id: Option<u64>,
    #[cfg(not(feature = "cluster"))] cluster_node_id: Option<u64>,
) {
    let json_enabled = std::env::var("AIKV_JSON_LOG")
        .ok()
        .and_then(|v| v.parse::<bool>().ok())
        .unwrap_or(true);

    let filter = EnvFilter::builder()
        .with_default_directive("info".parse().unwrap())
        .from_env_lossy();

    #[cfg(feature = "monitoring")]
    if let Some(config) = aikv::server::otel::otel_config_from_env(tcp_port, cluster_node_id) {
        if aikv::server::otel::init_otel(&config) {
            let tracer = opentelemetry::global::tracer("aikv");
            let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);
            let base = Registry::default().with(otel_layer);
            let subscriber = if json_enabled {
                base.with(
                    fmt::layer()
                        .json()
                        .flatten_event(false)
                        .with_current_span(true)
                        .with_span_list(true)
                        .boxed(),
                )
                .with(filter)
            } else {
                base.with(fmt::layer().compact().with_target(true).boxed())
                    .with(filter)
            };
            let _ = subscriber.try_init();
            return;
        }
    }

    // 无 OTel 时只初始化 fmt subscriber
    let subscriber = if json_enabled {
        Registry::default()
            .with(
                fmt::layer()
                    .json()
                    .flatten_event(false)
                    .with_current_span(true)
                    .with_span_list(true)
                    .boxed(),
            )
            .with(filter)
    } else {
        Registry::default()
            .with(fmt::layer().compact().with_target(true).boxed())
            .with(filter)
    };
    if let Err(e) = subscriber.try_init() {
        eprintln!("warn: tracing subscriber already initialized ({})", e);
    }
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    init_logging(
        args.bind.port(),
        #[cfg(feature = "cluster")]
        args.cluster_node_id,
        #[cfg(not(feature = "cluster"))]
        None,
    );
    if matches!(args.engine, EngineKind::Memory) {
        eprintln!(
      "WARN: engine=memory — data is NOT persisted; use --engine aidb --data-dir for production"
    );
    }
    if matches!(args.engine, EngineKind::AiDb) && args.data_dir.is_none() {
        eprintln!("error: --data-dir is required when --engine aidb");
        std::process::exit(1);
    }

    tracing::info!(bind = %args.bind, engine = ?args.engine, "aikv starting");

    let observation = StorageObservation::new();
    let (storage, engine_kind, data_dir, _cluster_db) =
        match build_storage(&args, observation.clone()) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(error = %e, "failed to initialize storage");
                std::process::exit(1);
            }
        };
    let tcp_port = args.bind.port();
    let connection_config = ConnectionConfig {
        max_clients: args.max_clients,
        ..Default::default()
    };
    let state = ServerSharedState::new_with_backup_dir(
        connection_config,
        storage,
        tcp_port,
        engine_kind,
        data_dir,
        args.backup_dir,
        args.metrics_port,
        args.metrics_addr.clone(),
        Some(observation),
    );

    // 集群初始化 (等待 init_cluster 完成, 确保 CLUSTER_STATE_MGR 就绪)
    #[cfg(feature = "cluster")]
    if let (Some(node_id), Some(rpc_addr)) = (args.cluster_node_id, args.cluster_rpc_addr.as_ref())
    {
        let d = args
            .data_dir
            .as_ref()
            .expect("--data-dir is required for cluster mode");
        if let Err(e) = init_cluster(
            node_id,
            rpc_addr,
            &args.cluster_peers,
            d,
            args.bind,
            _cluster_db,
            args.raft_election_timeout_min,
            args.raft_election_timeout_max,
            args.raft_rpc_timeout_ms,
            args.raft_heartbeat_interval,
            args.lifecycle_tick_ms,
            args.gossip_interval,
            args.config_auto_save_ms,
            args.cluster_data_port_offset,
            args.sync_wal,
            Arc::clone(&state.metrics),
        )
        .await
        {
            tracing::error!(error = %e, "cluster initialization failed");
            std::process::exit(1);
        }
    }

    // 启动 Metrics HTTP 服务器 (仅 monitoring feature)
    #[cfg(feature = "monitoring")]
    {
        let metrics_addr_str = format!("{}:{}", args.metrics_addr, args.metrics_port);
        match metrics_addr_str.parse::<std::net::SocketAddr>() {
            Ok(metrics_addr) => {
                let metrics_server =
                    aikv::server::metrics_server::MetricsServer::new(metrics_addr);
                tokio::spawn(async move {
                    metrics_server.run().await;
                });
            }
            Err(_) => {
                tracing::warn!(addr = %metrics_addr_str, "invalid metrics address, metrics server disabled");
            }
        }

        // 后台定期刷新 uptime / memory 指标
        let bg_state = Arc::clone(&state);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(15));
            loop {
                interval.tick().await;
                bg_state.refresh_runtime_metrics().await;
                bg_state.metrics().refresh_process_metrics();
            }
        });
    }

    blocking::start_background_eviction();

    if let Err(e) = Server::run(args.bind, state).await {
        tracing::error!(error = %e, "server exited with error");
        #[cfg(feature = "monitoring")]
        aikv::server::otel::shutdown_otel();
        std::process::exit(1);
    }

    #[cfg(feature = "monitoring")]
    aikv::server::otel::shutdown_otel();
}
