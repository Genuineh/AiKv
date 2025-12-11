pub mod connection;
pub mod monitor;

pub use monitor::{MonitorBroadcaster, MonitorMessage};

use self::connection::Connection;
use crate::command::CommandExecutor;
use crate::error::Result;
use crate::observability::Metrics;
use crate::storage::StorageEngine;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::{error, info};

#[cfg(feature = "cluster")]
use crate::cluster::{ClusterNode, ClusterState, MetaRaftClient, MultiRaftNode, NodeInfo};
#[cfg(feature = "cluster")]
use std::sync::RwLock;
#[cfg(feature = "cluster")]
use std::{collections::hash_map::DefaultHasher, env, hash::Hash, hash::Hasher};

#[cfg(feature = "cluster")]
#[derive(Debug, Clone)]
struct ClusterInitPlan {
    data_dir: String,
    node_id: u64,
    node_addr: String,
    cluster_port: u16,
    is_bootstrap: bool,
}

/// AiKv server
pub struct Server {
    addr: String,
    port: u16,
    storage: StorageEngine,
    metrics: Arc<Metrics>,
    monitor_broadcaster: Arc<MonitorBroadcaster>,
    #[cfg(feature = "cluster")]
    node_id: u64,
    #[cfg(feature = "cluster")]
    cluster_state: Arc<RwLock<ClusterState>>,
    #[cfg(feature = "cluster")]
    meta_raft_client: Option<Arc<MetaRaftClient>>,
    #[cfg(feature = "cluster")]
    cluster_init_plan: Option<ClusterInitPlan>,
    #[cfg(feature = "cluster")]
    cluster_node: Option<ClusterNode>,
    #[cfg(feature = "cluster")]
    cluster_multi_raft: Option<Arc<MultiRaftNode>>,
}

impl Server {
    /// Create a new server with the specified address and storage engine
    pub fn new(addr: String, storage: StorageEngine) -> Self {
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
        let (node_id, cluster_state, cluster_init_plan) = {
            let node_id = resolve_node_id(port);

            let cluster_state = Arc::new(RwLock::new(ClusterState::new()));

            // Register this node in the cluster state with an externally reachable address
            let node_addr = if addr.starts_with("0.0.0.0:") {
                format!("127.0.0.1:{}", port)
            } else {
                addr.clone()
            };

            {
                let mut state_guard = cluster_state.write().unwrap();
                let node_info = NodeInfo::new(node_id, node_addr.clone());
                state_guard.nodes.insert(node_id, node_info);
            }

            let cluster_plan = storage.aidb_data_path().map(|data_path| ClusterInitPlan {
                data_dir: data_path.to_string_lossy().into_owned(),
                node_id,
                node_addr: node_addr.clone(),
                cluster_port: port + 10000,
                is_bootstrap: env_flag_or_default("AIKV_BOOTSTRAP", false),
            });

            info!(
                "Cluster mode enabled: node_id={:040x}, port={}, bootstrap={}, data_dir_present={}",
                node_id,
                port,
                cluster_plan.as_ref().map(|p| p.is_bootstrap).unwrap_or(false),
                cluster_plan.is_some()
            );

            (node_id, cluster_state, cluster_plan)
        };

        Self {
            addr,
            port,
            storage,
            metrics: Arc::new(Metrics::new()),
            monitor_broadcaster: Arc::new(MonitorBroadcaster::new()),
            #[cfg(feature = "cluster")]
            node_id,
            #[cfg(feature = "cluster")]
            cluster_state,
            #[cfg(feature = "cluster")]
            meta_raft_client: None,
            #[cfg(feature = "cluster")]
            cluster_init_plan,
            #[cfg(feature = "cluster")]
            cluster_node: None,
            #[cfg(feature = "cluster")]
            cluster_multi_raft: None,
        }
    }

    /// Get server metrics
    pub fn metrics(&self) -> Arc<Metrics> {
        Arc::clone(&self.metrics)
    }

    /// Get monitor broadcaster
    pub fn monitor_broadcaster(&self) -> Arc<MonitorBroadcaster> {
        Arc::clone(&self.monitor_broadcaster)
    }

