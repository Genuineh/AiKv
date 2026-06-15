use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use parking_lot::RwLock;

use aidb::cluster::meta_types::ClusterMeta;

/// Node ID type alias.
pub type NodeId = u64;

/// 节点状态.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeStatus {
  Ok,
  PFail,
  Fail,
}

/// Gossip 节点条目.
#[derive(Debug, Clone)]
pub struct GossipNodeEntry {
  pub node_id: NodeId,
  pub addr: String,
  pub role: String,
  pub last_pong: Instant,
  pub ping_failures: u32,
  pub status: NodeStatus,
  pub config_epoch: u64,
}

/// 轻量 Gossip 状态 — 主要用于 CLUSTER NODES 的 flags/timestamps.
/// 故障检测依赖 MetaRaft Raft 心跳, Gossip 不决策 PFAIL/FAIL.
pub struct GossipState {
  nodes: Arc<RwLock<HashMap<NodeId, GossipNodeEntry>>>,
}

impl Default for GossipState {
  fn default() -> Self {
    Self::new()
  }
}

impl GossipState {
  pub fn new() -> Self {
    Self {
      nodes: Arc::new(RwLock::new(HashMap::new())),
    }
  }

  /// 从 MetaRaft ClusterMeta 刷新节点列表.
  pub fn refresh_from_meta(&self, meta: &ClusterMeta) {
    let mut nodes = self.nodes.write();
    for (nid, info) in &meta.nodes {
      let role = super::commands::cluster_node_role_label(meta, *nid).to_string();
      nodes.entry(*nid).or_insert_with(|| GossipNodeEntry {
        node_id: *nid,
        addr: info.rpc_addr.clone(),
        role,
        last_pong: Instant::now(),
        ping_failures: 0,
        status: NodeStatus::Ok,
        config_epoch: meta.version,
      });
    }
  }

  pub fn get_all_nodes(&self) -> Vec<GossipNodeEntry> {
    self.nodes.read().values().cloned().collect()
  }

  pub fn get_node(&self, node_id: NodeId) -> Option<GossipNodeEntry> {
    self.nodes.read().get(&node_id).cloned()
  }
}

/// 启动 Gossip 后台刷新循环 (从 MetaRaft 同步拓扑到 GossipState).
/// 不发送实际 PING/PONG — 故障检测委托 MetaRaft Raft 心跳.
#[tracing::instrument(name = "gossip_tick", skip_all)]
pub fn start_background_refresh(
  gossip: Arc<GossipState>,
  state: Arc<crate::cluster::state::ClusterStateManager>,
  interval_secs: u64,
) -> tokio::task::JoinHandle<()> {
  let interval = std::time::Duration::from_secs(interval_secs);
  tokio::spawn(async move {
    let mut ticker = tokio::time::interval(interval);
    loop {
      ticker.tick().await;
      let meta = state.meta_raft.get_cluster_meta();
      gossip.refresh_from_meta(&meta);
      // Refresh local leader cache so routing decisions are up-to-date.
      state.refresh();
      if let Some(ref m) = state.metrics {
        m.on_gossip_refresh(meta.nodes.len());
      }
      tracing::trace!(known_nodes = meta.nodes.len(), "gossip refresh tick");
    }
  })
}
