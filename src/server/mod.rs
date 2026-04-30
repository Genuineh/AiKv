pub mod connection;
pub mod monitor;

pub use monitor::{MonitorBroadcaster, MonitorMessage};

use self::connection::Connection;
use crate::command::server::{ClientInfo, ServerCommands};
use crate::command::CommandExecutor;
use crate::error::Result;
use crate::observability::{KeyspaceCache, Metrics, SlowQueryLog};
use crate::storage::StorageEngine;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, RwLock};
use std::time::Instant;
use tokio::net::TcpListener;
use tracing::{error, info, warn, Level};

#[cfg(feature = "cluster")]
use crate::cluster::{ClusterCommands, MetaRaftNode, MultiRaftNode, Router};

/// First-time MetaRaft bootstrap. If the node restarted with data on disk, OpenRaft returns
/// `NotAllowed` for duplicate `initialize()`; treat that as success.
#[cfg(feature = "cluster")]
async fn bootstrap_meta_raft_if_empty(
    multi_raft: &MultiRaftNode,
    node_id: u64,
    raft_addr: &str,
) -> std::result::Result<(), crate::error::AikvError> {
    match multi_raft
        .initialize_meta_cluster(vec![(node_id, raft_addr.to_string())])
        .await
    {
        Ok(()) => Ok(()),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("NotAllowed") {
                info!(
                    "MetaRaft bootstrap skipped: persisted cluster already initialized (OpenRaft NotAllowed)"
                );
                Ok(())
            } else {
                Err(crate::error::AikvError::Internal(format!(
                    "Failed to bootstrap MetaRaft: {}",
                    e
                )))
            }
        }
    }
}

/// AiKv server
pub struct Server {
    addr: String,
    port: u16,
    storage: StorageEngine,
    metrics: Arc<Metrics>,
    monitor_broadcaster: Arc<MonitorBroadcaster>,
    /// Shared server config — persists CONFIG SET changes across connections.
    shared_config: Arc<RwLock<HashMap<String, String>>>,
    /// Shared slow query log — persists threshold changes across connections.
    slow_query_log: Arc<SlowQueryLog>,
    /// Shared client registry — CLIENT LIST reflects all active connections.
    clients: Arc<RwLock<HashMap<usize, ClientInfo>>>,
    /// Shared log level — CONFIG SET loglevel takes effect server-wide.
    current_log_level: Arc<RwLock<Level>>,
    #[cfg(feature = "cluster")]
    node_id: u64,
    #[cfg(feature = "cluster")]
    meta_raft: Option<Arc<MetaRaftNode>>,
    #[cfg(feature = "cluster")]
    multi_raft: Option<Arc<MultiRaftNode>>,
    #[cfg(feature = "cluster")]
    router: Option<Arc<Router>>,
    #[cfg(feature = "cluster")]
    cluster_commands: Option<Arc<ClusterCommands>>,
}

impl Server {
    /// Create a new server with the specified address and storage engine
    pub fn new(addr: String, mut storage: StorageEngine) -> Self {
        // Extract port from address string using proper SocketAddr parsing
        // This handles both IPv4 (127.0.0.1:6379) and IPv6 ([::1]:6379) formats
        let port = addr
            .parse::<SocketAddr>()
            .map(|a| a.port())
            .unwrap_or_else(|_| {
                // Fallback: try to extract port from the end after last ':'
                // This handles edge cases where the string isn't a valid SocketAddr
                addr.rsplit(':')
                    .next()
                    .and_then(|p| p.trim_end_matches(']').parse().ok())
                    .unwrap_or(6379)
            });

        #[cfg(feature = "cluster")]
        let node_id = {
            // Node ID will be set during initialize_cluster based on raft_address
            // This ensures consistent node IDs across restarts
            0
        };

        let metrics = Arc::new(Metrics::new());
        storage.set_lock_metrics(Arc::clone(&metrics.locks));

        Self {
            addr,
            port,
            storage,
            metrics,
            monitor_broadcaster: Arc::new(MonitorBroadcaster::new()),
            shared_config: Arc::new(RwLock::new(ServerCommands::default_config(port))),
            slow_query_log: Arc::new(SlowQueryLog::new()),
            clients: Arc::new(RwLock::new(HashMap::new())),
            current_log_level: Arc::new(RwLock::new(Level::INFO)),
            #[cfg(feature = "cluster")]
            node_id,
            #[cfg(feature = "cluster")]
            meta_raft: None,
            #[cfg(feature = "cluster")]
            multi_raft: None,
            #[cfg(feature = "cluster")]
            router: None,
            #[cfg(feature = "cluster")]
            cluster_commands: None,
        }
    }

