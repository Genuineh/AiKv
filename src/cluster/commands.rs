//! Redis Cluster commands implementation using AiDb Multi-Raft API.
//!
//! This module provides a thin glue layer that maps Redis Cluster protocol
//! commands to AiDb's Multi-Raft API as documented in MULTI_RAFT_API_REFERENCE.md.
//!
//! Key principle: Minimal code - only Redis protocol format conversion.
//! All cluster logic is delegated to AiDb's MetaRaftNode, MultiRaftNode, Router, etc.
//!
//! ## MOVED Redirection
//!
//! This module implements Redis Cluster's MOVED redirection protocol. When a client
//! sends a command to the wrong node (based on the key's slot), the server returns:
//!
//! ```text
//! -MOVED <slot> <ip>:<port>
//! ```
//!
//! This tells the client which node owns the slot and where to retry the command.
//! The client should update its slot-to-node mapping and redirect future requests
//! for that slot to the correct node.

use crate::error::{AikvError, Result};
use crate::protocol::RespValue;
use bytes::Bytes;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::time::{sleep, timeout};
use tracing::{debug, info, warn};

#[cfg(feature = "cluster")]
use aidb::cluster::{
    ClusterMeta, GroupId, MetaNodeInfo, MetaRaftNode, MigrationManager, MultiRaftNode, NodeId,
    NodeStatus, Router, SlotMigration, SlotMigrationState,
};

#[cfg(feature = "cluster")]
use openraft::BasicNode;

/// Redis Cluster has 16384 slots
const TOTAL_SLOTS: u16 = 16384;

/// Extract the hash tag from a key.
///
/// Redis Cluster implements a concept called hash tags that makes it possible
/// to force certain keys to be stored in the same slot. If the key contains
/// a "{...}" pattern, only the substring between { and } is hashed.
///
/// The first occurrence of { and the first occurrence of } after it are used.
/// If the key contains {} with nothing in between, the whole key is hashed.
pub fn physical_raft_storage_key(db_index: usize, user_key: &str) -> Vec<u8> {
    if db_index == 0 {
        return user_key.as_bytes().to_vec();
    }
    let tag = extract_hash_tag(user_key.as_bytes());
    let tag_display = String::from_utf8_lossy(tag);
    format!("{{{}}}:{}:{}", tag_display, db_index, user_key).into_bytes()
}

/// Reverse [`physical_raft_storage_key`] for logical key names in `CLUSTER GETKEYSINSLOT`.
///
/// DB `0` keys are stored as the raw user key. Other DBs use `{tag}:<db>:<user_key>` (see
/// [`physical_raft_storage_key`]); if the pattern does not match, the physical bytes are returned as-is.
pub(crate) fn user_key_from_physical_raft_key(physical: &[u8]) -> Bytes {
    if !physical.starts_with(&[b'{']) {
        return Bytes::copy_from_slice(physical);
    }
    let Some(close) = physical.iter().position(|&b| b == b'}') else {
        return Bytes::copy_from_slice(physical);
    };
    let after = &physical[close + 1..];
    if !after.starts_with(&[b':']) {
        return Bytes::copy_from_slice(physical);
    }
    let after = &after[1..];
    let Some(colon) = after.iter().position(|&b| b == b':') else {
        return Bytes::copy_from_slice(physical);
    };
    let db_part = &after[..colon];
    if db_part.iter().all(|b| b.is_ascii_digit()) && db_part != b"0" {
        return Bytes::copy_from_slice(&after[colon + 1..]);
    }
    Bytes::copy_from_slice(physical)
}

fn extract_hash_tag(key: &[u8]) -> &[u8] {
    // Find the first '{'
    if let Some(start) = key.iter().position(|&b| b == b'{') {
        // Find the first '}' after '{'
        if let Some(end) = key[start + 1..].iter().position(|&b| b == b'}') {
            // Check if there's content between { and }
            if end > 0 {
                return &key[start + 1..start + 1 + end];
            }
        }
    }
    // No hash tag, return the whole key
    key
}

/// Calculate the slot for a key, respecting hash tags (same as `Router::key_to_slot`).
pub fn key_to_slot_with_hash_tag(key: &[u8]) -> u16 {
    Router::key_to_slot(key)
}

/// Failover mode for CLUSTER FAILOVER command
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailoverMode {
    /// Default failover - wait for master agreement
    Default,
    /// Force failover without master agreement
    Force,
    /// Takeover - force failover even if master is unreachable
    Takeover,
}

/// Redirection type for cluster routing
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedirectType {
    /// -MOVED redirect: key belongs to another node
    Moved,
    /// -ASK redirect: key is being migrated to another node
    Ask,
}

/// Node information for CLUSTER NODES response.
/// Maps from AiDb's MetaNodeInfo to Redis format.
#[derive(Debug, Clone)]
pub struct NodeInfo {
    pub id: NodeId,
    pub addr: String,
    pub cluster_port: u16,
    pub is_master: bool,
    pub is_connected: bool,
    pub master_id: Option<NodeId>,
    pub replica_ids: Vec<NodeId>,
}

impl NodeInfo {
    /// Create from AiDb's MetaNodeInfo
    pub fn from_meta_node_info(id: NodeId, info: &MetaNodeInfo) -> Self {
        let cluster_port = Self::extract_cluster_port(&info.addr);
        Self {
            id,
            addr: info.addr.clone(),
            cluster_port,
            is_master: true, // Will be updated based on group info
            is_connected: matches!(info.status, NodeStatus::Online),
            master_id: None,
            replica_ids: Vec::new(),
        }
    }

    fn extract_cluster_port(addr: &str) -> u16 {
        if let Some(port_str) = addr.split(':').nth_back(0) {
            port_str.parse::<u16>().unwrap_or(6379) + 10000
        } else {
            16379
        }
    }
}

/// Redis Cluster commands handler.
///
/// This is a thin wrapper around AiDb's Multi-Raft components:
/// - MetaRaftNode: For cluster metadata management
/// - MultiRaftNode: For data operations with automatic routing
/// - Router: For key-to-slot-to-group routing
/// - MigrationManager: For slot migration (optional)
#[cfg(feature = "cluster")]
pub struct ClusterCommands {
    /// This node's ID
    node_id: NodeId,

    /// Reference to MetaRaftNode for cluster metadata
    meta_raft: Arc<MetaRaftNode>,

    /// Reference to MultiRaftNode for data operations
    multi_raft: Arc<MultiRaftNode>,

    /// Router for key-to-slot-to-group mapping
    #[allow(dead_code)]
    router: Arc<Router>,

    /// Optional hook for AiDb's [`MigrationManager`] (single-process / shared `ShardedStateMachine`).
    ///
    /// Not wired in distributed AiKv: each node only holds local Raft groups, so cross-node slot
    /// moves rely on `CLUSTER GETKEYSINSLOT` + `MIGRATE` (or external tooling), not this type.
    migration_manager: Option<Arc<MigrationManager>>,

    /// Automatic failover debounce/cooldown state keyed by group.
    auto_failover_state: Mutex<HashMap<GroupId, AutoFailoverState>>,
}

#[cfg(feature = "cluster")]
#[derive(Debug, Clone)]
struct AutoFailoverState {
    last_leader_id: Option<NodeId>,
    consecutive_offline_checks: u32,
    first_offline_at: Instant,
    last_trigger_at: Option<Instant>,
}

#[cfg(feature = "cluster")]
impl ClusterCommands {
    /// Create a new ClusterCommands handler.
    ///
    /// # Arguments
    ///
    /// * `node_id` - This node's unique identifier
    /// * `meta_raft` - MetaRaftNode for cluster metadata
    /// * `multi_raft` - MultiRaftNode for data operations
    /// * `router` - Router for key routing
    pub fn new(
        node_id: NodeId,
        meta_raft: Arc<MetaRaftNode>,
        multi_raft: Arc<MultiRaftNode>,
        router: Arc<Router>,
    ) -> Self {
        Self {
            node_id,
            meta_raft,
            multi_raft,
            router,
            migration_manager: None,
            auto_failover_state: Mutex::new(HashMap::new()),
        }
    }

    fn auto_failover_required_consecutive_checks() -> u32 {
        std::env::var("AIKV_AUTO_FAILOVER_REQUIRED_CHECKS")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(3)
    }

    fn auto_failover_min_offline_duration() -> Duration {
        std::env::var("AIKV_AUTO_FAILOVER_MIN_OFFLINE_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .map(Duration::from_secs)
            .unwrap_or(Duration::from_secs(10))
    }

    fn auto_failover_cooldown_duration() -> Duration {
        std::env::var("AIKV_AUTO_FAILOVER_COOLDOWN_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .map(Duration::from_secs)
            .unwrap_or(Duration::from_secs(30))
    }

    fn normalize_addr_for_tcp(addr: &str) -> String {
        addr.trim_start_matches("http://")
            .trim_start_matches("https://")
            .to_string()
    }

    async fn is_addr_reachable(addr: &str, timeout_ms: u64) -> bool {
        let target = Self::normalize_addr_for_tcp(addr);
        matches!(
            timeout(Duration::from_millis(timeout_ms), TcpStream::connect(&target)).await,
            Ok(Ok(_))
        )
    }

    /// Set the migration manager (optional)
    pub fn set_migration_manager(&mut self, manager: Arc<MigrationManager>) {
        self.migration_manager = Some(manager);
    }

    fn find_group_by_node(meta: &ClusterMeta, node_id: NodeId) -> Option<GroupId> {
        // Prefer leader-owned group first: for masters this avoids picking an
        // unrelated replica membership when metadata is temporarily inconsistent.
        let leader_group = meta
            .groups
            .iter()
            .find_map(|(gid, g)| (g.leader == Some(node_id)).then_some(*gid));
        if leader_group.is_some() {
            return leader_group;
        }

        let replica_groups: Vec<GroupId> = meta
            .groups
            .iter()
            .filter_map(|(gid, g)| g.replicas.contains(&node_id).then_some(*gid))
            .collect();
        match replica_groups.len() {
            0 => None,
            1 => Some(replica_groups[0]),
            _ => {
                warn!(
                    diag_event = "cluster_group_lookup_ambiguous",
                    node_id = %format!("{:040x}", node_id),
                    groups = ?replica_groups,
                    "Node appears in multiple replica groups; refusing ambiguous mapping"
                );
                None
            }
        }
    }

    /// 节点已在 `CLUSTER MEET` 元数据里，但可能尚无分片组（扩容空 master、未 ADDSLOTS）。
    /// SETSLOT MIGRATING/IMPORTING/NODE 需要 `to_group`，与 `ADDSLOTS` 一样按需 `create_group`。
    async fn ensure_shard_group_for_node(&self, node_id: NodeId) -> Result<GroupId> {
        let meta = self.meta_raft.get_cluster_meta();
        if let Some(gid) = Self::find_group_by_node(&meta, node_id) {
            return Ok(gid);
        }
        if !meta.nodes.contains_key(&node_id) {
            return Err(AikvError::Invalid(format!(
                "Unknown target node: {:040x}",
                node_id
            )));
        }
        let group_id = node_id;
        let created = match self.meta_raft.create_group(group_id, vec![node_id]).await {
            Ok(_) => true,
            Err(e) => {
                let msg = format!("{}", e);
                if msg.contains("already exists") {
                    false
                } else {
                    return Err(AikvError::Internal(format!(
                        "Failed to create shard group for node {:040x}: {}",
                        node_id, e
                    )));
                }
            }
        };
        if created {
            self.meta_raft
                .update_group_leader(group_id, node_id)
                .await
                .map_err(|e| {
                    AikvError::Internal(format!(
                        "Failed to set group leader for new shard {:040x}: {}",
                        node_id, e
                    ))
                })?;
        }
        self.schedule_post_meta_sync();
        let meta = self.meta_raft.get_cluster_meta();
        Self::find_group_by_node(&meta, node_id).ok_or_else(|| {
            AikvError::Internal(format!(
                "Shard group for node {:040x} missing after ensure",
                node_id
            ))
        })
    }

    fn leader_addr_for_group(meta: &ClusterMeta, group_id: GroupId) -> Option<String> {
        let leader_id = meta.groups.get(&group_id)?.leader?;
        let leader_info = meta.nodes.get(&leader_id)?;
        Some(Self::extract_data_address(&leader_info.addr))
    }

    fn active_slot_migration(meta: &ClusterMeta, slot: u16) -> Option<SlotMigration> {
        meta.migrations
            .iter()
            .find(|m| m.slot == slot && !m.is_complete())
            .cloned()
    }

    /// Runs local data-group + router refresh after MetaRaft metadata has committed.
    ///
    /// Used both inline (failover / SETSLOT) and from a background task (provisioning).
    async fn post_meta_sync_for_multi(
        multi: &Arc<MultiRaftNode>,
        requester_hex: &str,
    ) -> Result<()> {
        let t0 = Instant::now();
        info!(
            diag_event = "cluster_meta_post_sync_start",
            requester_node_id = %requester_hex,
            "Starting data-group/router sync after meta change"
        );
        // Unit tests and partial setups may use MultiRaftNode without `init_router()`.
        // Local data groups + Router refresh belong to full server startup only.
        if multi.router().is_none() {
            info!(
                diag_event = "cluster_meta_post_sync_success",
                requester_node_id = %requester_hex,
                sync_mode = "skip_no_router",
                duration_ms = t0.elapsed().as_millis() as u64,
                "Skipped post-meta sync because router is not initialized"
            );
            return Ok(());
        }
        multi
            .sync_data_groups_from_meta()
            .await
            .map_err(|e| {
                warn!(
                    diag_event = "cluster_meta_post_sync_failed",
                    requester_node_id = %requester_hex,
                    stage = "sync_data_groups_from_meta",
                    duration_ms = t0.elapsed().as_millis() as u64,
                    error = %e,
                    "Post-meta sync failed"
                );
                AikvError::Internal(format!("Failed to sync local Raft data groups: {}", e))
            })?;
        if let Some(r) = multi.router() {
            r.refresh_metadata().map_err(|e| {
                warn!(
                    diag_event = "cluster_meta_post_sync_failed",
                    requester_node_id = %requester_hex,
                    stage = "router_refresh_metadata",
                    duration_ms = t0.elapsed().as_millis() as u64,
                    error = %e,
                    "Router refresh failed after meta sync"
                );
                AikvError::Internal(format!("Router metadata refresh failed: {}", e))
            })?;
        }
        info!(
            diag_event = "cluster_meta_post_sync_success",
            requester_node_id = %requester_hex,
            sync_mode = "full",
            duration_ms = t0.elapsed().as_millis() as u64,
            "Post-meta sync finished"
        );
        Ok(())
    }

    /// Create local OpenRaft data-group instances from ClusterMeta (failover, SETSLOT, …).
    async fn sync_data_raft_groups_after_meta_change(&self) -> Result<()> {
        Self::post_meta_sync_for_multi(&self.multi_raft, &format!("{:040x}", self.node_id)).await
    }

    /// Wait for the Data Raft group to elect a leader after a metadata-level failover.
    ///
    /// Raft best practice: after updating `GroupMeta.leader` in MetaRaft, we must
    /// wait for the Data Raft group's actual leader election to complete before
    /// reporting success to the client. Otherwise the client gets OK but the
    /// promoted node may not yet be the Data Raft leader, causing write failures.
    ///
    /// Returns `true` if the promoted node became leader within the timeout.
    async fn wait_for_data_raft_leader(
        multi_raft: &MultiRaftNode,
        group_id: GroupId,
        promoted_node_id: NodeId,
        timeout: Duration,
    ) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if let Some(raft) = multi_raft.get_raft_group(group_id) {
                let metrics = raft.metrics().borrow().clone();
                match metrics.current_leader {
                    Some(leader) if leader == promoted_node_id => return true,
                    Some(leader) => {
                        // Raft elected a different node — log but accept it,
                        // the quorum may have converged on another replica.
                        info!(
                            "Data Raft group {} elected leader {:040x} (expected {:040x}), accepting",
                            group_id, leader, promoted_node_id
                        );
                        return true;
                    }
                    None => {} // No leader yet, keep waiting
                }
            }
            sleep(Duration::from_millis(200)).await;
        }
        warn!(
            "Data Raft group {} did not elect a leader within {:?} after failover",
            group_id, timeout
        );
        false
    }