    /// Run the server
    pub async fn run(&mut self) -> Result<()> {
        #[cfg(feature = "cluster")]
        {
            if let Some(plan) = self.cluster_init_plan.clone() {
                self.init_cluster_meta(plan).await?;
            } else {
                info!("Cluster feature enabled but AiDb storage not detected; skipping MetaRaft wiring");
            }
        }

        let listener = TcpListener::bind(&self.addr).await?;
        info!("AiKv server listening on {}", self.addr);

        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    info!("New connection from: {}", addr);

                    // Record connection metrics
                    self.metrics.connections.record_connection();

                    #[cfg(feature = "cluster")]
                    let executor = CommandExecutor::with_shared_cluster_state_and_meta(
                        self.storage.clone(),
                        self.port,
                        self.node_id,
                        Arc::clone(&self.cluster_state),
                        self.meta_raft_client.clone(),
                        self.cluster_multi_raft.clone(),
                    );

                    #[cfg(not(feature = "cluster"))]
                    let executor = CommandExecutor::with_port(self.storage.clone(), self.port);

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
                            error!("Connection error: {}", e);
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

#[cfg(feature = "cluster")]
impl Server {
    async fn init_cluster_meta(&mut self, plan: ClusterInitPlan) -> Result<()> {
        // Initialize ClusterNode (AiDb MultiRaft + MetaRaft)
        let mut cluster_node = ClusterNode::new(plan.node_id, plan.node_addr.clone(), plan.cluster_port);

        cluster_node
            .initialize(&plan.data_dir, plan.is_bootstrap)
            .await?;

        // Bootstrap MetaRaft on the first node if requested
        if plan.is_bootstrap {
            let raft_addr = raft_addr_from(&plan.node_addr, plan.cluster_port);
            cluster_node
                .bootstrap_meta_cluster(vec![(plan.node_id, raft_addr)])
                .await
                .map_err(|e| crate::error::AikvError::Storage(format!(
                    "Failed to bootstrap MetaRaft cluster: {}",
                    e
                )))?;
        }

        // Build MetaRaftClient from the initialized node
        let multi_raft = cluster_node.inner().cloned().ok_or_else(|| {
            crate::error::AikvError::Storage("MultiRaftNode not available".to_string())
        })?;

        let raft_addr = raft_addr_from(&plan.node_addr, plan.cluster_port);
        let meta_client = Arc::new(MetaRaftClient::new(
            multi_raft.clone(),
            plan.node_id,
            plan.node_addr.clone(),
            raft_addr,
        ));

        meta_client.start_heartbeat();

        info!(
            "MetaRaft wired for node {:040x}, data_addr={}, cluster_port={}",
            plan.node_id, plan.node_addr, plan.cluster_port
        );

        self.meta_raft_client = Some(meta_client.clone());
        self.cluster_node = Some(cluster_node);
        self.cluster_multi_raft = Some(multi_raft);

        Ok(())
    }

    /// Expose MetaRaftClient for tests/diagnostics.
    pub fn meta_raft_client(&self) -> Option<Arc<MetaRaftClient>> {
        self.meta_raft_client.clone()
    }
}

#[cfg(feature = "cluster")]
fn resolve_node_id(port: u16) -> u64 {
    if let Ok(val) = env::var("AIKV_NODE_ID") {
        if let Ok(id) = u64::from_str_radix(val.trim_start_matches("0x"), 16) {
            return id;
        }
        if let Ok(id_dec) = val.parse::<u64>() {
            return id_dec;
        }
        // Fallback: hash the provided string
        let mut hasher = DefaultHasher::new();
        val.hash(&mut hasher);
        return hasher.finish();
    }

    // Deterministic fallback based on port to keep stable across restarts
    let mut hasher = DefaultHasher::new();
    port.hash(&mut hasher);
    hasher.finish()
}

#[cfg(feature = "cluster")]
fn env_flag_or_default(key: &str, default: bool) -> bool {
    match env::var(key) {
        Ok(v) => matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"),
        Err(_) => default,
    }
}

#[cfg(feature = "cluster")]
fn raft_addr_from(data_addr: &str, cluster_port: u16) -> String {
    if let Some((host, _)) = data_addr.rsplit_once(':') {
        format!("{}:{}", host, cluster_port)
    } else {
        data_addr.to_string()
    }
}