    /// Set a value in the shared server configuration.
    pub fn set_config(&self, key: impl Into<String>, value: impl Into<String>) {
        let mut config = self.shared_config.write().unwrap_or_else(|e| e.into_inner());
        config.insert(key.into(), value.into());
    }

    /// Initialize cluster components (cluster feature only)
    #[cfg(feature = "cluster")]
    pub async fn initialize_cluster(
        &mut self,
        data_dir: &str,
        raft_addr: &str,
        is_bootstrap: bool,
        peers: &[String],
        storage_databases: usize,
    ) -> Result<()> {
        use openraft::Config as RaftConfig;
        use openraft::SnapshotPolicy;

        // Generate consistent node ID from raft address
        // This ensures the same node always gets the same ID across restarts
        let node_id = ClusterCommands::generate_node_id_from_addr(raft_addr);
        self.node_id = node_id;

        info!(
            "Initializing cluster: node_id={:040x}, raft_addr={}, bootstrap={}, peers={:?}",
            self.node_id, raft_addr, is_bootstrap, peers
        );

        let raft_config = RaftConfig {
            snapshot_policy: SnapshotPolicy::Never,
            ..RaftConfig::default()
        };

        // In cluster mode, AiDb WAL is redundant (Raft log ensures durability)
        let storage_opts = aidb::Options::default().use_wal(false);

        // Create MultiRaftNode with WAL disabled on AiDb instances for Raft groups
        let mut multi_raft = MultiRaftNode::new(
            self.node_id,
            std::path::Path::new(data_dir),
            raft_config.clone(),
            Some(storage_opts),
        )
        .await
        .map_err(|e| {
            crate::error::AikvError::Internal(format!("Failed to create MultiRaftNode: {}", e))
        })?;

        // Initialize MetaRaft
        multi_raft.init_meta_raft(raft_config).await.map_err(|e| {
            crate::error::AikvError::Internal(format!("Failed to init MetaRaft: {}", e))
        })?;

        // RaftCore publishes the first `RaftMetrics` asynchronously; reading the watch channel
        // immediately can still see `RaftMetrics::new_initial()` and falsely skip / duplicate init.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Check if the cluster is already initialized by checking Raft metrics
        // If there's already a committed vote or log entries, the cluster was previously initialized
        let already_initialized = {
            if let Some(meta_raft) = multi_raft.meta_raft() {
                let raft = meta_raft.raft();
                let metrics = raft.metrics().borrow().clone();

                // Check if there are any voters in the membership (excluding empty membership)
                let has_voters = !metrics
                    .membership_config
                    .membership()
                    .voter_ids()
                    .collect::<Vec<_>>()
                    .is_empty();

                let has_committed_log = metrics.last_applied.is_some();
                let has_log = metrics.last_log_index.map(|i| i > 0).unwrap_or(false);
                let has_committed_vote = metrics.vote.is_committed();

                if has_voters || has_committed_log || has_log || has_committed_vote {
                    info!(
                        "MetaRaft already initialized: has_voters={}, has_committed_log={}, has_log={}, has_committed_vote={}, membership={:?}",
                        has_voters,
                        has_committed_log,
                        has_log,
                        has_committed_vote,
                        metrics.membership_config.membership()
                    );
                    true
                } else {
                    false
                }
            } else {
                false
            }
        };

        // If bootstrap node and not already initialized, initialize MetaRaft cluster
        if is_bootstrap && !already_initialized {
            // For multi-master setup with pre-configured peers, we need a different approach:
            // All nodes must start BEFORE bootstrap can add them as voters
            // For now, bootstrap as single node and peers will be added dynamically via CLUSTER MEET
            // TODO: Implement proper multi-node bootstrap once all nodes are confirmed running

            if !peers.is_empty() && peers.len() > 1 {
                // Multi-master mode: bootstrap with just this node first
                // Other peers will be added as MetaRaft voters when they join via CLUSTER MEET
                info!("Multi-master mode: Bootstrapping with this node only. Peers will be added when they join: {:?}", peers);
                warn!("Multi-node bootstrap requires all peers to be running. For now, bootstrapping as single node.");
                warn!("Use dynamic membership via CLUSTER MEET to add other masters as MetaRaft voters.");
            } else {
                // Single-node bootstrap (standard behavior)
                info!("Single-node bootstrap");
            }

            bootstrap_meta_raft_if_empty(&multi_raft, self.node_id, raft_addr).await?;

            info!("Cluster bootstrap complete");
        } else if is_bootstrap && already_initialized {
            info!("Skipping cluster bootstrap - MetaRaft already initialized from persisted state");
        }

        // Router + data groups: required for replicated KV (same MetaRaft view as AiDb docs).
        multi_raft.init_router().map_err(|e| {
            crate::error::AikvError::Internal(format!("init_router failed: {}", e))
        })?;
        multi_raft
            .load_existing_groups()
            .await
            .map_err(|e| crate::error::AikvError::Internal(format!("load_existing_groups: {}", e)))?;
        multi_raft
            .sync_data_groups_from_meta()
            .await
            .map_err(|e| crate::error::AikvError::Internal(format!("sync_data_groups: {}", e)))?;

        // Wrap in Arc after initialization
        let multi_raft = Arc::new(multi_raft);

        // Start Raft network listener in background (gRPC server)
        let raft_addr_clone = raft_addr.to_string();
        let multi_raft_clone = multi_raft.clone();
        tokio::spawn(async move {
            info!("Starting Raft gRPC listener on {}", raft_addr_clone);

            // Extract port from raft address (which may be a hostname like "aikv1:50051")
            // and bind to 0.0.0.0:PORT to accept connections from all interfaces
            let port = raft_addr_clone
                .rsplit(':')
                .next()
                .and_then(|p| p.parse::<u16>().ok())
                .unwrap_or(50051);

            let bind_addr: SocketAddr = format!("0.0.0.0:{}", port)
                .parse()
                .expect("Failed to create bind address");

            info!(
                "Binding Raft gRPC server to {} (advertised as {})",
                bind_addr, raft_addr_clone
            );

            // Build the Raft gRPC service that dispatches to the MultiRaftNode
            let svc =
                aidb::cluster::raft_network::raft_rpc::raft_service_server::RaftServiceServer::new(
                    crate::cluster::raft_service::MultiRaftService::new(multi_raft_clone),
                );

            if let Err(e) = tonic::transport::Server::builder()
                .add_service(svc)
                .serve(bind_addr)
                .await
            {
                error!("Raft listener failed: {}", e);
                std::process::exit(1);
            }
        });

        // Get MetaRaftNode reference
        let meta_raft = multi_raft.meta_raft().ok_or_else(|| {
            crate::error::AikvError::Internal("MetaRaft not initialized".to_string())
        })?;

        // Share the same Router as MultiRaftNode (MetaRaft-backed cache).
        let router = multi_raft
            .router()
            .ok_or_else(|| crate::error::AikvError::Internal("Router missing".to_string()))?
            .clone();

        // Create ClusterCommands for this server (used for background tasks)
        let cluster_commands = Arc::new(ClusterCommands::new(
            self.node_id,
            Arc::clone(&meta_raft),
            Arc::clone(&multi_raft),
            Arc::clone(&router),
        ));

        self.meta_raft = Some(meta_raft.clone());
        let multi_arc = Arc::clone(&multi_raft);

        // Background watcher: periodically sync data groups + router from MetaRaft.
        Self::spawn_data_groups_watcher(Arc::clone(&multi_raft));

        self.multi_raft = Some(multi_raft);
        self.router = Some(router);
        self.cluster_commands = Some(cluster_commands.clone());

        // Spawn automatic failover monitoring task
        self.spawn_failover_monitor_task(cluster_commands);

        // Persisted Redis data goes through Multi-Raft (not the pre-opened per-DB files).
        self.storage = StorageEngine::new_cluster_raft(multi_arc, storage_databases);
        self.storage.set_lock_metrics(Arc::clone(&self.metrics.locks));

        let advertise_host = std::env::var("AIKV_ADVERTISE_HOST").unwrap_or_else(|_| String::new());
        let auto_failover = std::env::var("AIKV_AUTO_FAILOVER").unwrap_or_else(|_| String::new());
        info!(
            diag_event = "cluster_init_complete_before_redis_bind",
            node_id = format!("{:040x}", self.node_id),
            redis_listen = %self.addr,
            advertise_host = %advertise_host,
            auto_failover = %auto_failover,
            "cluster initialize_cluster finished; Redis bind happens next in run()"
        );

        Ok(())
    }