    /// Wait until the local metadata's config_version reaches at least the target.
    async fn wait_for_meta_version(
        meta_raft: &MetaRaftNode,
        target_version: u64,
        timeout: Duration,
    ) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let meta = meta_raft.get_cluster_meta();
            if meta.config_version >= target_version {
                return true;
            }
            sleep(Duration::from_millis(100)).await;
        }
        warn!(
            "Local metadata config_version did not reach {} within {:?} (current: {})",
            target_version,
            timeout,
            meta_raft.get_cluster_meta().config_version,
        );
        false
    }

    /// Same as [`Self::sync_data_raft_groups_after_meta_change`] but does not block the caller.
    ///
    /// MetaRaft changes are already durable when this runs; local data-group construction
    /// (OpenRaft, RocksDB) can panic or be slow — running it on the request task caused
    /// redis-cli to see "Server closed the connection" during CLUSTER ADDSLOTSRANGE.
    fn schedule_post_meta_sync(&self) {
        let multi = Arc::clone(&self.multi_raft);
        let requester_hex = format!("{:040x}", self.node_id);
        tokio::spawn(async move {
            if let Err(e) = Self::post_meta_sync_for_multi(&multi, &requester_hex).await {
                debug!(
                    diag_event = "cluster_meta_post_sync_background_done_err",
                    requester_node_id = %requester_hex,
                    error = %e,
                    "Background post-meta sync finished with error (details were logged above)"
                );
            }
        });
    }

    /// Wait until local meta view can observe slot importing state for this node.
    async fn wait_importing_visibility(&self, slot: u16, timeout_ms: u64) -> bool {
        let begin = Instant::now();
        let deadline = begin + Duration::from_millis(timeout_ms);
        let mut attempts: u64 = 0;
        loop {
            attempts += 1;
            let meta = self.meta_raft.get_cluster_meta();
            let local_group = Self::find_group_by_node(&meta, self.node_id);
            let visible = if let (Some(group), Some(migration)) =
                (local_group, Self::active_slot_migration(&meta, slot))
            {
                matches!(
                    migration.state,
                    SlotMigrationState::Importing { to_group, .. }
                        | SlotMigrationState::Migrating { to_group, .. } if to_group == group
                )
            } else {
                false
            };

            if visible {
                debug!(
                    diag_event = "cluster_setslot_importing_visibility_wait_ok",
                    requester_node_id = %format!("{:040x}", self.node_id),
                    slot = slot,
                    wait_ms = begin.elapsed().as_millis() as u64,
                    attempts = attempts,
                    "Observed importing migration in local meta"
                );
                return true;
            }

            if Instant::now() >= deadline {
                warn!(
                    diag_event = "cluster_setslot_importing_visibility_wait_timeout",
                    requester_node_id = %format!("{:040x}", self.node_id),
                    slot = slot,
                    wait_ms = begin.elapsed().as_millis() as u64,
                    attempts = attempts,
                    "Importing visibility wait timed out; continue without blocking"
                );
                return false;
            }

            sleep(Duration::from_millis(20)).await;
        }
    }

    /// Handle CLUSTER INFO command.
    ///
    /// Maps to: `meta_raft.get_cluster_meta()`
    pub fn cluster_info(&self) -> Result<RespValue> {
        let meta: ClusterMeta = self.meta_raft.get_cluster_meta();

        // Count assigned slots
        let assigned_slots = meta.slots.iter().filter(|&&g| g > 0).count();

        // Count online nodes
        let known_nodes = meta.nodes.len();
        #[allow(unused)]
        let online_nodes = meta
            .nodes
            .values()
            .filter(|n| matches!(n.status, NodeStatus::Online))
            .count();

        // Determine cluster state
        // Cluster is OK if all slots are assigned and all groups with slots have leaders
        let all_groups_have_leaders = meta.groups.iter().all(|(gid, g)| {
            // Check if this group owns any slots
            let owns_slots = meta.slots.contains(gid);
            // If it owns slots, it must have a leader
            !owns_slots || g.leader.is_some()
        });

        // Every non-zero slot must point at a group that exists in `meta.groups` (orphan slot
        // mappings make routing fail while naive slot counts can still look "full").
        let slot_maps_to_known_group = meta
            .slots
            .iter()
            .all(|&g| g == 0 || meta.groups.contains_key(&g));

        let cluster_state = if assigned_slots == TOTAL_SLOTS as usize
            && all_groups_have_leaders
            && slot_maps_to_known_group
        {
            "ok"
        } else {
            "fail"
        };

        info!(
            diag_event = "cluster_info_snapshot",
            node_id = %format!("{:040x}", self.node_id),
            config_version = meta.config_version,
            known_nodes = known_nodes,
            groups = meta.groups.len(),
            slots_assigned = assigned_slots,
            cluster_state = cluster_state,
            "CLUSTER INFO snapshot"
        );

        let info = format!(
            "cluster_state:{}\r\n\
             cluster_slots_assigned:{}\r\n\
             cluster_slots_ok:{}\r\n\
             cluster_slots_pfail:0\r\n\
             cluster_slots_fail:0\r\n\
             cluster_known_nodes:{}\r\n\
             cluster_size:{}\r\n\
             cluster_current_epoch:{}\r\n\
             cluster_my_epoch:{}\r\n\
             cluster_stats_messages_sent:0\r\n\
             cluster_stats_messages_received:0",
            cluster_state,
            assigned_slots,
            assigned_slots,
            known_nodes,
            meta.groups.len(),
            meta.config_version,
            meta.config_version,
        );

        Ok(RespValue::BulkString(Some(Bytes::from(info))))
    }

    /// Handle CLUSTER NODES command.
    ///
    /// Maps to: `meta_raft.get_cluster_meta().nodes` and `.groups`
    pub fn cluster_nodes(&self) -> Result<RespValue> {
        let meta: ClusterMeta = self.meta_raft.get_cluster_meta();
        let mut lines = Vec::new();

        info!(
            diag_event = "cluster_nodes_snapshot",
            node_id = %format!("{:040x}", self.node_id),
            config_version = meta.config_version,
            known_nodes = meta.nodes.len(),
            groups = meta.groups.len(),
            "CLUSTER NODES snapshot start"
        );

        for (node_id, node_info) in &meta.nodes {
            // For Redis cluster compatibility, report all nodes as "connected"
            // since they are registered in the cluster metadata and reachable.
            // TODO: Implement proper health checking to determine actual node status
            let status = match node_info.status {
                NodeStatus::Online => "connected",
                NodeStatus::Offline => "disconnected",
                // Treat Joining and other states as connected for Redis compatibility
                _ => "connected",
            };

            // Redis: 「master」= 分片主或尚未持槽的空主；「slave」= 挂在某 master 下的副本。
            // 仅把「在某组 replicas 里且不是该组 leader」的节点标为 slave。
            // 旧逻辑把「还未 ADDSLOTS、因而不是任何组 leader」的新节点标成 slave，会误显示扩容 master。
            let is_replica = meta.groups.values().any(|g| {
                if !g.replicas.contains(node_id) {
                    return false;
                }
                match g.leader {
                    Some(leader) => leader != *node_id,
                    None => false,
                }
            });
            let is_master = !is_replica;
            let role = if is_master { "master" } else { "slave" };

            // Find the master node ID if this is a replica
            let master_id = if is_master {
                "-".to_string()
            } else {
                // Find which group this replica belongs to and get its leader
                meta.groups
                    .values()
                    .find(|g| {
                        g.replicas.contains(node_id)
                            && g.leader.is_some()
                            && g.leader != Some(*node_id)
                    })
                    .and_then(|g| g.leader)
                    .map(|lid| format!("{:040x}", lid))
                    .unwrap_or_else(|| "-".to_string())
            };

            // Only masters have slot ranges in CLUSTER NODES output
            let mut slot_ranges = Vec::new();
            let mut migration_flags = Vec::new();
            if is_master {
                for (group_id, group_meta) in &meta.groups {
                    if group_meta.leader == Some(*node_id) {
                        // Find slot range for this group
                        let mut start = None;
                        let mut end = None;
                        for (slot_idx, &assigned_group) in meta.slots.iter().enumerate() {
                            if assigned_group == *group_id {
                                if start.is_none() {
                                    start = Some(slot_idx);
                                }
                                end = Some(slot_idx);
                            } else if start.is_some() {
                                slot_ranges.push(format!("{}-{}", start.unwrap(), end.unwrap()));
                                start = None;
                                end = None;
                            }
                        }
                        if let Some(s) = start {
                            slot_ranges.push(format!("{}-{}", s, end.unwrap()));
                        }

                        for m in &meta.migrations {
                            if m.is_complete() {
                                continue;
                            }
                            match m.state {
                                SlotMigrationState::Migrating { from_group, to_group }
                                | SlotMigrationState::Importing { from_group, to_group } => {
                                    if *group_id == from_group {
                                        if let Some(to_leader) =
                                            meta.groups.get(&to_group).and_then(|g| g.leader)
                                        {
                                            migration_flags.push(format!(
                                                "[{}->-{:040x}]",
                                                m.slot, to_leader
                                            ));
                                        }
                                    }
                                    if *group_id == to_group {
                                        if let Some(from_leader) =
                                            meta.groups.get(&from_group).and_then(|g| g.leader)
                                        {
                                            migration_flags.push(format!(
                                                "[{}-<-{:040x}]",
                                                m.slot, from_leader
                                            ));
                                        }
                                    }
                                }
                                SlotMigrationState::Idle | SlotMigrationState::Complete => {}
                            }
                        }
                    }
                }
            }

            // Format address properly: ip:data_port@cluster_bus_port
            // node_info.addr is like "aikv1:50051" (raft address), we need to convert to data port
            let data_addr = Self::extract_data_address(&node_info.addr);
            let cluster_port = Self::extract_cluster_port_from_data_port(&data_addr);

            // Format: <id> <ip:port@cport> <flags> <master> <ping-sent> <pong-recv> <config-epoch> <link-state> <slot> <slot> ...
            let myself_flag = if *node_id == self.node_id {
                "myself,"
            } else {
                ""
            };
            let slots_and_flags = if migration_flags.is_empty() {
                slot_ranges.join(" ")
            } else if slot_ranges.is_empty() {
                migration_flags.join(" ")
            } else {
                format!("{} {}", slot_ranges.join(" "), migration_flags.join(" "))
            };
            let node_line = format!(
                "{:040x} {}@{} {}{} {} 0 0 {} {} {}",
                node_id,
                data_addr,
                cluster_port,
                myself_flag,
                role,
                master_id,
                meta.config_version,
                status,
                slots_and_flags
            );

            lines.push(node_line);
        }

        let result = lines.join("\r\n");
        debug!(
            diag_event = "cluster_nodes_snapshot_end",
            node_id = %format!("{:040x}", self.node_id),
            rows = lines.len(),
            bytes = result.len(),
            "CLUSTER NODES snapshot end"
        );
        Ok(RespValue::BulkString(Some(Bytes::from(result))))
    }

    /// Handle CLUSTER SLOTS command.
    ///
    /// Maps to: `meta_raft.get_cluster_meta().slots` and `.groups`
    pub fn cluster_slots(&self) -> Result<RespValue> {
        let meta: ClusterMeta = self.meta_raft.get_cluster_meta();
        let mut slots_info = Vec::new();

        // Group consecutive slots by group_id
        let mut current_group: Option<GroupId> = None;
        let mut range_start: u16 = 0;

        for (slot, &group_id) in meta.slots.iter().enumerate() {
            if group_id == 0 {
                // Unassigned slot
                if current_group.is_some() {
                    if let Some(group) = current_group {
                        slots_info.push(self.format_slot_range(
                            &meta,
                            range_start,
                            (slot - 1) as u16,
                            group,
                        ));
                    }
                    current_group = None;
                }
                continue;
            }

            match current_group {
                None => {
                    // Start new range
                    current_group = Some(group_id);
                    range_start = slot as u16;
                }
                Some(cg) if cg != group_id => {
                    // Different group, output previous range and start new one
                    slots_info.push(self.format_slot_range(
                        &meta,
                        range_start,
                        (slot - 1) as u16,
                        cg,
                    ));
                    current_group = Some(group_id);
                    range_start = slot as u16;
                }
                _ => {
                    // Same group, continue range
                }
            }
        }

        // Output last range if any
        if let Some(group) = current_group {
            slots_info.push(self.format_slot_range(&meta, range_start, TOTAL_SLOTS - 1, group));
        }

        let assigned_slots = meta.slots.iter().filter(|&&g| g > 0).count();
        info!(
            diag_event = "cluster_slots_snapshot",
            node_id = %format!("{:040x}", self.node_id),
            config_version = meta.config_version,
            known_nodes = meta.nodes.len(),
            groups = meta.groups.len(),
            slot_ranges = slots_info.len(),
            slots_assigned = assigned_slots,
            "CLUSTER SLOTS snapshot"
        );

        Ok(RespValue::Array(Some(slots_info)))
    }

    /// Format a slot range for CLUSTER SLOTS response
    fn format_slot_range(
        &self,
        meta: &ClusterMeta,
        start: u16,
        end: u16,
        group_id: GroupId,
    ) -> RespValue {
        let mut elements = vec![
            RespValue::Integer(start as i64),
            RespValue::Integer(end as i64),
        ];

        if let Some(group_meta) = meta.groups.get(&group_id) {
            // Add master node first
            if let Some(leader_id) = group_meta.leader {
                if let Some(node_info) = meta.nodes.get(&leader_id) {
                    elements.push(self.format_node_info(leader_id, node_info));
                }
            }

            // Add replica nodes
            for &replica_id in &group_meta.replicas {
                if Some(replica_id) != group_meta.leader {
                    if let Some(node_info) = meta.nodes.get(&replica_id) {
                        elements.push(self.format_node_info(replica_id, node_info));
                    }
                }
            }

            // Redis-compatible hint during migration: include importing master
            // in slot response so clients can warm target-node cache.
            if start == end {
                if let Some(m) = Self::active_slot_migration(meta, start) {
                    if let SlotMigrationState::Migrating { from_group, to_group }
                    | SlotMigrationState::Importing { from_group, to_group } = m.state
                    {
                        if group_id == from_group {
                            if let Some(target_group) = meta.groups.get(&to_group) {
                                if let Some(target_leader) = target_group.leader {
                                    if let Some(node_info) = meta.nodes.get(&target_leader) {
                                        elements.push(self.format_node_info(target_leader, node_info));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        RespValue::Array(Some(elements))
    }

    /// Format node info for CLUSTER SLOTS response
    fn format_node_info(&self, node_id: NodeId, node_info: &MetaNodeInfo) -> RespValue {
        // Convert Raft address to data address
        let data_addr = Self::extract_data_address(&node_info.addr);
        let (ip, port) = Self::parse_addr(&data_addr);
        RespValue::Array(Some(vec![
            RespValue::BulkString(Some(Bytes::from(ip))),
            RespValue::Integer(port),
            RespValue::BulkString(Some(Bytes::from(format!("{:040x}", node_id)))),
        ]))
    }

    /// Parse address into (ip, port)
    fn parse_addr(addr: &str) -> (String, i64) {
        if let Some((ip, port_str)) = addr.rsplit_once(':') {
            let port = port_str.parse::<i64>().unwrap_or(6379);
            (ip.to_string(), port)
        } else {
            (addr.to_string(), 6379)
        }
    }

    /// Extract cluster port from address string
    #[allow(dead_code)]
    fn extract_cluster_port(addr: &str) -> u16 {
        if let Some(port_str) = addr.split(':').nth_back(0) {
            port_str.parse::<u16>().unwrap_or(6379) + 10000
        } else {
            16379
        }
    }

    /// Extract data address from node address
    /// Handles two formats:
    /// - Data format: "127.0.0.1:6380" -> returns as is
    /// - Raft format: "aikv-master-1:50051" -> same host + Redis port `6379 + (raft_port - 50051)`
    ///   (so peer containers resolve the leader via Docker DNS / hostname).
    ///
    /// When the host part is empty or `0.0.0.0`, falls back to `AIKV_ADVERTISE_HOST` or
    /// `127.0.0.1`.
    fn extract_data_address(addr: &str) -> String {
        let addr = addr.trim();
        let addr = addr
            .strip_prefix("http://")
            .or_else(|| addr.strip_prefix("https://"))
            .unwrap_or(addr);

        let advertise_host = std::env::var("AIKV_ADVERTISE_HOST")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| "127.0.0.1".to_string());

        if let Some(port_str) = addr.split(':').nth_back(0) {
            if let Ok(port) = port_str.parse::<u16>() {
                // Raft listen port: 50051+k -> Redis 6379+k（与 docker-compose-cluster / expand 宿主机 gRPC 映射一致）
                // 保持窄范围，避免把无关的高端口误判为 Raft（与改动前行为一致，仅多支持 50057 第三分片）
                if (50051..=50057).contains(&port) {
                    let data_port = 6379 + (port - 50051);
                    let host = addr
                        .rsplit_once(':')
                        .map(|(h, _)| h)
                        .filter(|h| {
                            !h.is_empty() && *h != "0.0.0.0" && *h != "127.0.0.1" && *h != "localhost"
                        })
                        .unwrap_or(advertise_host.as_str());
                    return format!("{}:{}", host, data_port);
                }
                // 已是 Redis 数据端口（compose 默认 6379-6384）
                if (6379..=6384).contains(&port) {
                    let host = addr
                        .rsplit_once(':')
                        .map(|(h, _)| h)
                        .filter(|h| {
                            !h.is_empty() && *h != "0.0.0.0" && *h != "127.0.0.1" && *h != "localhost"
                        })
                        .unwrap_or(advertise_host.as_str());
                    return format!("{}:{}", host, port);
                }
            }
        }
        // Fallback: host:port as-is (helps when addr is already a Redis endpoint).
        addr.to_string()
    }

    /// Extract cluster bus port from data port
    fn extract_cluster_port_from_data_port(data_addr: &str) -> u16 {
        if let Some(port_str) = data_addr.split(':').nth_back(0) {
            port_str.parse::<u16>().unwrap_or(6379) + 10000
        } else {
            16379
        }
    }

    /// Handle CLUSTER MYID command.
    ///
    /// Maps to: node_id
    pub fn cluster_myid(&self) -> Result<RespValue> {
        Ok(RespValue::BulkString(Some(Bytes::from(format!(
            "{:040x}",
            self.node_id
        )))))
    }

    /// Handle CLUSTER KEYSLOT command.
    ///
    /// Uses hash tag extraction for proper Redis Cluster compatibility.
    pub fn cluster_keyslot(&self, key: &[u8]) -> Result<RespValue> {
        let slot = key_to_slot_with_hash_tag(key);
        Ok(RespValue::Integer(slot as i64))
    }

    /// Handle CLUSTER MEET command.
    ///
    /// Maps to: `meta_raft.add_node(node_id, addr)`
    ///
    /// # Arguments
    ///
    /// * `ip` - IP address of the node to add
    /// * `port` - Port of the node to add
    /// * `node_id_opt` - Optional pre-assigned node ID
    pub async fn cluster_meet(
        &self,
        ip: String,
        port: u16,
        node_id_opt: Option<NodeId>,
    ) -> Result<RespValue> {
        let addr = format!("{}:{}", ip, port);

        // Generate node ID if not provided
        let node_id = node_id_opt.unwrap_or_else(|| {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut hasher = DefaultHasher::new();
            addr.hash(&mut hasher);
            hasher.finish()
        });

        // Add node to cluster metadata via MetaRaft (must be proposed on MetaRaft leader)
        let add_res = self.meta_raft.add_node(node_id, addr.clone()).await;
        if add_res.is_ok() {
            return Ok(RespValue::SimpleString("OK".to_string()));
        }
        let e = add_res.unwrap_err();
        let err_msg = e.to_string();
        if err_msg.to_ascii_lowercase().contains("forwardtoleader") {
            let mut leader_redis_addr =
                Self::extract_forward_leader_addr_from_error(&err_msg).unwrap_or_default();
            if leader_redis_addr.is_empty() {
                let meta_live = self.meta_raft.get_cluster_meta();
                leader_redis_addr = if let Some(meta_leader_id) = self.meta_raft.get_leader().await {
                    if let Some(meta_leader) = meta_live.nodes.get(&meta_leader_id) {
                        Self::extract_data_address(&meta_leader.addr)
                    } else if let Some(addr) = self.meta_raft.get_member_address(meta_leader_id) {
                        Self::extract_data_address(&addr)
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                };
            }
            if !leader_redis_addr.is_empty() {
                self.forward_meet_to_leader(&leader_redis_addr, &ip, port, node_id_opt)
                    .await?;
                return Ok(RespValue::SimpleString("OK".to_string()));
            }
        }

        Err(AikvError::Internal(format!(
            "Failed to add node to cluster: {}",
            e
        )))
    }

    /// Handle CLUSTER FORGET command.
    ///
    /// Maps to: `meta_raft.remove_node(node_id)`
    pub async fn cluster_forget(&self, node_id: NodeId) -> Result<RespValue> {
        // Remove node via MetaRaft - this will sync to all nodes via Raft consensus
        self.meta_raft
            .remove_node(node_id)
            .await
            .map_err(|e| AikvError::Internal(format!("Failed to remove node: {}", e)))?;

        self.schedule_post_meta_sync();

        Ok(RespValue::SimpleString("OK".to_string()))
    }

    /// Handle CLUSTER ADDSLOTS command.
    ///
    /// Maps to: `meta_raft.update_slots(start, end, group_id)`
    ///
    /// Note: For Redis compatibility, we need to assign slots to a group.
    /// The group_id is determined by finding which group this node belongs to.
    /// If the node doesn't belong to any group yet, we create one automatically.
    pub async fn cluster_addslots(&self, slots: Vec<u16>) -> Result<RespValue> {
        let meta = self.meta_raft.get_cluster_meta();

        // Find the group that this node belongs to, or create one if it doesn't exist
        let group_id = if let Some(gid) = Self::find_group_by_node(&meta, self.node_id) {
            gid
        } else {
            // Auto-create a group for this node using its node_id as the group_id
            // This matches Redis behavior where each master initially forms its own group
            let group_id = self.node_id;
            self.meta_raft
                .create_group(group_id, vec![self.node_id])
                .await
                .map_err(|e| {
                    AikvError::Internal(format!("Failed to create group for node: {}", e))
                })?;
            group_id
        };

        // Validate all slots first
        for &slot in &slots {
            if slot >= TOTAL_SLOTS {
                return Err(AikvError::Invalid(format!("Invalid slot: {}", slot)));
            }
        }

        // Optimize: merge consecutive slots into ranges for batch updates
        if slots.is_empty() {
            return Ok(RespValue::SimpleString("OK".to_string()));
        }

        let mut sorted_slots = slots.clone();
        sorted_slots.sort_unstable();

        // Group consecutive slots into ranges
        let mut ranges: Vec<(u16, u16)> = Vec::new();
        let mut range_start = sorted_slots[0];
        let mut range_end = sorted_slots[0];

        for &slot in &sorted_slots[1..] {
            if slot == range_end + 1 {
                range_end = slot;
            } else {
                ranges.push((range_start, range_end + 1)); // end is exclusive
                range_start = slot;
                range_end = slot;
            }
        }
        ranges.push((range_start, range_end + 1));

        // Apply each range in a single update
        for (start, end) in ranges {
            self.meta_raft
                .update_slots(start, end, group_id)
                .await
                .map_err(|e| {
                    AikvError::Internal(format!(
                        "Failed to assign slots {}-{}: {}",
                        start,
                        end - 1,
                        e
                    ))
                })?;
        }

        self.schedule_post_meta_sync();

        Ok(RespValue::SimpleString("OK".to_string()))
    }

    /// Handle CLUSTER ADDSLOTSRANGE command.
    ///
    /// Maps to: `meta_raft.update_slots(start, end, group_id)` and `meta_raft.create_group`
    /// This is more efficient than ADDSLOTS for large ranges as it uses a single Raft proposal.
    ///
    /// # Arguments
    /// * `start` - Start slot (inclusive)
    /// * `end` - End slot (inclusive)
    /// * `target_node_id` - Node to assign slots to (0 means current node)
    pub async fn cluster_addslotsrange(
        &self,
        start: u16,
        end: u16,
        target_node_id: NodeId,
    ) -> Result<RespValue> {
        let meta = self.meta_raft.get_cluster_meta();

        // Determine the actual node_id to use
        let node_id = if target_node_id == 0 {
            self.node_id
        } else {
            // Verify the target node exists
            if !meta.nodes.contains_key(&target_node_id) {
                return Err(AikvError::Invalid(format!(
                    "Target node {:040x} not found in cluster",
                    target_node_id
                )));
            }
            target_node_id
        };

        // Validate range
        if start > end || end >= TOTAL_SLOTS {
            return Err(AikvError::Invalid(format!(
                "Invalid slot range: {}-{}",
                start, end
            )));
        }

        let update_res: std::result::Result<(), AikvError> = async {
            // Find or create group for the target node
            let group_id = if let Some((gid, _)) = meta
                .groups
                .iter()
                .find(|(_, g)| g.replicas.contains(&node_id))
            {
                *gid
            } else {
                // Create a group for this node using its node_id as the group_id
                let group_id = node_id;
                self.meta_raft
                    .create_group(group_id, vec![node_id])
                    .await
                    .map_err(|e| {
                        AikvError::Internal(format!("Failed to create group for node: {}", e))
                    })?;
                // Set the node as the leader of this group
                self.meta_raft
                    .update_group_leader(group_id, node_id)
                    .await
                    .map_err(|e| {
                        AikvError::Internal(format!("Failed to set group leader: {}", e))
                    })?;
                group_id
            };

            // Assign the entire range in a single update (end is exclusive in update_slots)
            self.meta_raft
                .update_slots(start, end + 1, group_id)
                .await
                .map_err(|e| {
                    AikvError::Internal(format!("Failed to assign slots {}-{}: {}", start, end, e))
                })?;
            Ok(())
        }
        .await;

        if let Err(e) = update_res {
            let err_msg = e.to_string();
            if err_msg.to_ascii_lowercase().contains("forwardtoleader") {
                let mut leader_redis_addr =
                    Self::extract_forward_leader_addr_from_error(&err_msg).unwrap_or_default();
                if leader_redis_addr.is_empty() {
                    let meta_live = self.meta_raft.get_cluster_meta();
                    leader_redis_addr =
                        if let Some(meta_leader_id) = self.meta_raft.get_leader().await {
                            if let Some(meta_leader) = meta_live.nodes.get(&meta_leader_id) {
                                Self::extract_data_address(&meta_leader.addr)
                            } else if let Some(addr) = self.meta_raft.get_member_address(meta_leader_id)
                            {
                                Self::extract_data_address(&addr)
                            } else {
                                String::new()
                            }
                        } else {
                            String::new()
                        };
                }
                if !leader_redis_addr.is_empty() {
                    self.forward_addslotsrange_to_leader(
                        &leader_redis_addr,
                        start,
                        end,
                        target_node_id,
                    )
                    .await?;
                    self.schedule_post_meta_sync();
                    return Ok(RespValue::SimpleString("OK".to_string()));
                }
            }
            return Err(e);
        }

        self.schedule_post_meta_sync();

        Ok(RespValue::SimpleString("OK".to_string()))
    }

    /// Handle CLUSTER DELSLOTS command.
    ///
    /// Maps to: `meta_raft.update_slots(start, end, 0)` where 0 means unassigned
    pub async fn cluster_delslots(&self, slots: Vec<u16>) -> Result<RespValue> {
        // Delete slots via MetaRaft - sync to all nodes via Raft consensus
        for slot in slots {
            if slot >= TOTAL_SLOTS {
                return Err(AikvError::Invalid(format!("Invalid slot: {}", slot)));
            }

            self.meta_raft
                .update_slots(slot, slot + 1, 0)
                .await
                .map_err(|e| {
                    AikvError::Internal(format!("Failed to delete slot {}: {}", slot, e))
                })?;
        }

        self.schedule_post_meta_sync();

        Ok(RespValue::SimpleString("OK".to_string()))
    }

    /// Handle CLUSTER REPLICATE command.
    ///
    /// Sets this node as a replica of the specified master node.
    /// Maps to: `meta_raft.update_group_members(group_id, new_replicas)`
    pub async fn cluster_replicate(&self, master_id: NodeId) -> Result<RespValue> {
        let meta = self.meta_raft.get_cluster_meta();

        // Find the group that the master belongs to
        // Groups are created with group_id == node_id, so we check:
        // 1. group_id matches master_id directly
        // 2. leader field matches master_id
        // 3. replicas list contains master_id
        let group_id = meta
            .groups
            .iter()
            .find(|(gid, g)| {
                **gid == master_id || g.leader == Some(master_id) || g.replicas.contains(&master_id)
            })
            .map(|(gid, _)| *gid)
            .ok_or_else(|| {
                AikvError::Internal(format!(
                    "Master node {:040x} does not belong to any group",
                    master_id
                ))
            })?;

        // Get current group members
        let group = meta
            .groups
            .get(&group_id)
            .ok_or_else(|| AikvError::Internal(format!("Group {} not found", group_id)))?;

        // Add this node to the group's replicas if not already present
        let mut new_replicas = group.replicas.clone();
        if !new_replicas.contains(&self.node_id) {
            new_replicas.push(self.node_id);

            // Update group membership via MetaRaft
            self.meta_raft
                .update_group_members(group_id, new_replicas)
                .await
                .map_err(|e| {
                    AikvError::Internal(format!(
                        "Failed to add replica to group {}: {}",
                        group_id, e
                    ))
                })?;
        }

        self.schedule_post_meta_sync();

        Ok(RespValue::SimpleString("OK".to_string()))
    }

    /// Handle CLUSTER ADDREPLICATION command.
    ///
    /// Adds a replica to a master's group. This command is sent to the MetaRaft leader
    /// and specifies both the replica and master node IDs, allowing it to work even when
    /// the replica node doesn't have the latest ClusterMeta.
    ///
    /// # Arguments
    /// * `replica_id` - Node ID of the replica to add
    /// * `master_id` - Node ID of the master to replicate
    pub async fn cluster_add_replication(
        &self,
        replica_id: NodeId,
        master_id: NodeId,
    ) -> Result<RespValue> {
        let meta = self.meta_raft.get_cluster_meta();

        // Find the group that the master belongs to
        // Groups are created with group_id == node_id, so we check:
        // 1. group_id matches master_id directly
        // 2. leader field matches master_id
        // 3. replicas list contains master_id
        let group_id = meta
            .groups
            .iter()
            .find(|(gid, g)| {
                **gid == master_id || g.leader == Some(master_id) || g.replicas.contains(&master_id)
            })
            .map(|(gid, _)| *gid)
            .ok_or_else(|| {
                AikvError::Internal(format!(
                    "Master node {:040x} does not belong to any group",
                    master_id
                ))
            })?;

        // Get current group members
        let group = meta
            .groups
            .get(&group_id)
            .ok_or_else(|| AikvError::Internal(format!("Group {} not found", group_id)))?;

        // Add the replica to the group's replicas if not already present
        let mut new_replicas = group.replicas.clone();
        if !new_replicas.contains(&replica_id) {
            new_replicas.push(replica_id);

            // Update group membership via MetaRaft
            self.meta_raft
                .update_group_members(group_id, new_replicas)
                .await
                .map_err(|e| {
                    AikvError::Internal(format!(
                        "Failed to add replica {:040x} to group {}: {}",
                        replica_id, group_id, e
                    ))
                })?;
        }

        self.schedule_post_meta_sync();

        Ok(RespValue::SimpleString("OK".to_string()))
    }

    /// Group that owns `slot` in meta, only if this node participates in that data group.
    fn local_group_for_slot_keys(&self, slot: u16) -> Result<Option<GroupId>> {
        let meta = self.meta_raft.get_cluster_meta();
        let group_id = *meta
            .slots
            .get(slot as usize)
            .ok_or_else(|| AikvError::Invalid(format!("Invalid slot: {}", slot)))?;
        if group_id == 0 {
            return Ok(None);
        }
        let member = meta.groups.get(&group_id).is_some_and(|g| {
            g.leader == Some(self.node_id) || g.replicas.contains(&self.node_id)
        });
        if !member {
            return Err(AikvError::Invalid(format!(
                "Hash slot {} is not served by this node",
                slot
            )));
        }
        Ok(Some(group_id))
    }

    /// Handle CLUSTER GETKEYSINSLOT command.
    ///
    /// Lists keys stored locally for `slot` via [`ShardedStateMachine::scan_slot_keys_sync`].
    /// At most `count` keys are returned (Redis does not guarantee which keys if the slot is larger).
    pub fn cluster_getkeysinslot(&self, slot: u16, count: usize) -> Result<RespValue> {
        if slot >= TOTAL_SLOTS {
            return Err(AikvError::Invalid(format!("Invalid slot: {}", slot)));
        }
        if count == 0 {
            return Ok(RespValue::Array(Some(vec![])));
        }

        let Some(group_id) = self.local_group_for_slot_keys(slot)? else {
            return Ok(RespValue::Array(Some(vec![])));
        };

        let keys = self
            .multi_raft
            .scan_local_group_slot_keys_sync(group_id, slot)
            .map_err(|e| AikvError::Internal(format!("GETKEYSINSLOT scan failed: {}", e)))?;

        let out: Vec<RespValue> = keys
            .into_iter()
            .take(count)
            .map(|k| RespValue::BulkString(Some(user_key_from_physical_raft_key(&k))))
            .collect();

        Ok(RespValue::Array(Some(out)))
    }

    /// Handle CLUSTER COUNTKEYSINSLOT command.
    pub fn cluster_countkeysinslot(&self, slot: u16) -> Result<RespValue> {
        if slot >= TOTAL_SLOTS {
            return Err(AikvError::Invalid(format!("Invalid slot: {}", slot)));
        }

        let Some(group_id) = self.local_group_for_slot_keys(slot)? else {
            return Ok(RespValue::Integer(0));
        };

        let keys = self
            .multi_raft
            .scan_local_group_slot_keys_sync(group_id, slot)
            .map_err(|e| AikvError::Internal(format!("COUNTKEYSINSLOT scan failed: {}", e)))?;

        Ok(RespValue::Integer(keys.len() as i64))
    }

    /// Handle CLUSTER SHARDS command (Redis 7.0+).
    ///
    /// Returns the mapping of cluster slots to shards in Redis 7.0+ format.
    /// This command is used by modern Redis clients (like RedisInsight) to detect cluster mode.
    ///
    /// Maps to: `meta_raft.get_cluster_meta()`
    pub fn cluster_shards(&self) -> Result<RespValue> {
        let meta: ClusterMeta = self.meta_raft.get_cluster_meta();
        let mut shards = Vec::new();

        // Build shard info for each group that has slots
        for (group_id, group_meta) in &meta.groups {
            // Find slots assigned to this group
            let mut slot_ranges: Vec<(u16, u16)> = Vec::new();
            let mut start: Option<u16> = None;
            let mut end: Option<u16> = None;

            for (slot_idx, &assigned_group) in meta.slots.iter().enumerate() {
                if assigned_group == *group_id {
                    if start.is_none() {
                        start = Some(slot_idx as u16);
                    }
                    end = Some(slot_idx as u16);
                } else if start.is_some() {
                    slot_ranges.push((start.unwrap(), end.unwrap()));
                    start = None;
                    end = None;
                }
            }
            if let Some(s) = start {
                slot_ranges.push((s, end.unwrap()));
            }

            // Skip groups without slots
            if slot_ranges.is_empty() {
                continue;
            }

            // Build slots array for this shard
            let mut slots_array = Vec::new();
            for (range_start, range_end) in &slot_ranges {
                slots_array.push(RespValue::Array(Some(vec![
                    RespValue::Integer(*range_start as i64),
                    RespValue::Integer(*range_end as i64),
                ])));
            }

            // Build nodes array for this shard
            let mut nodes_array = Vec::new();

            // Add master node first (leader)
            if let Some(leader_id) = group_meta.leader {
                if let Some(node_info) = meta.nodes.get(&leader_id) {
                    let data_addr = Self::extract_data_address(&node_info.addr);
                    let (ip, port) = Self::parse_addr(&data_addr);
                    let health = match node_info.status {
                        NodeStatus::Online => "online",
                        NodeStatus::Offline => "offline",
                        _ => "loading",
                    };

                    nodes_array.push(RespValue::Array(Some(vec![
                        RespValue::BulkString(Some(Bytes::from("id"))),
                        RespValue::BulkString(Some(Bytes::from(format!("{:040x}", leader_id)))),
                        RespValue::BulkString(Some(Bytes::from("port"))),
                        RespValue::Integer(port),
                        RespValue::BulkString(Some(Bytes::from("ip"))),
                        RespValue::BulkString(Some(Bytes::from(ip.clone()))),
                        RespValue::BulkString(Some(Bytes::from("endpoint"))),
                        RespValue::BulkString(Some(Bytes::from(ip.clone()))),
                        RespValue::BulkString(Some(Bytes::from("role"))),
                        RespValue::BulkString(Some(Bytes::from("master"))),
                        RespValue::BulkString(Some(Bytes::from("replication-offset"))),
                        RespValue::Integer(0),
                        RespValue::BulkString(Some(Bytes::from("health"))),
                        RespValue::BulkString(Some(Bytes::from(health))),
                    ])));
                }
            }

            // Add replica nodes
            for &replica_id in &group_meta.replicas {
                // Skip leader (already added as master)
                if Some(replica_id) == group_meta.leader {
                    continue;
                }
                if let Some(node_info) = meta.nodes.get(&replica_id) {
                    let data_addr = Self::extract_data_address(&node_info.addr);
                    let (ip, port) = Self::parse_addr(&data_addr);
                    let health = match node_info.status {
                        NodeStatus::Online => "online",
                        NodeStatus::Offline => "offline",
                        _ => "loading",
                    };

                    nodes_array.push(RespValue::Array(Some(vec![
                        RespValue::BulkString(Some(Bytes::from("id"))),
                        RespValue::BulkString(Some(Bytes::from(format!("{:040x}", replica_id)))),
                        RespValue::BulkString(Some(Bytes::from("port"))),
                        RespValue::Integer(port),
                        RespValue::BulkString(Some(Bytes::from("ip"))),
                        RespValue::BulkString(Some(Bytes::from(ip.clone()))),
                        RespValue::BulkString(Some(Bytes::from("endpoint"))),
                        RespValue::BulkString(Some(Bytes::from(ip.clone()))),
                        RespValue::BulkString(Some(Bytes::from("role"))),
                        RespValue::BulkString(Some(Bytes::from("replica"))),
                        RespValue::BulkString(Some(Bytes::from("replication-offset"))),
                        RespValue::Integer(0),
                        RespValue::BulkString(Some(Bytes::from("health"))),
                        RespValue::BulkString(Some(Bytes::from(health))),
                    ])));
                }
            }

            // Build shard entry
            shards.push(RespValue::Array(Some(vec![
                RespValue::BulkString(Some(Bytes::from("slots"))),
                RespValue::Array(Some(slots_array)),
                RespValue::BulkString(Some(Bytes::from("nodes"))),
                RespValue::Array(Some(nodes_array)),
            ])));
        }

        Ok(RespValue::Array(Some(shards)))
    }

    /// Handle CLUSTER MYSHARDID command.
    ///
    /// Returns the shard ID that this node belongs to.
    pub fn cluster_myshardid(&self) -> Result<RespValue> {
        let meta: ClusterMeta = self.meta_raft.get_cluster_meta();

        // Find which group this node belongs to
        for (group_id, group_meta) in &meta.groups {
            if group_meta.leader == Some(self.node_id)
                || group_meta.replicas.contains(&self.node_id)
            {
                return Ok(RespValue::BulkString(Some(Bytes::from(format!(
                    "{:040x}",
                    group_id
                )))));
            }
        }

        // Node not assigned to any shard yet - return node_id as shard id
        Ok(RespValue::BulkString(Some(Bytes::from(format!(
            "{:040x}",
            self.node_id
        )))))
    }

    /// Handle CLUSTER SET-CONFIG-EPOCH command.
    ///
    /// Sets the configuration epoch for this node.
    pub fn cluster_set_config_epoch(&self, _epoch: u64) -> Result<RespValue> {
        // In our implementation, config epoch is managed by MetaRaft
        // This command is used during cluster creation to set initial epochs
        // For now, just return OK as the epoch is managed internally
        Ok(RespValue::SimpleString("OK".to_string()))
    }

    /// Handle CLUSTER REPLICAS command.
    ///
    /// Returns a list of replica nodes for the given master node.
    pub fn cluster_replicas(&self, master_id: NodeId) -> Result<RespValue> {
        let meta: ClusterMeta = self.meta_raft.get_cluster_meta();
        let mut replicas = Vec::new();

        // Find the group where this node is leader
        for group_meta in meta.groups.values() {
            if group_meta.leader == Some(master_id) {
                // Found the group, list all replicas (excluding the leader)
                for &replica_id in &group_meta.replicas {
                    if replica_id == master_id {
                        continue;
                    }
                    if let Some(node_info) = meta.nodes.get(&replica_id) {
                        // For Redis cluster compatibility, report nodes as "connected"
                        // TODO: Implement proper health checking
                        let status = match node_info.status {
                            NodeStatus::Online => "connected",
                            NodeStatus::Offline => "disconnected",
                            _ => "connected",
                        };
                        let data_addr = Self::extract_data_address(&node_info.addr);
                        let cluster_port = Self::extract_cluster_port_from_data_port(&data_addr);

                        // Format: <id> <ip:port@cport> slave <master-id> <ping-sent> <pong-recv> <config-epoch> <link-state>
                        let line = format!(
                            "{:040x} {}@{} slave {:040x} 0 0 {} {}",
                            replica_id,
                            data_addr,
                            cluster_port,
                            master_id,
                            meta.config_version,
                            status
                        );
                        replicas.push(RespValue::BulkString(Some(Bytes::from(line))));
                    }
                }
                break;
            }
        }

        // Also check if node_id might be a hex string
        if replicas.is_empty() {
            // The master might not be a leader of any group (could be a replica itself)
            return Err(AikvError::Invalid(format!(
                "Node {:040x} is not a master or does not exist",
                master_id
            )));
        }

        Ok(RespValue::Array(Some(replicas)))
    }

    /// Handle CLUSTER SAVECONFIG command.
    ///
    /// Forces the node to save cluster configuration to disk.
    pub fn cluster_saveconfig(&self) -> Result<RespValue> {
        // In our implementation, cluster config is persisted via Raft log
        // This is essentially a no-op since Raft handles persistence
        Ok(RespValue::SimpleString("OK".to_string()))
    }

    fn failover_mode_name(mode: FailoverMode) -> &'static str {
        match mode {
            FailoverMode::Default => "",
            FailoverMode::Force => "FORCE",
            FailoverMode::Takeover => "TAKEOVER",
        }
    }

    fn node_status_name(status: NodeStatus) -> &'static str {
        match status {
            NodeStatus::Online => "ONLINE",
            NodeStatus::Offline => "OFFLINE",
            NodeStatus::Joining => "JOINING",
            NodeStatus::Leaving => "LEAVING",
        }
    }

    fn trim_error_for_log(err: &str) -> String {
        const MAX: usize = 512;
        if err.len() <= MAX {
            return err.to_string();
        }
        format!("{}...(truncated, total={} chars)", &err[..MAX], err.len())
    }

    fn is_loopback_redis_addr(addr: &str) -> bool {
        let a = addr.trim().to_ascii_lowercase();
        if a.is_empty() {
            return false;
        }
        let host = a.split(':').next().unwrap_or("");
        host == "127.0.0.1" || host == "localhost" || host == "::1"
    }

    fn extract_forward_leader_addr_from_error(err_msg: &str) -> Option<String> {
        // OpenRaft wraps: `APIError(ForwardToLeader(ForwardToLeader { ... leader_node: Some(BasicNode { addr: "..." }) }))`
        // Scan from the *last* `ForwardToLeader` so nested Debug still finds the inner `addr`.
        if let Some(fidx) = err_msg.rfind("ForwardToLeader") {
            let tail = &err_msg[fidx..];
            // Debug uses double quotes around the address string.
            if let Some(pos) = tail.find("addr: \"") {
                let rest = &tail[pos + "addr: \"".len()..];
                if let Some(end) = rest.find('"') {
                    let addr = rest[..end].trim();
                    if !addr.is_empty() {
                        return Some(Self::extract_data_address(addr));
                    }
                }
            }
        }
        // OpenRaft `Debug` (~8.1): `ForwardToLeader { leader_node: Some(BasicNode { addr: "..." }) }`
        // Prefer explicit `leader_node` / `BasicNode` markers so we do not grab an unrelated `addr:`.
        const MARKERS: &[&str] = &[
            "leader_node: Some(BasicNode { addr: \"",
            "BasicNode { addr: \"",
        ];
        for marker in MARKERS {
            if let Some(start) = err_msg.rfind(marker) {
                let rest = &err_msg[start + marker.len()..];
                if let Some(end) = rest.find('"') {
                    let addr = &rest[..end];
                    if !addr.is_empty() {
                        return Some(Self::extract_data_address(addr));
                    }
                }
            }
        }
        let marker = "addr: \"";
        let start = err_msg.rfind(marker)? + marker.len();
        let rest = &err_msg[start..];
        let end = rest.find('"')?;
        let addr = &rest[..end];
        if addr.is_empty() {
            return None;
        }
        Some(Self::extract_data_address(addr))
    }

    async fn forward_failover_to_leader(
        &self,
        leader_redis_addr: &str,
        mode: FailoverMode,
        promoted_node_id: NodeId,
    ) -> Result<()> {
        let client = redis::Client::open(format!("redis://{}/", leader_redis_addr)).map_err(|e| {
            AikvError::Internal(format!(
                "Failed to create redis client for MetaRaft leader {}: {}",
                leader_redis_addr, e
            ))
        })?;

        let mut conn = client.get_multiplexed_async_connection().await.map_err(|e| {
            AikvError::Internal(format!(
                "Failed to connect to MetaRaft leader {}: {}",
                leader_redis_addr, e
            ))
        })?;

        let mut cmd = redis::cmd("CLUSTER");
        cmd.arg("FAILOVER");
        let mode_name = Self::failover_mode_name(mode);
        if !mode_name.is_empty() {
            cmd.arg(mode_name);
        }
        cmd.arg(format!("{:040x}", promoted_node_id));

        let _: String = cmd.query_async(&mut conn).await.map_err(|e| {
            AikvError::Internal(format!(
                "MetaRaft leader {} failed to execute CLUSTER FAILOVER for {:040x}: {}",
                leader_redis_addr, promoted_node_id, e
            ))
        })?;

        Ok(())
    }

    async fn forward_meet_to_leader(
        &self,
        leader_redis_addr: &str,
        ip: &str,
        port: u16,
        node_id_opt: Option<NodeId>,
    ) -> Result<()> {
        let client = redis::Client::open(format!("redis://{}/", leader_redis_addr)).map_err(|e| {
            AikvError::Internal(format!(
                "Failed to create redis client for MetaRaft leader {}: {}",
                leader_redis_addr, e
            ))
        })?;
        let mut conn = client.get_multiplexed_async_connection().await.map_err(|e| {
            AikvError::Internal(format!(
                "Failed to connect to MetaRaft leader {}: {}",
                leader_redis_addr, e
            ))
        })?;
        let mut cmd = redis::cmd("CLUSTER");
        cmd.arg("MEET").arg(ip).arg(port);
        if let Some(id) = node_id_opt {
            cmd.arg(format!("{:040x}", id));
        }
        let _: String = cmd.query_async(&mut conn).await.map_err(|e| {
            AikvError::Internal(format!(
                "MetaRaft leader {} failed to execute CLUSTER MEET {}:{}: {}",
                leader_redis_addr, ip, port, e
            ))
        })?;
        Ok(())
    }

    async fn forward_metaraft_setstatus_to_leader(
        &self,
        leader_redis_addr: &str,
        node_id: NodeId,
        status: NodeStatus,
    ) -> Result<()> {
        // Guard: avoid forwarding loops caused by loopback leader addresses
        // inside containers (e.g. 127.0.0.1 points back to self).
        if Self::is_loopback_redis_addr(leader_redis_addr) {
            return Err(AikvError::Internal(format!(
                "Refusing to forward CLUSTER METARAFT SETSTATUS to loopback address {}",
                leader_redis_addr
            )));
        }
        let client = redis::Client::open(format!("redis://{}/", leader_redis_addr)).map_err(|e| {
            AikvError::Internal(format!(
                "Failed to create redis client for MetaRaft leader {}: {}",
                leader_redis_addr, e
            ))
        })?;

        let mut conn = client.get_multiplexed_async_connection().await.map_err(|e| {
            AikvError::Internal(format!(
                "Failed to connect to MetaRaft leader {}: {}",
                leader_redis_addr, e
            ))
        })?;

        let mut cmd = redis::cmd("CLUSTER");
        cmd.arg("METARAFT")
            .arg("SETSTATUS")
            .arg(format!("{:040x}", node_id))
            .arg(Self::node_status_name(status))
            // Single-hop forwarding marker: prevent re-forward recursion loops.
            .arg("__FORWARDED__");

        let _: String = cmd.query_async(&mut conn).await.map_err(|e| {
            AikvError::Internal(format!(
                "MetaRaft leader {} failed to execute CLUSTER METARAFT SETSTATUS for {:040x}: {}",
                leader_redis_addr, node_id, e
            ))
        })?;
        Ok(())
    }

    async fn forward_setslot_to_leader(
        &self,
        leader_redis_addr: &str,
        slot: u16,
        mode: &str,
        node_id: Option<NodeId>,
        requester_node_id: Option<NodeId>,
    ) -> Result<()> {
        let t0 = Instant::now();
        info!(
            diag_event = "cluster_setslot_forward_rpc_attempt",
            requester_node_id = %format!("{:040x}", self.node_id),
            slot = slot,
            mode = %mode,
            forward_redis_addr = %leader_redis_addr,
            target_node_id = %node_id.map(|n| format!("{:040x}", n)).unwrap_or_else(|| "-".to_string()),
            "Forwarding CLUSTER SETSLOT to MetaRaft leader"
        );
        let client = redis::Client::open(format!("redis://{}/", leader_redis_addr)).map_err(|e| {
            AikvError::Internal(format!(
                "Failed to create redis client for MetaRaft leader {}: {}",
                leader_redis_addr, e
            ))
        })?;
        let mut conn = client.get_multiplexed_async_connection().await.map_err(|e| {
            AikvError::Internal(format!(
                "Failed to connect to MetaRaft leader {}: {}",
                leader_redis_addr, e
            ))
        })?;
        let mut cmd = redis::cmd("CLUSTER");
        cmd.arg("SETSLOT").arg(slot).arg(mode);
        if let Some(id) = node_id {
            cmd.arg(format!("{:040x}", id));
        }
        if let Some(requester) = requester_node_id {
            cmd.arg(format!("{:040x}", requester));
        }
        let _: String = cmd.query_async(&mut conn).await.map_err(|e| {
            warn!(
                diag_event = "cluster_setslot_forward_rpc_failed",
                requester_node_id = %format!("{:040x}", self.node_id),
                slot = slot,
                mode = %mode,
                forward_redis_addr = %leader_redis_addr,
                duration_ms = t0.elapsed().as_millis() as u64,
                error = %e,
                "Forwarded CLUSTER SETSLOT failed on MetaRaft leader"
            );
            AikvError::Internal(format!(
                "MetaRaft leader {} failed to execute CLUSTER SETSLOT {} {}: {}",
                leader_redis_addr, slot, mode, e
            ))
        })?;
        info!(
            diag_event = "cluster_setslot_forward_rpc_success",
            requester_node_id = %format!("{:040x}", self.node_id),
            slot = slot,
            mode = %mode,
            forward_redis_addr = %leader_redis_addr,
            duration_ms = t0.elapsed().as_millis() as u64,
            "Forwarded CLUSTER SETSLOT succeeded"
        );
        Ok(())
    }

    async fn forward_addslotsrange_to_leader(
        &self,
        leader_redis_addr: &str,
        start: u16,
        end: u16,
        target_node_id: NodeId,
    ) -> Result<()> {
        let client = redis::Client::open(format!("redis://{}/", leader_redis_addr)).map_err(|e| {
            AikvError::Internal(format!(
                "Failed to create redis client for MetaRaft leader {}: {}",
                leader_redis_addr, e
            ))
        })?;
        let mut conn = client.get_multiplexed_async_connection().await.map_err(|e| {
            AikvError::Internal(format!(
                "Failed to connect to MetaRaft leader {}: {}",
                leader_redis_addr, e
            ))
        })?;
        let mut cmd = redis::cmd("CLUSTER");
        cmd.arg("ADDSLOTSRANGE").arg(start).arg(end);
        if target_node_id != 0 {
            cmd.arg(format!("{:040x}", target_node_id));
        }
        let _: String = cmd.query_async(&mut conn).await.map_err(|e| {
            AikvError::Internal(format!(
                "MetaRaft leader {} failed to execute CLUSTER ADDSLOTSRANGE {} {} {}: {}",
                leader_redis_addr,
                start,
                end,
                if target_node_id == 0 {
                    "-".to_string()
                } else {
                    format!("{:040x}", target_node_id)
                },
                e
            ))
        })?;
        Ok(())
    }

    async fn update_node_status_with_forward(
        &self,
        meta: &ClusterMeta,
        node_id: NodeId,
        status: NodeStatus,
        is_forwarded: bool,
    ) -> Result<()> {
        info!(
            diag_event = "metaraft_setstatus_attempt",
            requester_node_id = %format!("{:040x}", self.node_id),
            target_node_id = %format!("{:040x}", node_id),
            target_status = Self::node_status_name(status),
            "Attempting node status update through MetaRaft"
        );
        let res = self.meta_raft.update_node_status(node_id, status).await;
        if let Err(e) = res {
            let err_msg = e.to_string();
            if err_msg.contains("ForwardToLeader") {
                if is_forwarded {
                    warn!(
                        diag_event = "metaraft_setstatus_forward_blocked",
                        requester_node_id = %format!("{:040x}", self.node_id),
                        target_node_id = %format!("{:040x}", node_id),
                        target_status = Self::node_status_name(status),
                        error = %Self::trim_error_for_log(&err_msg),
                        "SETSTATUS already forwarded once; refuse recursive forwarding"
                    );
                    return Err(AikvError::Internal(format!(
                        "SETSTATUS forwarding loop detected for {:040x}",
                        node_id
                    )));
                }
                if let Some(meta_leader_id) = self.meta_raft.get_leader().await {
                    if meta_leader_id != self.node_id {
                        if let Some(leader_node) = meta.nodes.get(&meta_leader_id) {
                            let leader_redis_addr = Self::extract_data_address(&leader_node.addr);
                            info!(
                                "CLUSTER METARAFT SETSTATUS: forwarding request to MetaRaft leader {} at {} (redis={})",
                                meta_leader_id, leader_node.addr, leader_redis_addr
                            );
                            self.forward_metaraft_setstatus_to_leader(
                                &leader_redis_addr,
                                node_id,
                                status,
                            )
                            .await?;
                            info!(
                                diag_event = "metaraft_setstatus_forward_success",
                                requester_node_id = %format!("{:040x}", self.node_id),
                                target_node_id = %format!("{:040x}", node_id),
                                target_status = Self::node_status_name(status),
                                forward_mode = "leader_lookup",
                                forward_leader_id = %format!("{:040x}", meta_leader_id),
                                forward_redis_addr = %leader_redis_addr,
                                "Node status update succeeded after forwarding to MetaRaft leader"
                            );
                            return Ok(());
                        }
                    }
                }
                if let Some(leader_redis_addr) =
                    Self::extract_forward_leader_addr_from_error(&err_msg)
                {
                    info!(
                        "CLUSTER METARAFT SETSTATUS: fallback forwarding via ForwardToLeader addr (redis={})",
                        leader_redis_addr
                    );
                    self.forward_metaraft_setstatus_to_leader(&leader_redis_addr, node_id, status)
                        .await?;
                    info!(
                        diag_event = "metaraft_setstatus_forward_success",
                        requester_node_id = %format!("{:040x}", self.node_id),
                        target_node_id = %format!("{:040x}", node_id),
                        target_status = Self::node_status_name(status),
                        forward_mode = "error_hint",
                        forward_redis_addr = %leader_redis_addr,
                        "Node status update succeeded after fallback forwarding"
                    );
                    return Ok(());
                }
            }
            warn!(
                diag_event = "metaraft_setstatus_failed",
                requester_node_id = %format!("{:040x}", self.node_id),
                target_node_id = %format!("{:040x}", node_id),
                target_status = Self::node_status_name(status),
                error = %Self::trim_error_for_log(&e.to_string()),
                "Node status update failed and was not recoverable by forwarding"
            );
            return Err(AikvError::Internal(format!(
                "Failed to update node status for {:040x}: {}",
                node_id, e
            )));
        }
        info!(
            diag_event = "metaraft_setstatus_success",
            requester_node_id = %format!("{:040x}", self.node_id),
            target_node_id = %format!("{:040x}", node_id),
            target_status = Self::node_status_name(status),
            mode = "local",
            "Node status update succeeded locally"
        );
        Ok(())
    }

    /// Handle CLUSTER FAILOVER command.
    ///
    /// Triggers a manual failover (replica becomes master).
    /// `target_node_id` is optional and intended for internal forwarding to MetaRaft leader.
    pub async fn cluster_failover(
        &self,
        mode: FailoverMode,
        target_node_id: Option<NodeId>,
    ) -> Result<RespValue> {
        let meta: ClusterMeta = self.meta_raft.get_cluster_meta();
        let promoted_node_id = target_node_id.unwrap_or(self.node_id);

        // Log voter/learner status before failover attempt
        let raft = self.meta_raft.raft();
        let metrics = raft.metrics().borrow().clone();
        let membership = metrics.membership_config.membership();
        let voters: Vec<_> = membership.voter_ids().collect();
        let learners: Vec<_> = membership.learner_ids().collect();
        info!(
            "CLUSTER FAILOVER: requester_node_id={}, promoted_node_id={}, voters={:?}, learners={:?}, mode={:?}",
            self.node_id, promoted_node_id, voters, learners, mode
        );

        // Check if this node is a voter
        let is_voter = membership.voter_ids().any(|id| id == self.node_id);
        if !is_voter {
            warn!(
                "CLUSTER FAILOVER: node {} is a learner, not a voter - cannot become leader!",
                self.node_id
            );
        }

        // Find which group this node is a replica of
        let group_id = meta
            .groups
            .iter()
            .find(|(_, g)| {
                g.replicas.contains(&promoted_node_id) && g.leader != Some(promoted_node_id)
            })
            .map(|(gid, _)| *gid);

        let group_id = match group_id {
            Some(id) => id,
            None => {
                return Err(AikvError::Invalid(
                    "This node is not a replica or already a master".to_string(),
                ));
            }
        };

        // Perform failover based on mode
        let update_res = match mode {
            FailoverMode::Default | FailoverMode::Force | FailoverMode::Takeover => {
                self.meta_raft.update_group_leader(group_id, promoted_node_id).await
            }
        };

        if let Err(e) = update_res {
            // OpenRaft returns ForwardToLeader when requester is not MetaRaft leader.
            // For manual failover we transparently retry by sending the same command
            // to MetaRaft leader's Redis endpoint, keeping CLI behavior simple.
            let err_msg = e.to_string();
            let looks_like_forward = err_msg.contains("ForwardToLeader");
            if looks_like_forward {
                if let Some(meta_leader_id) = self.meta_raft.get_leader().await {
                    if meta_leader_id != self.node_id {
                        if let Some(leader_node) = meta.nodes.get(&meta_leader_id) {
                            let leader_redis_addr = Self::extract_data_address(&leader_node.addr);
                            info!(
                                "CLUSTER FAILOVER: forwarding request to MetaRaft leader {} at {} (redis={})",
                                meta_leader_id, leader_node.addr, leader_redis_addr
                            );
                            self.forward_failover_to_leader(
                                &leader_redis_addr,
                                mode,
                                promoted_node_id,
                            )
                            .await?;
                            info!(
                                diag_event = "cluster_failover_post_sync_start",
                                requester_node_id = %format!("{:040x}", self.node_id),
                                promoted_node_id = %format!("{:040x}", promoted_node_id),
                                group_id = group_id,
                                mode = ?mode,
                                sync_reason = "after_forward_success",
                                "Starting local data-group sync after forwarded failover"
                            );
                            self.sync_data_raft_groups_after_meta_change()
                                .await
                                .map_err(|e| {
                                    warn!(
                                        diag_event = "cluster_failover_post_sync_failed",
                                        requester_node_id = %format!("{:040x}", self.node_id),
                                        promoted_node_id = %format!("{:040x}", promoted_node_id),
                                        group_id = group_id,
                                        mode = ?mode,
                                        sync_reason = "after_forward_success",
                                        error = %e,
                                        "Data-group sync failed after forwarded failover"
                                    );
                                    e
                                })?;
                            info!(
                                diag_event = "cluster_failover_post_sync_success",
                                requester_node_id = %format!("{:040x}", self.node_id),
                                promoted_node_id = %format!("{:040x}", promoted_node_id),
                                group_id = group_id,
                                mode = ?mode,
                                sync_reason = "after_forward_success",
                                "Data-group sync completed after forwarded failover"
                            );
                            info!(
                                diag_event = "cluster_failover_forward_success",
                                requester_node_id = %format!("{:040x}", self.node_id),
                                promoted_node_id = %format!("{:040x}", promoted_node_id),
                                group_id = group_id,
                                mode = ?mode,
                                forward_mode = "leader_lookup",
                                forward_leader_id = %format!("{:040x}", meta_leader_id),
                                forward_redis_addr = %leader_redis_addr,
                                "Failover proposal succeeded after forwarding to MetaRaft leader"
                            );

                            // Raft best practice: wait for the promoted node to actually
                            // become the Data Raft leader before returning OK.
                            Self::wait_for_meta_version(
                                &self.meta_raft,
                                meta.config_version + 1,
                                Duration::from_secs(5),
                            ).await;
                            Self::wait_for_data_raft_leader(
                                &self.multi_raft,
                                group_id,
                                promoted_node_id,
                                Duration::from_secs(10),
                            ).await;

                            return Ok(RespValue::SimpleString("OK".to_string()));
                        }
                    }
                }
                if let Some(leader_redis_addr) =
                    Self::extract_forward_leader_addr_from_error(&err_msg)
                {
                    info!(
                        "CLUSTER FAILOVER: fallback forwarding via ForwardToLeader addr (redis={})",
                        leader_redis_addr
                    );
                    self.forward_failover_to_leader(&leader_redis_addr, mode, promoted_node_id)
                        .await?;
                    info!(
                        diag_event = "cluster_failover_post_sync_start",
                        requester_node_id = %format!("{:040x}", self.node_id),
                        promoted_node_id = %format!("{:040x}", promoted_node_id),
                        group_id = group_id,
                        mode = ?mode,
                        sync_reason = "after_forward_fallback_success",
                        "Starting local data-group sync after fallback-forwarded failover"
                    );
                    self.sync_data_raft_groups_after_meta_change()
                        .await
                        .map_err(|e| {
                            warn!(
                                diag_event = "cluster_failover_post_sync_failed",
                                requester_node_id = %format!("{:040x}", self.node_id),
                                promoted_node_id = %format!("{:040x}", promoted_node_id),
                                group_id = group_id,
                                mode = ?mode,
                                sync_reason = "after_forward_fallback_success",
                                error = %e,
                                "Data-group sync failed after fallback-forwarded failover"
                            );
                            e
                        })?;
                    info!(
                        diag_event = "cluster_failover_post_sync_success",
                        requester_node_id = %format!("{:040x}", self.node_id),
                        promoted_node_id = %format!("{:040x}", promoted_node_id),
                        group_id = group_id,
                        mode = ?mode,
                        sync_reason = "after_forward_fallback_success",
                        "Data-group sync completed after fallback-forwarded failover"
                    );
                    info!(
                        diag_event = "cluster_failover_forward_success",
                        requester_node_id = %format!("{:040x}", self.node_id),
                        promoted_node_id = %format!("{:040x}", promoted_node_id),
                        group_id = group_id,
                        mode = ?mode,
                        forward_mode = "error_hint",
                        forward_redis_addr = %leader_redis_addr,
                        "Failover proposal succeeded after fallback forwarding"
                    );

                    Self::wait_for_meta_version(
                        &self.meta_raft,
                        meta.config_version + 1,
                        Duration::from_secs(5),
                    ).await;
                    Self::wait_for_data_raft_leader(
                        &self.multi_raft,
                        group_id,
                        promoted_node_id,
                        Duration::from_secs(10),
                    ).await;

                    return Ok(RespValue::SimpleString("OK".to_string()));
                }
            }
            warn!(
                diag_event = "cluster_failover_failed",
                requester_node_id = %format!("{:040x}", self.node_id),
                promoted_node_id = %format!("{:040x}", promoted_node_id),
                group_id = group_id,
                mode = ?mode,
                error = %e,
                "Failover proposal failed and was not recoverable by forwarding"
            );

            let action = if mode == FailoverMode::Takeover { "takeover" } else { "failover" };
            return Err(AikvError::Internal(format!(
                "Failed to perform {}: {}",
                action, e
            )));
        }
        info!(
            diag_event = "cluster_failover_success",
            requester_node_id = %format!("{:040x}", self.node_id),
            promoted_node_id = %format!("{:040x}", promoted_node_id),
            group_id = group_id,
            mode = ?mode,
            commit_mode = "local",
            "Failover proposal committed locally"
        );
        info!(
            diag_event = "cluster_failover_post_sync_start",
            requester_node_id = %format!("{:040x}", self.node_id),
            promoted_node_id = %format!("{:040x}", promoted_node_id),
            group_id = group_id,
            mode = ?mode,
            sync_reason = "after_local_commit",
            "Starting local data-group sync after failover commit"
        );
        self.sync_data_raft_groups_after_meta_change()
            .await
            .map_err(|e| {
                warn!(
                    diag_event = "cluster_failover_post_sync_failed",
                    requester_node_id = %format!("{:040x}", self.node_id),
                    promoted_node_id = %format!("{:040x}", promoted_node_id),
                    group_id = group_id,
                    mode = ?mode,
                    sync_reason = "after_local_commit",
                    error = %e,
                    "Data-group sync failed after local failover commit"
                );
                e
            })?;
        info!(
            diag_event = "cluster_failover_post_sync_success",
            requester_node_id = %format!("{:040x}", self.node_id),
            promoted_node_id = %format!("{:040x}", promoted_node_id),
            group_id = group_id,
            mode = ?mode,
            sync_reason = "after_local_commit",
            "Data-group sync completed after failover commit"
        );

        // Wait for Data Raft to elect a leader. For the local commit path
        // the metadata is already up-to-date, so no meta-version wait needed.
        Self::wait_for_data_raft_leader(
            &self.multi_raft,
            group_id,
            promoted_node_id,
            Duration::from_secs(10),
        ).await;

        Ok(RespValue::SimpleString("OK".to_string()))
    }

    /// Check if automatic failover is needed and trigger it if so.
    ///
    /// This should be called periodically by a background task.
    /// If this node is a replica and its master is unreachable, this will
    /// trigger failover to promote this replica to master.
    ///
    /// Returns Some(group_id) if failover was triggered, None otherwise.
    pub async fn trigger_automatic_failover_if_needed(&self) -> Result<Option<GroupId>> {
        let meta: ClusterMeta = self.meta_raft.get_cluster_meta();

        // Find if this node is a replica for any group: it must appear in `replicas` and
        // meta must record a *different* leader.
        //
        // Important: when `leader` is `None` (e.g. CLUSTER ADDSLOTSRANGE just ran
        // `CreateGroup` and has not yet applied `UpdateGroupLeader`), the condition
        // `g.leader != Some(self)` is true for the sole shard master and we would
        // incorrectly enter Takeover failover, racing provisioning and tearing down
        // the Redis connection (client sees "Server closed the connection").
        let replica_group = meta.groups.iter().find(|(_, g)| {
            g.replicas.contains(&self.node_id)
                && g.leader.is_some_and(|l| l != self.node_id)
        });

        let (group_id, group_meta) = match replica_group {
            Some((gid, g)) => (*gid, g),
            None => {
                // This node is not a replica, no automatic failover needed
                debug!(
                    "Automatic failover: this node {:040x} is not a replica in any group, groups have these replicas: {:?}",
                    self.node_id,
                    meta.groups.iter().map(|(gid, g)| (*gid, g.replicas.clone())).collect::<Vec<_>>()
                );
                return Ok(None);
            }
        };

        info!(
            "Automatic failover check: node {:040x} is replica of group {}, leader is {:?}, replicas are {:?}",
            self.node_id, group_id, group_meta.leader, group_meta.replicas
        );

        // Check if the master (leader) is reachable
        if let Some(leader_id) = group_meta.leader {
            // Try to get leader info
            if let Some(leader_info) = meta.nodes.get(&leader_id) {
                let leader_reachable = Self::is_addr_reachable(&leader_info.addr, 300).await;

                // If connectivity says leader is reachable but status is stale (e.g. Joining),
                // eagerly repair metadata status to Online so automatic failover logic can work.
                if leader_reachable && leader_info.status != NodeStatus::Online {
                    let update_res = self
                        .update_node_status_with_forward(
                            &meta,
                            leader_id,
                            NodeStatus::Online,
                            false,
                        )
                        .await;
                    if let Ok(mut st) = self.auto_failover_state.lock() {
                        st.remove(&group_id);
                    }
                    match update_res {
                        Ok(_) => info!(
                            diag_event = "auto_failover_mark_online",
                            group_id = group_id,
                            leader_id = leader_id,
                            leader_addr = %leader_info.addr,
                            old_status = ?leader_info.status,
                            leader_reachable = leader_reachable,
                            "Automatic failover path repaired stale leader status to Online"
                        ),
                        Err(e) => warn!(
                            diag_event = "auto_failover_mark_online_failed",
                            group_id = group_id,
                            leader_id = leader_id,
                            leader_addr = %leader_info.addr,
                            old_status = ?leader_info.status,
                            leader_reachable = leader_reachable,
                            error = %e,
                            "Failed to repair stale leader status to Online"
                        ),
                    }
                    return Ok(None);
                }

                // If metadata says Online and connectivity agrees, no failover needed.
                // IMPORTANT: if status is Online but connectivity is down, we must continue
                // into offline debounce path below instead of returning early.
                if leader_info.status == NodeStatus::Online && leader_reachable {
                    if let Ok(mut st) = self.auto_failover_state.lock() {
                        st.remove(&group_id);
                    }
                    debug!(
                        "Automatic failover: leader {} is online for group {}, no failover needed",
                        leader_id, group_id
                    );
                    return Ok(None);
                }

                // Guardrail: do NOT trigger automatic failover for non-offline states.
                if leader_info.status != NodeStatus::Offline {
                    let now = Instant::now();
                    let required_checks = Self::auto_failover_required_consecutive_checks();
                    let min_offline = Self::auto_failover_min_offline_duration();
                    let (ready_to_mark_offline, checks, offline_for_ms) = {
                        let mut st = self.auto_failover_state.lock().map_err(|e| {
                            AikvError::Internal(format!("auto_failover_state lock: {}", e))
                        })?;
                        let entry = st.entry(group_id).or_insert(AutoFailoverState {
                            last_leader_id: Some(leader_id),
                            consecutive_offline_checks: 0,
                            first_offline_at: now,
                            last_trigger_at: None,
                        });
                        if entry.last_leader_id != Some(leader_id) {
                            *entry = AutoFailoverState {
                                last_leader_id: Some(leader_id),
                                consecutive_offline_checks: 1,
                                first_offline_at: now,
                                last_trigger_at: None,
                            };
                        } else {
                            entry.consecutive_offline_checks =
                                entry.consecutive_offline_checks.saturating_add(1);
                        }
                        let checks = entry.consecutive_offline_checks;
                        let offline_for_ms =
                            now.duration_since(entry.first_offline_at).as_millis() as u64;
                        let ready = !leader_reachable
                            && entry.consecutive_offline_checks >= required_checks
                            && now.duration_since(entry.first_offline_at) >= min_offline;
                        (ready, checks, offline_for_ms)
                    };

                    if ready_to_mark_offline {
                        let mark_offline_res = self.update_node_status_with_forward(
                            &meta,
                            leader_id,
                            NodeStatus::Offline,
                            false,
                        )
                        .await;
                        if let Err(e) = mark_offline_res {
                            warn!(
                                diag_event = "auto_failover_blocked",
                                stage = "mark_offline",
                                group_id = group_id,
                                leader_id = leader_id,
                                leader_addr = %leader_info.addr,
                                leader_reachable = leader_reachable,
                                checks = checks,
                                required_checks = required_checks,
                                offline_for_ms = offline_for_ms,
                                min_offline_ms = min_offline.as_millis() as u64,
                                error = %e,
                                "Automatic failover blocked at status update stage"
                            );
                            return Err(AikvError::Internal(format!(
                                "Failed to mark leader {} Offline before automatic failover: {}",
                                leader_id, e
                            )));
                        }
                        warn!(
                            diag_event = "auto_failover_mark_offline",
                            group_id = group_id,
                            leader_id = leader_id,
                            leader_addr = %leader_info.addr,
                            previous_status = ?leader_info.status,
                            checks = checks,
                            required_checks = required_checks,
                            offline_for_ms = offline_for_ms,
                            min_offline_ms = min_offline.as_millis() as u64,
                            leader_reachable = leader_reachable,
                            "Leader marked Offline after repeated unreachable checks"
                        );
                    }

                    warn!(
                        diag_event = "auto_failover_skip_non_offline",
                        group_id = group_id,
                        leader_id = leader_id,
                        leader_status = ?leader_info.status,
                        leader_addr = %leader_info.addr,
                        leader_reachable = leader_reachable,
                        checks = checks,
                        required_checks = required_checks,
                        offline_for_ms = offline_for_ms,
                        min_offline_ms = min_offline.as_millis() as u64,
                        will_mark_offline = ready_to_mark_offline,
                        "Automatic failover skipped: leader is not explicitly Offline"
                    );
                    return Ok(None);
                }
                // Guardrail: status says Offline, but if endpoint is reachable, do not failover.
                if leader_reachable {
                    if let Ok(mut st) = self.auto_failover_state.lock() {
                        st.remove(&group_id);
                    }
                    warn!(
                        diag_event = "auto_failover_skip_leader_reachable",
                        group_id = group_id,
                        leader_id = leader_id,
                        leader_addr = %leader_info.addr,
                        "Automatic failover skipped: leader addr still reachable"
                    );
                    return Ok(None);
                }

                // Debounce + cooldown to avoid oscillation or cross-shard accidental promotion.
                let now = Instant::now();
                let required_checks = Self::auto_failover_required_consecutive_checks();
                let min_offline = Self::auto_failover_min_offline_duration();
                let cooldown = Self::auto_failover_cooldown_duration();
                {
                    let mut st = self
                        .auto_failover_state
                        .lock()
                        .map_err(|e| AikvError::Internal(format!("auto_failover_state lock: {}", e)))?;
                    let entry = st.entry(group_id).or_insert(AutoFailoverState {
                        last_leader_id: Some(leader_id),
                        consecutive_offline_checks: 0,
                        first_offline_at: now,
                        last_trigger_at: None,
                    });
                    if entry.last_leader_id != Some(leader_id) {
                        *entry = AutoFailoverState {
                            last_leader_id: Some(leader_id),
                            consecutive_offline_checks: 1,
                            first_offline_at: now,
                            last_trigger_at: None,
                        };
                    } else {
                        entry.consecutive_offline_checks = entry.consecutive_offline_checks.saturating_add(1);
                    }

                    if let Some(last_trigger) = entry.last_trigger_at {
                        if now.duration_since(last_trigger) < cooldown {
                            info!(
                                diag_event = "auto_failover_cooldown_skip",
                                group_id = group_id,
                                leader_id = leader_id,
                                cooldown_secs = cooldown.as_secs(),
                                "Automatic failover skipped due to cooldown window"
                            );
                            return Ok(None);
                        }
                    }

                    if entry.consecutive_offline_checks < required_checks
                        || now.duration_since(entry.first_offline_at) < min_offline
                    {
                        info!(
                            diag_event = "auto_failover_debounce_wait",
                            group_id = group_id,
                            leader_id = leader_id,
                            checks = entry.consecutive_offline_checks,
                            required_checks = required_checks,
                            offline_for_ms = now.duration_since(entry.first_offline_at).as_millis() as u64,
                            min_offline_ms = min_offline.as_millis() as u64,
                            "Automatic failover debounce window not reached yet"
                        );
                        return Ok(None);
                    }
                }
            }

            // Leader is not reachable (offline or not found)
            info!(
                "Automatic failover: detected unreachable master {} for group {}, triggering failover",
                leader_id, group_id
            );

            // Trigger failover via the same path as manual failover:
            // this path already supports ForwardToLeader auto-forwarding.
            self.cluster_failover(FailoverMode::Takeover, Some(self.node_id))
                .await
                .map_err(|e| {
                    warn!(
                        diag_event = "auto_failover_blocked",
                        stage = "failover_proposal",
                        group_id = group_id,
                        leader_id = leader_id,
                        requester_node_id = %format!("{:040x}", self.node_id),
                        error = %e,
                        "Automatic failover blocked at failover proposal stage"
                    );
                    AikvError::Internal(format!(
                        "Failed to perform automatic failover for group {}: {}",
                        group_id, e
                    ))
                })?;

            info!(
                "Automatic failover: successfully promoted node {} to master of group {}",
                self.node_id, group_id
            );

            if let Ok(mut st) = self.auto_failover_state.lock() {
                if let Some(entry) = st.get_mut(&group_id) {
                    entry.last_trigger_at = Some(Instant::now());
                    entry.consecutive_offline_checks = 0;
                }
            }

            return Ok(Some(group_id));
        }

        // No leader is set for this group - this might mean failover is needed
        // but we don't know who the master should be
        warn!(
            "Automatic failover: group {} has no leader set, this node {} is a replica but cannot determine previous master",
            group_id, self.node_id
        );

        // If we're a replica and there's no leader, we should become the leader
        // This handles the case where the original master completely failed
        info!(
            "Automatic failover: promoting replica {} to master of group {} (no leader exists)",
            self.node_id, group_id
        );

        self.cluster_failover(FailoverMode::Takeover, Some(self.node_id))
            .await
            .map_err(|e| {
                AikvError::Internal(format!(
                    "Failed to perform automatic failover for group {}: {}",
                    group_id, e
                ))
            })?;

        return Ok(Some(group_id));
    }

    /// Handle CLUSTER RESET command.
    ///
    /// Resets the cluster node (SOFT or HARD).
    pub async fn cluster_reset(&self, hard: bool) -> Result<RespValue> {
        if hard {
            // HARD reset: clear all data and cluster state
            // This would require clearing the storage and MetaRaft state
            // For now, just clear slot assignments for this node
            let meta = self.meta_raft.get_cluster_meta();

            for (group_id, group_meta) in &meta.groups {
                if group_meta.leader == Some(self.node_id) {
                    // Clear slots for groups where this node is leader
                    for (slot_idx, &assigned_group) in meta.slots.iter().enumerate() {
                        if assigned_group == *group_id {
                            self.meta_raft
                                .update_slots(slot_idx as u16, (slot_idx + 1) as u16, 0)
                                .await
                                .map_err(|e| {
                                    AikvError::Internal(format!("Failed to clear slot: {}", e))
                                })?;
                        }
                    }
                }
            }
        }
        // SOFT reset: just return OK (minimal reset)
        Ok(RespValue::SimpleString("OK".to_string()))
    }

    /// Handle CLUSTER COUNT-FAILURE-REPORTS command.
    ///
    /// Returns the number of failure reports for a given node.
    pub fn cluster_count_failure_reports(&self, _node_id: NodeId) -> Result<RespValue> {
        // In our implementation, failure detection is handled by Raft
        // Return 0 as we don't track failure reports separately
        Ok(RespValue::Integer(0))
    }

    /// Handle CLUSTER BUMPEPOCH command.
    ///
    /// Advances the cluster config epoch.
    pub fn cluster_bumpepoch(&self) -> Result<RespValue> {
        // In our implementation, epochs are managed by MetaRaft
        // Just return the current epoch
        let meta = self.meta_raft.get_cluster_meta();
        Ok(RespValue::BulkString(Some(Bytes::from(format!(
            "BUMPED {}",
            meta.config_version
        )))))
    }

    /// Handle CLUSTER FLUSHSLOTS command.
    ///
    /// Deletes all slots from this node.
    pub async fn cluster_flushslots(&self) -> Result<RespValue> {
        let meta = self.meta_raft.get_cluster_meta();

        // Find groups where this node is leader and clear their slots
        for (group_id, group_meta) in &meta.groups {
            if group_meta.leader == Some(self.node_id) {
                for (slot_idx, &assigned_group) in meta.slots.iter().enumerate() {
                    if assigned_group == *group_id {
                        self.meta_raft
                            .update_slots(slot_idx as u16, (slot_idx + 1) as u16, 0)
                            .await
                            .map_err(|e| {
                                AikvError::Internal(format!("Failed to flush slot: {}", e))
                            })?;
                    }
                }
            }
        }

        Ok(RespValue::SimpleString("OK".to_string()))
    }

    /// Handle CLUSTER DELSLOTSRANGE command.
    ///
    /// Deletes a range of slots from this node.
    pub async fn cluster_delslotsrange(&self, start: u16, end: u16) -> Result<RespValue> {
        if start > end || end >= TOTAL_SLOTS {
            return Err(AikvError::Invalid(format!(
                "Invalid slot range: {}-{}",
                start, end
            )));
        }

        // Clear the entire range (end is exclusive)
        self.meta_raft
            .update_slots(start, end + 1, 0)
            .await
            .map_err(|e| {
                AikvError::Internal(format!("Failed to delete slots {}-{}: {}", start, end, e))
            })?;

        Ok(RespValue::SimpleString("OK".to_string()))
    }

    /// Handle CLUSTER SETSLOT command.
    pub async fn cluster_setslot(
        &self,
        slot: u16,
        mode: &str,
        node_id: Option<NodeId>,
        requester_node_id: Option<NodeId>,
    ) -> Result<RespValue> {
        let t0 = Instant::now();
        info!(
            diag_event = "cluster_setslot_attempt",
            requester_node_id = %format!("{:040x}", self.node_id),
            slot = slot,
            mode = %mode,
            target_node_id = %node_id.map(|n| format!("{:040x}", n)).unwrap_or_else(|| "-".to_string()),
            "Received CLUSTER SETSLOT request"
        );
        if slot >= TOTAL_SLOTS {
            return Err(AikvError::Invalid(format!("Invalid slot: {}", slot)));
        }
        let meta = self.meta_raft.get_cluster_meta();
        let proposal_res: std::result::Result<(), AikvError> = match mode {
            "MIGRATING" => {
                let target_node =
                    node_id.ok_or_else(|| AikvError::WrongArgCount("CLUSTER SETSLOT".to_string()))?;
                let from_group = *meta
                    .slots
                    .get(slot as usize)
                    .ok_or_else(|| AikvError::Invalid(format!("Invalid slot: {}", slot)))?;
                if from_group == 0 {
                    return Err(AikvError::Internal(format!(
                        "CLUSTERDOWN Hash slot {} not served",
                        slot
                    )));
                }
                let to_group = self.ensure_shard_group_for_node(target_node).await?;
                self.meta_raft
                    .start_migration(slot, from_group, to_group)
                    .await
                    .map_err(|e| AikvError::Internal(format!("Failed to start migration: {}", e)))
                    .map(|_| ())
            }
            "IMPORTING" => {
                let source_node =
                    node_id.ok_or_else(|| AikvError::WrongArgCount("CLUSTER SETSLOT".to_string()))?;
                let from_group = Self::find_group_by_node(&meta, source_node).ok_or_else(|| {
                    AikvError::Invalid(format!("Unknown source node: {:040x}", source_node))
                })?;
                let importing_target = requester_node_id.unwrap_or(self.node_id);
                let to_group = self.ensure_shard_group_for_node(importing_target).await?;
                self.meta_raft
                    .set_slot_migration_state(
                        slot,
                        SlotMigrationState::Importing {
                            from_group,
                            to_group,
                        },
                    )
                    .await
                    .map_err(|e| {
                        AikvError::Internal(format!("Failed to set importing state: {}", e))
                    })
                    .map(|_| ())
            }
            "STABLE" => {
                self.meta_raft
                    .clear_slot_migration(slot)
                    .await
                    .map_err(|e| AikvError::Internal(format!("Failed to clear migration: {}", e)))
                    .map(|_| ())
            }
            "NODE" => {
                let target_node =
                    node_id.ok_or_else(|| AikvError::WrongArgCount("CLUSTER SETSLOT".to_string()))?;
                let target_group = self.ensure_shard_group_for_node(target_node).await?;
                self.meta_raft
                    .update_slots(slot, slot + 1, target_group)
                    .await
                    .map_err(|e| AikvError::Internal(format!("Failed to update slot owner: {}", e)))
                    .map(|_| ())?;
                let _ = self.meta_raft.complete_migration(slot).await;
                let _ = self.meta_raft.clear_slot_migration(slot).await;
                Ok(())
            }
            _ => {
                return Err(AikvError::Invalid(format!(
                    "Invalid CLUSTER SETSLOT mode: {}",
                    mode
                )))
            }
        };

        if let Err(e) = proposal_res {
            let err_msg = e.to_string();
            if err_msg.to_ascii_lowercase().contains("forwardtoleader") {
                info!(
                    diag_event = "cluster_setslot_forward_to_leader",
                    requester_node_id = %format!("{:040x}", self.node_id),
                    slot = slot,
                    mode = %mode,
                    target_node_id = %node_id.map(|n| format!("{:040x}", n)).unwrap_or_else(|| "-".to_string()),
                    error = %err_msg,
                    "CLUSTER SETSLOT got ForwardToLeader, attempting redirect"
                );
                // Prefer `leader_node.addr` from the ForwardToLeader payload: it matches the
                // MetaRaft member address (often `hostname:50051` in Docker) and maps to a
                // container-reachable Redis endpoint (`hostname:6379`). `ClusterMeta.nodes`
                // frequently stores `AIKV_ADVERTISE_HOST` (e.g. 192.168.x.x:6379); using that
                // first for in-cluster redis forwarding often fails (hairpin / wrong path),
                // so we only fall back to meta when the error string carries no addr.
                let mut leader_redis_addr =
                    Self::extract_forward_leader_addr_from_error(&err_msg).unwrap_or_default();
                if leader_redis_addr.is_empty() {
                    let meta_live = self.meta_raft.get_cluster_meta();
                    leader_redis_addr =
                        if let Some(meta_leader_id) = self.meta_raft.get_leader().await {
                            if let Some(meta_leader) = meta_live.nodes.get(&meta_leader_id) {
                                Self::extract_data_address(&meta_leader.addr)
                            } else if let Some(addr) = self.meta_raft.get_member_address(meta_leader_id)
                            {
                                Self::extract_data_address(&addr)
                            } else {
                                String::new()
                            }
                        } else {
                            String::new()
                        };
                }
                if !leader_redis_addr.is_empty() {
                    let forward_requester = if mode == "IMPORTING" {
                        Some(self.node_id)
                    } else {
                        None
                    };
                    self.forward_setslot_to_leader(
                        &leader_redis_addr,
                        slot,
                        mode,
                        node_id,
                        forward_requester,
                    )
                    .await?;
                    self.sync_data_raft_groups_after_meta_change().await?;
                    if mode == "IMPORTING" {
                        self.wait_importing_visibility(slot, 500).await;
                    }
                    info!(
                        diag_event = "cluster_setslot_meta_apply_success",
                        requester_node_id = %format!("{:040x}", self.node_id),
                        slot = slot,
                        mode = %mode,
                        apply_mode = "forwarded",
                        duration_ms = t0.elapsed().as_millis() as u64,
                        "CLUSTER SETSLOT applied via leader forwarding"
                    );
                    return Ok(RespValue::SimpleString("OK".to_string()));
                } else {
                    let metaraft_leader_known = self.meta_raft.get_leader().await.is_some();
                    warn!(
                        diag_event = "cluster_setslot_forward_leader_addr_unresolved",
                        requester_node_id = %format!("{:040x}", self.node_id),
                        slot = slot,
                        mode = %mode,
                        metaraft_leader_known = metaraft_leader_known,
                        target_node_id = %node_id.map(|n| format!("{:040x}", n)).unwrap_or_else(|| "-".to_string()),
                        "CLUSTER SETSLOT ForwardToLeader but could not resolve leader Redis address"
                    );
                }
            }
            warn!(
                diag_event = "cluster_setslot_meta_apply_failed",
                requester_node_id = %format!("{:040x}", self.node_id),
                slot = slot,
                mode = %mode,
                duration_ms = t0.elapsed().as_millis() as u64,
                error = %e,
                "CLUSTER SETSLOT failed"
            );
            return Err(e);
        }
        self.sync_data_raft_groups_after_meta_change().await?;
        if mode == "IMPORTING" {
            self.wait_importing_visibility(slot, 500).await;
        }
        info!(
            diag_event = "cluster_setslot_meta_apply_success",
            requester_node_id = %format!("{:040x}", self.node_id),
            slot = slot,
            mode = %mode,
            apply_mode = "local",
            duration_ms = t0.elapsed().as_millis() as u64,
            "CLUSTER SETSLOT applied locally"
        );
        Ok(RespValue::SimpleString("OK".to_string()))
    }

    /// Handle ASKING command.
    ///
    /// Signals that the next command is for a key being migrated.
    /// This is called on the target node after receiving -ASK redirect.
    pub fn asking(&self) -> Result<RespValue> {
        // In a full implementation, this would set a flag on the connection
        // to allow the next command to operate on an importing slot
        Ok(RespValue::SimpleString("OK".to_string()))
    }

    /// Generate a unique node ID.
    /// This is a utility function for server initialization.
    pub fn generate_node_id() -> NodeId {
        use std::time::{SystemTime, UNIX_EPOCH};

        // Use a combination of timestamp and random number
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64;

        // Mix with random bits
        let random: u64 = rand::random();
        timestamp ^ random
    }

    /// Generate a consistent node ID from a peer address.
    /// This ensures all nodes agree on each other's IDs in multi-master setup.
    pub fn generate_node_id_from_addr(addr: &str) -> NodeId {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        addr.hash(&mut hasher);
        hasher.finish()
    }

    /// Execute a CLUSTER subcommand.
    ///
    /// This is the main dispatcher for CLUSTER commands.
    pub fn execute(&self, args: &[Bytes]) -> Result<RespValue> {
        if args.is_empty() {
            return Err(AikvError::WrongArgCount("CLUSTER".to_string()));
        }

        let subcommand = String::from_utf8_lossy(&args[0]).to_uppercase();
        match subcommand.as_str() {
            "INFO" => self.cluster_info(),
            "NODES" => self.cluster_nodes(),
            "SLOTS" => self.cluster_slots(),
            "MYID" => self.cluster_myid(),
            "KEYSLOT" => {
                if args.len() != 2 {
                    return Err(AikvError::WrongArgCount("CLUSTER KEYSLOT".to_string()));
                }
                self.cluster_keyslot(&args[1])
            }
            "GETKEYSINSLOT" => {
                if args.len() != 3 {
                    return Err(AikvError::WrongArgCount(
                        "CLUSTER GETKEYSINSLOT".to_string(),
                    ));
                }
                let slot = String::from_utf8_lossy(&args[1])
                    .parse::<u16>()
                    .map_err(|_| AikvError::Invalid("Invalid slot".to_string()))?;
                let count = String::from_utf8_lossy(&args[2])
                    .parse::<usize>()
                    .map_err(|_| AikvError::Invalid("Invalid count".to_string()))?;
                self.cluster_getkeysinslot(slot, count)
            }
            "COUNTKEYSINSLOT" => {
                if args.len() != 2 {
                    return Err(AikvError::WrongArgCount(
                        "CLUSTER COUNTKEYSINSLOT".to_string(),
                    ));
                }
                let slot = String::from_utf8_lossy(&args[1])
                    .parse::<u16>()
                    .map_err(|_| AikvError::Invalid("Invalid slot".to_string()))?;
                self.cluster_countkeysinslot(slot)
            }
            "SHARDS" => self.cluster_shards(),
            "MYSHARDID" => self.cluster_myshardid(),
            "SET-CONFIG-EPOCH" => {
                if args.len() != 2 {
                    return Err(AikvError::WrongArgCount(
                        "CLUSTER SET-CONFIG-EPOCH".to_string(),
                    ));
                }
                let epoch = String::from_utf8_lossy(&args[1])
                    .parse::<u64>()
                    .map_err(|_| AikvError::Invalid("Invalid epoch".to_string()))?;
                self.cluster_set_config_epoch(epoch)
            }
            "REPLICAS" => {
                if args.len() != 2 {
                    return Err(AikvError::WrongArgCount("CLUSTER REPLICAS".to_string()));
                }
                let node_id_str = String::from_utf8_lossy(&args[1]);
                let node_id = u64::from_str_radix(&node_id_str, 16)
                    .or_else(|_| node_id_str.parse::<u64>())
                    .map_err(|_| AikvError::Invalid("Invalid node ID".to_string()))?;
                self.cluster_replicas(node_id)
            }
            "SLAVES" => {
                // Deprecated alias for REPLICAS
                if args.len() != 2 {
                    return Err(AikvError::WrongArgCount("CLUSTER SLAVES".to_string()));
                }
                let node_id_str = String::from_utf8_lossy(&args[1]);
                let node_id = u64::from_str_radix(&node_id_str, 16)
                    .or_else(|_| node_id_str.parse::<u64>())
                    .map_err(|_| AikvError::Invalid("Invalid node ID".to_string()))?;
                self.cluster_replicas(node_id)
            }
            "SAVECONFIG" => self.cluster_saveconfig(),
            "BUMPEPOCH" => self.cluster_bumpepoch(),
            "COUNT-FAILURE-REPORTS" => {
                if args.len() != 2 {
                    return Err(AikvError::WrongArgCount(
                        "CLUSTER COUNT-FAILURE-REPORTS".to_string(),
                    ));
                }
                let node_id_str = String::from_utf8_lossy(&args[1]);
                let node_id = u64::from_str_radix(&node_id_str, 16)
                    .or_else(|_| node_id_str.parse::<u64>())
                    .map_err(|_| AikvError::Invalid("Invalid node ID".to_string()))?;
                self.cluster_count_failure_reports(node_id)
            }
            _ => Err(AikvError::InvalidCommand(format!(
                "Unknown CLUSTER subcommand: {}",
                subcommand
            ))),
        }
    }

    /// Handle READONLY command.
    ///
    /// Sets connection to read-only mode for replica reads.
    pub fn readonly(&self) -> Result<RespValue> {
        // For now, just return OK
        // In a full implementation, this would set a flag on the connection
        Ok(RespValue::SimpleString("OK".to_string()))
    }

    /// Handle READWRITE command.
    ///
    /// Sets connection back to read-write mode (default).
    pub fn readwrite(&self) -> Result<RespValue> {
        // For now, just return OK
        // In a full implementation, this would clear the read-only flag
        Ok(RespValue::SimpleString("OK".to_string()))
    }
    /// Handle CLUSTER METARAFT ADDLEARNER command.
    ///
    /// Adds a node as a learner to the MetaRaft cluster. This is the first step
    /// in adding a new voting member to the MetaRaft cluster.
    ///
    /// # Arguments
    ///
    /// * `node_id` - ID of the node to add
    /// * `addr` - Raft address of the node (ip:port for gRPC)
    ///
    /// # Returns
    ///
    /// `OK` on success
    ///
    /// # Example
    ///
    /// ```text
    /// CLUSTER METARAFT ADDLEARNER 2 127.0.0.1:50052
    /// ```
    pub async fn cluster_metaraft_addlearner(
        &self,
        node_id: NodeId,
        addr: String,
    ) -> Result<RespValue> {
        // CRITICAL: Register node address in network factory BEFORE adding learner
        // This enables the Leader to connect to the new node for log replication
        // The address must include http:// scheme for gRPC client
        let grpc_addr = if addr.starts_with("http://") || addr.starts_with("https://") {
            addr.clone()
        } else {
            format!("http://{}", addr)
        };

        // Register address in BOTH factories:
        // 1. MultiRaft factory (for data group replication)
        self.multi_raft.add_node_address(node_id, grpc_addr.clone());
        // 2. MetaRaft factory (for metadata replication) - new in AiDb v0.6.1
        self.meta_raft.add_node_address(node_id, grpc_addr.clone());

        // BasicNode.addr MUST also have http:// scheme for Raft replication
        let node = BasicNode {
            addr: grpc_addr,
        };

        self.meta_raft
            .add_learner(node_id, node)
            .await
            .map_err(|e| AikvError::Internal(format!("Failed to add MetaRaft learner: {}", e)))?;

        Ok(RespValue::SimpleString("OK".to_string()))
    }

    /// Handle CLUSTER METARAFT PROMOTE command.
    ///
    /// Promotes one or more learners to voting members in the MetaRaft cluster.
    /// The provided node IDs will be added to the existing voter set.
    /// Existing voters are automatically retained.
    ///
    /// # Arguments
    ///
    /// * `new_voters` - List of learner node IDs to promote to voters
    ///
    /// # Returns
    ///
    /// `OK` on success
    ///
    /// # Example
    ///
    /// ```text
    /// CLUSTER METARAFT PROMOTE 2 3
    /// ```
    pub async fn cluster_metaraft_promote(&self, new_voters: Vec<NodeId>) -> Result<RespValue> {
        use std::collections::BTreeSet;

        // Get current voters from metrics
        let raft = self.meta_raft.raft();
        let metrics = raft.metrics().borrow().clone();
        let current_voters: BTreeSet<NodeId> =
            metrics.membership_config.membership().voter_ids().collect();

        // Merge current voters with new voters to promote
        let mut all_voters: BTreeSet<NodeId> = current_voters;
        for voter in new_voters {
            all_voters.insert(voter);
        }

        info!("Promoting to voter set: {:?}", all_voters);

        self.meta_raft
            .change_membership(all_voters, true)
            .await
            .map_err(|e| AikvError::Internal(format!("Failed to promote voters: {}", e)))?;

        Ok(RespValue::SimpleString("OK".to_string()))
    }

    /// Handle CLUSTER METARAFT MEMBERS command.
    ///
    /// Returns information about MetaRaft cluster members, including voters and learners.
    ///
    /// # Returns
    ///
    /// Array of member information
    ///
    /// # Example
    ///
    /// ```text
    /// CLUSTER METARAFT MEMBERS
    /// ```
    pub async fn cluster_metaraft_members(&self) -> Result<RespValue> {
        // Get Raft metrics to determine current voters and learners
        let raft = self.meta_raft.raft();
        let metrics = raft.metrics().borrow().clone();

        let mut members = Vec::new();

        // Add voters
        let membership = metrics.membership_config.membership();
        for node_id in membership.voter_ids() {
            members.push(RespValue::Array(Some(vec![
                RespValue::BulkString(Some(Bytes::from(format!("{}", node_id)))),
                RespValue::SimpleString("voter".to_string()),
            ])));
        }

        // Add learners
        for node_id in membership.learner_ids() {
            members.push(RespValue::Array(Some(vec![
                RespValue::BulkString(Some(Bytes::from(format!("{}", node_id)))),
                RespValue::SimpleString("learner".to_string()),
            ])));
        }

        Ok(RespValue::Array(Some(members)))
    }

    /// Return raw raft metrics and membership state for diagnostics
    pub async fn cluster_metaraft_status(&self) -> Result<RespValue> {
        let raft = self.meta_raft.raft();
        let metrics = raft.metrics().borrow().clone();

        // Also include cluster meta snapshot
        let cluster_meta = self.meta_raft.get_cluster_meta();

        let mut info = String::new();
        info.push_str(&format!("metrics: {:?}\n", metrics));
        info.push_str(&format!("cluster_meta: {:?}\n", cluster_meta));

        Ok(RespValue::BulkString(Some(Bytes::from(info))))
    }

    /// Handle CLUSTER METARAFT SETSTATUS command.
    ///
    /// Internal command for updating node status in MetaRaft with consensus.
    pub async fn cluster_metaraft_setstatus(
        &self,
        node_id: NodeId,
        status: NodeStatus,
        is_forwarded: bool,
    ) -> Result<RespValue> {
        self.update_node_status_with_forward(
            &self.meta_raft.get_cluster_meta(),
            node_id,
            status,
            is_forwarded,
        )
        .await?;
        Ok(RespValue::SimpleString("OK".to_string()))
    }
}

#[cfg(feature = "cluster")]
impl ClusterCommands {
    /// Create error response for -MOVED redirection
    pub fn moved_error(slot: u16, addr: &str) -> AikvError {
        AikvError::Moved(slot, addr.to_string())
    }

    /// Create error response for -ASK redirection
    pub fn ask_error(slot: u16, addr: &str) -> AikvError {
        AikvError::Ask(slot, addr.to_string())
    }

    /// Check if a key should be handled by this node.
    ///
    /// Returns `Ok(())` if the key belongs to this node, or an error with
    /// MOVED/ASK redirection information if the key should be handled elsewhere.
    ///
    /// # Arguments
    ///
    /// * `key` - The key to check
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Key belongs to this node
    /// * `Err(AikvError::Moved(slot, addr))` - Key belongs to another node
    /// * `Err(AikvError::Ask(slot, addr))` - Key is being migrated
    ///
    /// # Example
    ///
    /// ```ignore
    /// // Before executing a command, check if the key belongs to this node
    /// cluster_commands.check_key_slot(b"user:1000")?;
    /// // If no error, proceed with the command
    /// ```
    pub fn check_key_slot(&self, key: &[u8]) -> Result<()> {
        self.check_key_slot_with_asking(key, false, false)
    }

    /// Check key ownership with optional ASKING allowance for importing slot.
    pub fn check_key_slot_with_asking(&self, key: &[u8], allow_importing: bool, readonly: bool) -> Result<()> {
        let slot = key_to_slot_with_hash_tag(key);
        self.check_slot_ownership_with_asking(slot, allow_importing, readonly)
    }

    /// Check if a slot should be handled by this node.
    ///
    /// Returns `Ok(())` if the slot belongs to this node, or an error with
    /// MOVED/ASK redirection information if the slot should be handled elsewhere.
    ///
    /// # Arguments
    ///
    /// * `slot` - The slot number to check (0-16383)
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Slot belongs to this node
    /// * `Err(AikvError::Moved(slot, addr))` - Slot belongs to another node
    /// * `Err(AikvError::Ask(slot, addr))` - Slot is being migrated
    pub fn check_slot_ownership(&self, slot: u16) -> Result<()> {
        self.check_slot_ownership_with_asking(slot, false, false)
    }

    pub fn check_slot_ownership_with_asking(&self, slot: u16, allow_importing: bool, readonly: bool) -> Result<()> {
        let meta: ClusterMeta = self.meta_raft.get_cluster_meta();

        // Check if slot is assigned to any group
        if slot as usize >= meta.slots.len() {
            return Err(AikvError::Invalid(format!("Invalid slot: {}", slot)));
        }

        let assigned_group = meta.slots[slot as usize];

        if let Some(migration) = Self::active_slot_migration(&meta, slot) {
            if let SlotMigrationState::Migrating { from_group, to_group }
            | SlotMigrationState::Importing { from_group, to_group } = migration.state
            {
                let in_group = |gid: GroupId| {
                    meta.groups.get(&gid).is_some_and(|g| {
                        g.leader == Some(self.node_id) || g.replicas.contains(&self.node_id)
                    })
                };
                let is_to_group_member = in_group(to_group);
                let is_from_group_member = in_group(from_group);

                if is_to_group_member {
                    if allow_importing {
                        debug!(
                            diag_event = "cluster_route_check_allow_importing",
                            requester_node_id = %format!("{:040x}", self.node_id),
                            slot = slot,
                            from_group = from_group,
                            to_group = to_group,
                            "Route check allowed importing slot by ASKING"
                        );
                        return Ok(());
                    }
                    if let Some(addr) = Self::leader_addr_for_group(&meta, from_group) {
                        debug!(
                            diag_event = "cluster_route_check_moved_redirect",
                            requester_node_id = %format!("{:040x}", self.node_id),
                            slot = slot,
                            from_group = from_group,
                            to_group = to_group,
                            target = %addr,
                            "Importing node without ASKING redirected by MOVED"
                        );
                        return Err(Self::moved_error(slot, &addr));
                    }
                    return Err(AikvError::Internal(format!(
                        "CLUSTERDOWN Hash slot {} not served (migration IMPORTING: no leader address for source group {})",
                        slot, from_group
                    )));
                }

                if is_from_group_member {
                    if let Some(addr) = Self::leader_addr_for_group(&meta, to_group) {
                        debug!(
                            diag_event = "cluster_route_check_ask_redirect",
                            requester_node_id = %format!("{:040x}", self.node_id),
                            slot = slot,
                            from_group = from_group,
                            to_group = to_group,
                            target = %addr,
                            "Migrating source redirected by ASK"
                        );
                        return Err(Self::ask_error(slot, &addr));
                    }
                    return Err(AikvError::Internal(format!(
                        "CLUSTERDOWN Hash slot {} not served (migration MIGRATING: no leader address for target group {})",
                        slot, to_group
                    )));
                }

                if !is_to_group_member && !is_from_group_member {
                    debug!(
                        diag_event = "cluster_route_check_migration_group_miss",
                        requester_node_id = %format!("{:040x}", self.node_id),
                        slot = slot,
                        from_group = from_group,
                        to_group = to_group,
                        "Node matches neither migration source nor target group"
                    );
                }
            }
        }

        // Slot not assigned to any group
        if assigned_group == 0 {
            warn!(
                diag_event = "cluster_route_check_clusterdown",
                requester_node_id = %format!("{:040x}", self.node_id),
                slot = slot,
                "Route check failed: slot not served"
            );
            return Err(AikvError::Internal(format!(
                "CLUSTERDOWN Hash slot {} not served",
                slot
            )));
        }

        // Check if this node owns the slot (is the leader of the assigned group)
        if let Some(group_meta) = meta.groups.get(&assigned_group) {
            // Check if this node is the leader of the group
            if group_meta.leader == Some(self.node_id) {
                // This node owns the slot
                return Ok(());
            }

            // Check if this node is a replica (can handle READONLY requests)
            if group_meta.replicas.contains(&self.node_id) && group_meta.leader != Some(self.node_id) {
                if readonly {
                    return Ok(());
                }
                // This node is a replica, redirect to the leader
                if let Some(leader_id) = group_meta.leader {
                    if let Some(leader_info) = meta.nodes.get(&leader_id) {
                        let data_addr = Self::extract_data_address(&leader_info.addr);
                        return Err(Self::moved_error(slot, &data_addr));
                    }
                }
            }

            // Slot belongs to another node, find the leader and redirect
            if let Some(leader_id) = group_meta.leader {
                if let Some(leader_info) = meta.nodes.get(&leader_id) {
                    let data_addr = Self::extract_data_address(&leader_info.addr);
                    debug!(
                        diag_event = "cluster_route_check_moved_redirect",
                        slot = slot,
                        requester_node_id = %format!("{:040x}", self.node_id),
                        target = %data_addr,
                        "Redirecting key to owner node"
                    );
                    return Err(Self::moved_error(slot, &data_addr));
                }
            }
        }

        // Fallback: slot is assigned but group info is missing
        Err(AikvError::Internal(format!(
            "CLUSTERDOWN Hash slot {} not served (group {} not found)",
            slot, assigned_group
        )))
    }

    /// Check if multiple keys all belong to this node.
    ///
    /// For multi-key commands (like MGET, MSET), all keys must belong to the same
    /// slot, or the command must be rejected. This method checks if all keys
    /// belong to this node.
    ///
    /// # Arguments
    ///
    /// * `keys` - The keys to check
    ///
    /// # Returns
    ///
    /// * `Ok(())` - All keys belong to this node
    /// * `Err(AikvError::Moved(slot, addr))` - Keys belong to another node
    /// * `Err(AikvError::CrossSlot)` - Keys span multiple slots (not supported)
    pub fn check_keys_slot(&self, keys: &[&[u8]]) -> Result<()> {
        self.check_keys_slot_with_asking(keys, false, false)
    }

    pub fn check_keys_slot_with_asking(&self, keys: &[&[u8]], allow_importing: bool, readonly: bool) -> Result<()> {
        if keys.is_empty() {
            return Ok(());
        }

        // Calculate slot for first key (using hash tag extraction)
        let first_slot = key_to_slot_with_hash_tag(keys[0]);

        // Verify all keys are in the same slot
        for key in &keys[1..] {
            let slot = key_to_slot_with_hash_tag(key);
            if slot != first_slot {
                return Err(AikvError::CrossSlot);
            }
        }

        // Check if the slot belongs to this node
        self.check_slot_ownership_with_asking(first_slot, allow_importing, readonly)
    }

    /// Get the slot number for a key.
    ///
    /// This uses hash tag extraction for Redis Cluster compatibility.
    pub fn get_key_slot(key: &[u8]) -> u16 {
        key_to_slot_with_hash_tag(key)
    }

    /// Check if cluster is fully operational (all slots assigned and served).
    ///
    /// Returns `Ok(())` if the cluster is operational, or an error describing
    /// what's wrong.
    pub fn check_cluster_state(&self) -> Result<()> {
        let meta: ClusterMeta = self.meta_raft.get_cluster_meta();

        // Check if all slots are assigned
        let assigned_slots = meta.slots.iter().filter(|&&g| g > 0).count();
        if assigned_slots != TOTAL_SLOTS as usize {
            return Err(AikvError::Internal(format!(
                "CLUSTERDOWN The cluster is down. Only {} of {} slots are assigned",
                assigned_slots, TOTAL_SLOTS
            )));
        }

        // Check if all groups have leaders
        for (group_id, group_meta) in &meta.groups {
            // Check if this group owns any slots
            let owns_slots = meta.slots.contains(group_id);
            if owns_slots && group_meta.leader.is_none() {
                return Err(AikvError::Internal(format!(
                    "CLUSTERDOWN The cluster is down. Group {group_id} has no leader",
                    group_id = group_id
                )));
            }
        }

        Ok(())
    }

    /// Get the node address that owns a specific slot.
    ///
    /// Returns `Some((node_id, addr))` if the slot is assigned, `None` otherwise.
    pub fn get_slot_owner(&self, slot: u16) -> Option<(NodeId, String)> {
        let meta: ClusterMeta = self.meta_raft.get_cluster_meta();

        if slot as usize >= meta.slots.len() {
            return None;
        }

        let assigned_group = meta.slots[slot as usize];
        if assigned_group == 0 {
            return None;
        }

        if let Some(group_meta) = meta.groups.get(&assigned_group) {
            if let Some(leader_id) = group_meta.leader {
                if let Some(leader_info) = meta.nodes.get(&leader_id) {
                    let data_addr = Self::extract_data_address(&leader_info.addr);
                    return Some((leader_id, data_addr));
                }
            }
        }

        None
    }

    /// Get this node's ID.
    pub fn node_id(&self) -> NodeId {
        self.node_id
    }
}

#[cfg(test)]
mod physical_key_tests {
    use super::{physical_raft_storage_key, user_key_from_physical_raft_key};

    fn decode_str(physical: &[u8]) -> String {
        String::from_utf8(user_key_from_physical_raft_key(physical).to_vec()).unwrap()
    }

    #[test]
    fn user_key_db0_round_trip() {
        for k in ["simple", "{tag}suffix", "a:b:c", ""] {
            let phys = physical_raft_storage_key(0, k);
            assert_eq!(decode_str(&phys), k, "db=0 key={:?}", k);
        }
    }

    #[test]
    fn user_key_nonzero_db_round_trip() {
        for (db, user) in [(1, "mykey"), (3, "with:colons:in:name"), (9, "{h}x")] {
            let phys = physical_raft_storage_key(db, user);
            assert_eq!(
                decode_str(&phys),
                user,
                "db={} user={:?} phys={:?}",
                db,
                user,
                String::from_utf8_lossy(&phys)
            );
        }
    }

    #[test]
    fn user_key_utf8_round_trip() {
        let user = "键🙂";
        let phys = physical_raft_storage_key(2, user);
        assert_eq!(decode_str(&phys), user);
    }

    #[test]
    fn user_key_redis_hash_tag_looks_like_physical_stays_whole() {
        // `{foo}bar` is a normal user key (hash tag), not `{tag}:db:user` encoding.
        let k = "{foo}bar";
        let phys = physical_raft_storage_key(0, k);
        assert_eq!(decode_str(&phys), k);
    }

    #[test]
    fn user_key_encoded_db_zero_suffix_returns_full_physical() {
        // Decoder treats `:0:` segment as "not multi-db encoding" and keeps bytes as-is.
        let physical = b"{t}:0:rest";
        assert_eq!(user_key_from_physical_raft_key(physical).as_ref(), physical.as_slice());
    }
}

/// Placeholder struct for when cluster feature is disabled
#[cfg(not(feature = "cluster"))]
pub struct ClusterCommands;

#[cfg(not(feature = "cluster"))]
impl ClusterCommands {
    pub fn cluster_info(&self) -> Result<RespValue> {
        Err(AikvError::ClusterDisabled)
    }
}