    /// Periodically ensure this node has Raft groups + fresh router for every group it
    /// participates in (includes `reconcile_data_group_membership` / `change_membership` on
    /// leaders). Interval: env **`AIKV_DATA_GROUPS_SYNC_INTERVAL_MS`** (default **2000**, min **100**).
    /// First tick runs after a fixed **1s** startup delay.
    #[cfg(feature = "cluster")]
    fn spawn_data_groups_watcher(multi_raft: Arc<aidb::cluster::MultiRaftNode>) {
        let interval_ms = std::env::var("AIKV_DATA_GROUPS_SYNC_INTERVAL_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|&ms| ms >= 100)
            .unwrap_or(2000);
        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            loop {
                if let Err(e) = multi_raft.sync_data_groups_from_meta().await {
                    tracing::warn!("data-groups watcher: sync failed: {}", e);
                }
                if let Some(r) = multi_raft.router() {
                    let _ = r.refresh_metadata();
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(interval_ms)).await;
            }
        });
    }

    /// Spawn background task that monitors for master failures and triggers automatic failover.
    ///
    /// This task runs every 5 seconds and checks if this node (as a replica) needs to
    /// take over as master due to the original master becoming unreachable.
    #[cfg(feature = "cluster")]
    fn spawn_failover_monitor_task(&self, cluster_commands: Arc<ClusterCommands>) {
        let auto_failover_enabled = std::env::var("AIKV_AUTO_FAILOVER")
            .map(|v| {
                let s = v.trim().to_ascii_lowercase();
                s == "1" || s == "true" || s == "yes" || s == "on"
            })
            .unwrap_or(false);
        if !auto_failover_enabled {
            info!("Automatic failover monitor disabled (AIKV_AUTO_FAILOVER not enabled)");
            return;
        }

        tokio::spawn(async move {
            info!("Starting automatic failover monitor task");

            // Initial delay to let the cluster stabilize
            tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;

            loop {
                match cluster_commands.trigger_automatic_failover_if_needed().await {
                    Ok(Some(group_id)) => {
                        info!(
                            "Automatic failover triggered successfully for group {}",
                            group_id
                        );
                    }
                    Ok(None) => {
                        // No failover needed, cluster is healthy
                    }
                    Err(e) => {
                        error!(
                            error = %e,
                            "Error during automatic failover check"
                        );
                    }
                }

                // Check every 5 seconds
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            }
        });
    }

    /// Get server metrics
    pub fn metrics(&self) -> Arc<Metrics> {
        Arc::clone(&self.metrics)
    }

    /// Get monitor broadcaster
    pub fn monitor_broadcaster(&self) -> Arc<MonitorBroadcaster> {
        Arc::clone(&self.monitor_broadcaster)
    }

    /// Spawn background task that refreshes keyspace stats every ~10 seconds.
    ///
    /// The scan uses AiDb's iterator which must merge ALL immutable MemTables.
    /// Under heavy write load this can take tens of seconds — far too slow to
    /// run inline inside an INFO request handler.  Running it here in a
    /// dedicated spawn_blocking loop keeps the request path instant (cache hit)
    /// while still providing reasonably fresh keyspace data.
    fn spawn_keyspace_refresh_task(&self) {
        let storage = self.storage.clone();
        let metrics = Arc::clone(&self.metrics);

        tokio::spawn(async move {
            // Brief initial delay so the first scan happens after the server is
            // fully ready rather than during startup.
            tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

            loop {
                let storage_c = storage.clone();
                let metrics_c = Arc::clone(&metrics);

                let scan_result = tokio::task::spawn_blocking(move || {
                    let t = Instant::now();
                    let mut all_stats: Vec<(usize, usize, u64)> = Vec::with_capacity(16);
                    for db_index in 0..16 {
                        match storage_c.keyspace_stats_in_db(db_index) {
                            Ok(s) => all_stats.push(s),
                            Err(_) => break,
                        }
                    }
                    (all_stats, t.elapsed())
                })
                .await;

                match scan_result {
                    Ok((stats, elapsed)) => {
                        if elapsed.as_secs() >= 5 {
                            warn!(
                                elapsed_ms = elapsed.as_millis(),
                                "background keyspace scan is slow (many unflushed MemTables)"
                            );
                        } else {
                            info!(
                                elapsed_ms = elapsed.as_millis(),
                                "background keyspace scan complete"
                            );
                        }
                        let mut cache = metrics_c
                            .keyspace_cache
                            .lock()
                            .unwrap_or_else(|e| e.into_inner());
                        *cache = Some(KeyspaceCache {
                            stats,
                            updated_at: Instant::now(),
                        });
                    }
                    Err(e) => {
                        error!("background keyspace scan task panicked: {}", e);
                    }
                }

                // Wait before the next scan so we don't pile up concurrent
                // scans when each one takes longer than the interval.
                tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
            }
        });
    }

    /// Run the server
    pub async fn run(&self) -> Result<()> {
        let listener = TcpListener::bind(&self.addr).await?;
        #[cfg(feature = "cluster")]
        info!(
            diag_event = "redis_listen_bound",
            redis_listen = %self.addr,
            "AiKv Redis protocol listening (query Loki: diag_event=redis_listen_bound)"
        );
        info!("AiKv server listening on {}", self.addr);

        // Kick off background tasks before accepting connections.
        // Refresh keyspace stats cache — keeps INFO fast under heavy load.
        self.spawn_keyspace_refresh_task();

        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    info!("New connection from: {}", addr);

                    // Record connection metrics
                    self.metrics.connections.record_connection();

                    // Create executor with shared server state so that CONFIG SET,
                    // SLOWLOG config, CLIENT LIST, and loglevel changes persist
                    // across connections.
                    #[allow(unused_mut)]
                    let mut executor = CommandExecutor::with_shared(
                        self.storage.clone(),
                        self.port,
                        Arc::clone(&self.metrics),
                        Arc::clone(&self.shared_config),
                        Arc::clone(&self.slow_query_log),
                        Arc::clone(&self.clients),
                        Arc::clone(&self.current_log_level),
                    );

                    #[cfg(feature = "cluster")]
                    if let (Some(meta_raft), Some(multi_raft), Some(router)) =
                        (&self.meta_raft, &self.multi_raft, &self.router)
                    {
                        // Create ClusterCommands for this connection
                        let cluster_commands = ClusterCommands::new(
                            self.node_id,
                            Arc::clone(meta_raft),
                            Arc::clone(multi_raft),
                            Arc::clone(router),
                        );
                        executor.set_cluster_commands(cluster_commands);
                    }

                    let metrics = Arc::clone(&self.metrics);
                    let monitor_broadcaster = Arc::clone(&self.monitor_broadcaster);

                    tokio::spawn(async move {
                        let mut conn = Connection::new(
                            stream,
                            executor,
                            Some(metrics.clone()),
                            Some(monitor_broadcaster),
                        );

                        if let Err(e) = conn.handle().await {
                            error!(addr = %addr, error = %e, "connection error (read/write or protocol)");
                        }

                        // Record disconnection
                        metrics.connections.record_disconnection();
                        info!("Connection closed: {}", addr);
                    });
                }
                Err(e) => {
                    error!("Failed to accept connection: {}", e);
                }
            }
        }
    }
}
