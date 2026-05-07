//! Cluster-mode storage backed by AiDb `MultiRaftNode` (replicated user data).
//!
//! Keys are mapped to the same hash slots as `CLUSTER KEYSLOT` / MOVED checks by
//! [`crate::cluster::commands::physical_raft_storage_key`].

#[cfg(feature = "cluster")]
use crate::cluster::physical_raft_storage_key;
#[cfg(feature = "cluster")]
use crate::cluster::key_to_slot_with_hash_tag;
#[cfg(feature = "cluster")]
use crate::error::{AikvError, Result};
#[cfg(feature = "cluster")]
use crate::storage::{BatchOp, SerializableStoredValue, StoredValue};
#[cfg(feature = "cluster")]
use aidb::cluster::ClusterMeta;
#[cfg(feature = "cluster")]
use aidb::cluster::GroupId;
#[cfg(feature = "cluster")]
use aidb::cluster::MultiRaftNode;
#[cfg(feature = "cluster")]
use aidb::cluster::NodeId;
#[cfg(feature = "cluster")]
use aidb::cluster::Request;
#[cfg(feature = "cluster")]
use aidb::cluster::SlotMigrationState;
#[cfg(feature = "cluster")]
use aidb::cluster::thin_replication::WriteBatch;
#[cfg(feature = "cluster")]
use bytes::Bytes;
#[cfg(feature = "cluster")]
use std::collections::HashMap;
#[cfg(feature = "cluster")]
use std::sync::Arc;
#[cfg(feature = "cluster")]
use std::sync::Mutex;
#[cfg(feature = "cluster")]
use std::sync::OnceLock;
#[cfg(feature = "cluster")]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(feature = "cluster")]
use std::time::{SystemTime, UNIX_EPOCH};
#[cfg(feature = "cluster")]
#[cfg(feature = "cluster")]
static RAFT_IO_RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
#[cfg(feature = "cluster")]
static SCAN_CURSOR_CACHE: OnceLock<Mutex<HashMap<u64, String>>> = OnceLock::new();
#[cfg(feature = "cluster")]
static SCAN_CURSOR_SEQ: AtomicU64 = AtomicU64::new(1);
#[cfg(feature = "cluster")]
const SCAN_CURSOR_CACHE_MAX: usize = 8192;

#[cfg(feature = "cluster")]
fn raft_io_rt() -> &'static tokio::runtime::Runtime {
    RAFT_IO_RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .thread_name("aikv-raft-io")
            .enable_all()
            .build()
            .expect("failed to build tokio runtime for Raft I/O")
    })
}

#[cfg(feature = "cluster")]
fn scan_cursor_cache() -> &'static Mutex<HashMap<u64, String>> {
    SCAN_CURSOR_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(feature = "cluster")]
fn encode_scan_cursor(opaque_cursor: String) -> String {
    if opaque_cursor.is_empty() {
        return String::new();
    }
    let id = SCAN_CURSOR_SEQ.fetch_add(1, Ordering::Relaxed);
    if let Ok(mut cache) = scan_cursor_cache().lock() {
        if cache.len() >= SCAN_CURSOR_CACHE_MAX {
            // Simple bounded cache policy: clear stale cursor map when full.
            cache.clear();
        }
        cache.insert(id, opaque_cursor);
    }
    id.to_string()
}

#[cfg(feature = "cluster")]
fn decode_scan_cursor(cursor: &str) -> Option<String> {
    let id = cursor.parse::<u64>().ok()?;
    scan_cursor_cache()
        .lock()
        .ok()
        .and_then(|cache| cache.get(&id).cloned())
}

#[cfg(feature = "cluster")]
fn map_aidb_err(e: aidb::Error) -> AikvError {
    AikvError::Storage(e.to_string())
}

/// Like [`map_aidb_write_err`] but for read paths: converts "No local storage
/// for Raft group" into a `MOVED` redirect by looking up the group leader in
/// [`ClusterMeta`].
#[cfg(feature = "cluster")]
fn map_aidb_read_err(e: aidb::Error, slot: u16, multi: &MultiRaftNode) -> AikvError {
    let msg = e.to_string();
    if msg.contains("No local storage for Raft group") {
        if let Some(meta_raft) = multi.meta_raft() {
            let meta = meta_raft.get_cluster_meta();
            let group_id = meta.slots[slot as usize];
            if group_id != 0 {
                if let Some(group_meta) = meta.groups.get(&group_id) {
                    // Try MetaRaft cache first, then Data Raft metrics
                    let leader_id = group_meta.leader.or_else(|| {
                        multi
                            .get_raft_group(group_id)
                            .and_then(|r| r.metrics().borrow().current_leader)
                    });
                    if let Some(leader_id) = leader_id {
                        if let Some(node_info) = meta.nodes.get(&leader_id) {
                            return AikvError::Moved(slot, node_info.addr.clone());
                        }
                    }
                    for replica_id in &group_meta.replicas {
                        if let Some(node_info) = meta.nodes.get(replica_id) {
                            return AikvError::Moved(slot, node_info.addr.clone());
                        }
                    }
                }
            }
        }
    }
    AikvError::Storage(msg)
}

/// Active migration for `slot` with a known target group (IMPORTING / MIGRATING).
#[cfg(feature = "cluster")]
fn migration_active_to_group(meta: &ClusterMeta, slot: u16) -> Option<GroupId> {
    let m = meta
        .migrations
        .iter()
        .find(|m| m.slot == slot && !m.is_complete())?;
    match m.state {
        SlotMigrationState::Importing { to_group, .. }
        | SlotMigrationState::Migrating { to_group, .. } => Some(to_group),
        SlotMigrationState::Idle | SlotMigrationState::Complete => None,
    }
}

/// Source-side slot during `CLUSTER SETSLOT MIGRATING`: data still lives in `from_group`.
#[cfg(feature = "cluster")]
fn migration_migrating_from_group(meta: &ClusterMeta, slot: u16) -> Option<GroupId> {
    let m = meta
        .migrations
        .iter()
        .find(|m| m.slot == slot && !m.is_complete())?;
    match m.state {
        SlotMigrationState::Migrating { from_group, .. } => Some(from_group),
        _ => None,
    }
}

#[cfg(feature = "cluster")]
fn node_is_member_of_group(meta: &ClusterMeta, group_id: GroupId, node_id: NodeId) -> bool {
    meta.groups.get(&group_id).is_some_and(|g| {
        g.leader == Some(node_id) || g.replicas.contains(&node_id)
    })
}

/// Read from a local data group without slot routing; retry once after group sync if storage is missing.
#[cfg(feature = "cluster")]
fn get_from_local_group_resilient(
    multi: &MultiRaftNode,
    group_id: GroupId,
    key: &[u8],
) -> aidb::Result<Option<Vec<u8>>> {
    match multi.get_from_local_group(group_id, key) {
        Err(aidb::Error::NotFound(_)) => {
            raft_io_rt().block_on(multi.sync_data_groups_from_meta())?;
            multi.get_from_local_group(group_id, key)
        }
        other => other,
    }
}

#[cfg(feature = "cluster")]
fn extract_leader_id(err: &str) -> Option<u64> {
    let marker = "leader_id: Some(";
    let start = err.find(marker)? + marker.len();
    let end = err[start..].find(')')? + start;
    err[start..end].parse::<u64>().ok()
}

#[cfg(feature = "cluster")]
fn normalize_data_addr(addr: &str) -> String {
    let advertise_host = std::env::var("AIKV_ADVERTISE_HOST")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "127.0.0.1".to_string());
    if let Some((host, port_str)) = addr.rsplit_once(':') {
        if let Ok(port) = port_str.parse::<u16>() {
            if (50051..=50056).contains(&port) {
                let data_port = 6379 + (port - 50051);
                return format!("{}:{}", advertise_host, data_port);
            }
            if (6379..=6384).contains(&port) {
                let h = if host.is_empty() || host == "0.0.0.0" || host == "127.0.0.1" || host == "localhost" {
                    advertise_host.as_str()
                } else {
                    host
                };
                return format!("{}:{}", h, port);
            }
        }
    }
    addr.to_string()
}

#[cfg(feature = "cluster")]
fn resolve_slot_owner_addr(
    slot: u16,
    meta: &aidb::cluster::ClusterMeta,
    multi: &MultiRaftNode,
    fallback_leader_id: Option<NodeId>,
) -> Option<String> {
    let group_id = *meta.slots.get(slot as usize)?;
    if group_id != 0 {
        if let Some(_group_meta) = meta.groups.get(&group_id) {
            // 1. Try MetaRaft cache
            if let Some(group_leader_id) =
                meta.groups.get(&group_id).and_then(|g| g.leader)
            {
                if let Some(node_info) = meta.nodes.get(&group_leader_id) {
                    return Some(normalize_data_addr(&node_info.addr));
                }
            }
            // 2. Fall back to local Data Raft metrics
            if let Some(raft) = multi.get_raft_group(group_id) {
                if let Some(leader_id) = raft.metrics().borrow().current_leader {
                    if let Some(node_info) = meta.nodes.get(&leader_id) {
                        return Some(normalize_data_addr(&node_info.addr));
                    }
                }
            }
            // 3. Ultimate fallback: any replica
            if let Some(group_meta) = meta.groups.get(&group_id) {
                for replica_id in &group_meta.replicas {
                    if let Some(node_info) = meta.nodes.get(replica_id) {
                        return Some(normalize_data_addr(&node_info.addr));
                    }
                }
            }
        }
    }
    if let Some(leader_id) = fallback_leader_id {
        if let Some(node_info) = meta.nodes.get(&leader_id) {
            return Some(normalize_data_addr(&node_info.addr));
        }
    }
    None
}

#[cfg(feature = "cluster")]
fn map_aidb_write_err(e: aidb::Error, slot: u16, multi: &MultiRaftNode) -> AikvError {
    let msg = e.to_string();
    if msg.contains("Raft write batch timeout") {
        tracing::warn!(
            diag_event = "cluster_raft_write_timeout",
            slot = slot,
            detail = %msg,
            "mapped raft write timeout to storage error"
        );
        return AikvError::Storage(msg);
    }
    if msg.contains("ForwardToLeader") {
        let hinted_leader_id = extract_leader_id(&msg);
        if let Some(meta_raft) = multi.meta_raft() {
            let meta = meta_raft.get_cluster_meta();
            if let Some(target_addr) = resolve_slot_owner_addr(slot, &meta, multi, hinted_leader_id) {
                let self_addr = meta
                    .nodes
                    .get(&multi.node_id())
                    .map(|n| normalize_data_addr(&n.addr))
                    .unwrap_or_default();
                if !self_addr.is_empty() && target_addr == self_addr {
                    let group_id = meta.slots[slot as usize];
                    let gm = meta.groups.get(&group_id);
                    tracing::warn!(
                        diag_event = "cluster_raft_forward_self_loop",
                        slot = slot,
                        group_id = group_id,
                        meta_group_leader = ?gm.and_then(|g| g.leader),
                        node_id = multi.node_id(),
                        hinted_leader_id = hinted_leader_id.unwrap_or(0),
                        self_addr = %self_addr,
                        target = %target_addr,
                        detail = %msg,
                        "ForwardToLeader but ClusterMeta routes to this node; data Raft has no usable leader (e.g. lost quorum)"
                    );
                    return AikvError::Storage(
                        "TRYAGAIN Data group leader is converging after failover; please retry"
                            .to_string(),
                    );
                }
                tracing::info!(
                    diag_event = "cluster_raft_forward_to_moved",
                    slot = slot,
                    hinted_leader_id = hinted_leader_id.unwrap_or(0),
                    target = %target_addr,
                    detail = %msg,
                    "mapped AiDb ForwardToLeader to Redis MOVED via latest ClusterMeta slot owner"
                );
                return AikvError::Moved(slot, target_addr);
            }
        }
        tracing::warn!(
            diag_event = "cluster_raft_forward_unparsed",
            slot = slot,
            detail = %msg,
            "ForwardToLeader in error but leader address not resolved"
        );
    }
    if msg.contains("No local storage for Raft group") {
        tracing::warn!(
            diag_event = "cluster_raft_no_local_group",
            slot = slot,
            detail = %msg,
            "write_batch: routed group has no local Raft/storage on this node"
        );
    }
    AikvError::Storage(msg)
}

#[cfg(feature = "cluster")]
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

#[cfg(feature = "cluster")]
fn expiration_meta_key(logical_key: &[u8]) -> Vec<u8> {
    let mut k = Vec::with_capacity(logical_key.len() + 8);
    k.extend_from_slice(b"__exp__:");
    k.extend_from_slice(logical_key);
    k
}

/// Filter a key by database index.
/// Returns true if the key belongs to the specified db_index.
#[cfg(feature = "cluster")]
fn filter_key_by_db_index(user_key: &str, db_index: usize) -> bool {
    if db_index == 0 {
        // DB 0: keys with no hash tag prefix (no {tag} prefix)
        !user_key.starts_with('{')
    } else {
        // Non-0 DB: must have {tag}:db:key format
        if let Some(rest) = user_key.strip_prefix('{') {
            if let Some(colon_pos) = rest.find(':') {
                let db_part = &rest[colon_pos + 1..];
                if let Some(next_colon) = db_part.find(':') {
                    let db_str = &db_part[..next_colon];
                    if let Ok(key_db) = db_str.parse::<usize>() {
                        return key_db == db_index;
                    }
                }
            }
        }
        false
    }
}

#[derive(Clone)]
#[cfg(feature = "cluster")]
pub struct ClusterRaftEngine {
    multi: Arc<MultiRaftNode>,
    db_count: usize,
}

#[cfg(feature = "cluster")]
impl ClusterRaftEngine {
    pub fn new(multi: Arc<MultiRaftNode>, db_count: usize) -> Self {
        Self { multi, db_count }
    }

    fn check_db(&self, db_index: usize) -> Result<()> {
        if db_index >= self.db_count {
            return Err(AikvError::Storage(format!(
                "Invalid database index: {}",
                db_index
            )));
        }
        Ok(())
    }

    /// Check if the key is expired via the legacy `__exp__:` metadata key.
    /// Used as a fallback when the blob's embedded `expires_at` is `None`.
    fn is_expired_legacy<F>(&self, ph: &[u8], map_err_fn: &F) -> Result<bool>
    where
        F: Fn(aidb::Error) -> AikvError,
    {
        let ex_key = expiration_meta_key(ph);
        let slot = key_to_slot_with_hash_tag(ph);
        match self
            .multi
            .get_in_slot(slot, &ex_key)
            .map_err(|e| map_err_fn(e))?
        {
            Some(bytes) if bytes.len() == 8 => {
                let exp_at = u64::from_le_bytes(bytes[..8].try_into().unwrap());
                Ok(now_ms() >= exp_at)
            }
            _ => Ok(false),
        }
    }

    /// Get expiration timestamp from legacy `__exp__:` metadata key.
    /// Returns `Some(timestamp_ms)` if the legacy key exists with valid data,
    /// `None` if the legacy key does not exist or has invalid data.
    fn legacy_expires_at<F>(&self, ph: &[u8], map_err_fn: &F) -> Result<Option<u64>>
    where
        F: Fn(aidb::Error) -> AikvError,
    {
        let ex_key = expiration_meta_key(ph);
        let slot = key_to_slot_with_hash_tag(ph);
        match self
            .multi
            .get_in_slot(slot, &ex_key)
            .map_err(|e| map_err_fn(e))?
        {
            Some(bytes) if bytes.len() == 8 => {
                let val = u64::from_le_bytes(bytes[..8].try_into().unwrap());
                Ok(Some(val))
            }
            _ => Ok(None),
        }
    }

    pub fn get_value(&self, db_index: usize, key: &str) -> Result<Option<StoredValue>> {
        self.check_db(db_index)?;
        let slot = key_to_slot_with_hash_tag(key.as_bytes());
        let map_read = |e: aidb::Error| map_aidb_read_err(e, slot, &self.multi);
        let ph = physical_raft_storage_key(db_index, key);

        // While the slot is MIGRATING, the source still holds keys in `from_group`. Normal
        // slot-based routing can sporadically resolve the wrong group (e.g. MOVED to importing
        // master with an unrelated slot id); read directly from the migration source group.
        if let Some(meta_raft) = self.multi.meta_raft() {
            let meta = meta_raft.get_cluster_meta();
            if let Some(from_group) = migration_migrating_from_group(&meta, slot) {
                let self_node = self.multi.node_id();
                if node_is_member_of_group(&meta, from_group, self_node) {
                    match self
                        .multi
                        .get_from_local_group(from_group, &ph)
                        .map_err(|e| map_read(e))?
                    {
                        Some(serialized) => {
                            let serializable: SerializableStoredValue =
                                bincode::deserialize(&serialized).map_err(|e| {
                                    AikvError::Storage(format!("deserialize: {}", e))
                                })?;
                            let stored_value = StoredValue::from_serializable(serializable);
                            let expired = match stored_value.expires_at() {
                                Some(expires_at) => now_ms() >= expires_at,
                                None => self.is_expired_legacy(&ph, &map_read)?,
                            };
                            if expired {
                                return Ok(None);
                            }
                            return Ok(Some(stored_value));
                        }
                        None => return Ok(None),
                    }
                }
            }
        }

        let route_slot = key_to_slot_with_hash_tag(&ph);
        match self
            .multi
            .get_in_slot(route_slot, &ph)
            .map_err(|e| map_read(e))?
        {
            Some(serialized) => {
                let serializable: SerializableStoredValue = bincode::deserialize(&serialized)
                    .map_err(|e| AikvError::Storage(format!("deserialize: {}", e)))?;
                let stored_value = StoredValue::from_serializable(serializable);
                let expired = match stored_value.expires_at() {
                    Some(expires_at) => now_ms() >= expires_at,
                    None => self.is_expired_legacy(&ph, &map_read)?,
                };
                if expired {
                    // Clean up: remove data key + legacy __exp__: key
                    let mut batch = WriteBatch::new();
                    batch.delete(ph.clone());
                    batch.delete(expiration_meta_key(&ph));
                    let _ = raft_io_rt()
                        .block_on(self.multi.write_batch_for_route_key(&ph, batch));
                    return Ok(None);
                }
                Ok(Some(stored_value))
            }
            None => Ok(None),
        }
    }

    /// During CLUSTER SETSLOT IMPORTING, allow target node to persist incoming
    /// RESTORE writes directly to the importing Raft group, instead of normal
    /// slot-owner routing based on `meta.slots`.
    fn try_write_importing_locally(&self, slot: u16, batch: WriteBatch) -> Result<bool> {
        let Some(meta_raft) = self.multi.meta_raft() else {
            return Ok(false);
        };
        let meta = meta_raft.get_cluster_meta();
        let Some(to_group) = migration_active_to_group(&meta, slot) else {
            return Ok(false);
        };

        let self_node = self.multi.node_id();
        if !node_is_member_of_group(&meta, to_group, self_node) {
            tracing::debug!(
                diag_event = "cluster_importing_local_write_skip_group_mismatch",
                slot = slot,
                node_id = self_node,
                to_group = to_group,
                "Skip importing local write: node not in migration target group"
            );
            return Ok(false);
        }

        let raft = if let Some(r) = self.multi.get_raft_group(to_group) {
            r
        } else {
            tracing::warn!(
                diag_event = "cluster_importing_local_write_group_missing_before_sync",
                slot = slot,
                node_id = self_node,
                to_group = to_group,
                "Importing local write target group missing; retry after sync_data_groups_from_meta"
            );
            if let Err(e) = raft_io_rt().block_on(self.multi.sync_data_groups_from_meta()) {
                tracing::warn!(
                    diag_event = "cluster_importing_local_write_group_sync_failed",
                    slot = slot,
                    node_id = self_node,
                    to_group = to_group,
                    error = %e,
                    "sync_data_groups_from_meta failed for importing local write"
                );
                return Ok(false);
            }
            let Some(r) = self.multi.get_raft_group(to_group) else {
                tracing::warn!(
                    diag_event = "cluster_importing_local_write_group_missing_after_sync",
                    slot = slot,
                    node_id = self_node,
                    to_group = to_group,
                    "Importing local write target group still missing after sync"
                );
                return Ok(false);
            };
            r
        };

        raft_io_rt()
            .block_on(async { raft.client_write(Request::WriteBatch(batch)).await })
            .map_err(|e| AikvError::Storage(format!("importing local write failed: {:?}", e)))?;
        tracing::debug!(
            diag_event = "cluster_importing_local_write_success",
            slot = slot,
            node_id = self_node,
            to_group = to_group,
            "Importing local write succeeded"
        );
        Ok(true)
    }

    /// During `MIGRATING`, propose the batch on `from_group` from the source node (mirror of
    /// [`Self::try_write_importing_locally`] for deletes at the end of `MIGRATE`).
    fn try_write_migrating_source_locally(&self, slot: u16, batch: WriteBatch) -> Result<bool> {
        let Some(meta_raft) = self.multi.meta_raft() else {
            return Ok(false);
        };
        let meta = meta_raft.get_cluster_meta();
        let Some(from_group) = migration_migrating_from_group(&meta, slot) else {
            return Ok(false);
        };

        let self_node = self.multi.node_id();
        if !node_is_member_of_group(&meta, from_group, self_node) {
            return Ok(false);
        }

        let raft = if let Some(r) = self.multi.get_raft_group(from_group) {
            r
        } else {
            if let Err(e) = raft_io_rt().block_on(self.multi.sync_data_groups_from_meta()) {
                tracing::warn!(
                    diag_event = "cluster_migrating_source_write_group_sync_failed",
                    slot = slot,
                    node_id = self_node,
                    from_group = from_group,
                    error = %e,
                    "sync_data_groups_from_meta failed for migrating source local delete"
                );
                return Ok(false);
            }
            let Some(r) = self.multi.get_raft_group(from_group) else {
                return Ok(false);
            };
            r
        };

        raft_io_rt()
            .block_on(async { raft.client_write(Request::WriteBatch(batch)).await })
            .map_err(|e| {
                AikvError::Storage(format!("migrating source local write failed: {:?}", e))
            })?;
        tracing::debug!(
            diag_event = "cluster_migrating_source_local_write_success",
            slot = slot,
            node_id = self_node,
            from_group = from_group,
            "Migrating source local write/delete succeeded"
        );
        Ok(true)
    }

    pub fn set_value(&self, db_index: usize, key: String, value: StoredValue) -> Result<()> {
        self.check_db(db_index)?;
        let slot = key_to_slot_with_hash_tag(key.as_bytes());
        let ph = physical_raft_storage_key(db_index, &key);
        let route_key = ph.clone();
        let serializable = value.to_serializable();
        let serialized = bincode::serialize(&serializable)
            .map_err(|e| AikvError::Storage(format!("serialize: {}", e)))?;

        let mut batch = WriteBatch::new();
        batch.put(ph.clone(), serialized);

        // Importing target fast-path for slot migration.
        if self.try_write_importing_locally(slot, batch.clone())? {
            return Ok(());
        }

        raft_io_rt()
            .block_on(self.multi.write_batch_for_route_key(&route_key, batch))
            .map_err(|e| map_aidb_write_err(e, slot, &self.multi))?;
        Ok(())
    }

    pub fn update_value<F>(&self, db_index: usize, key: &str, f: F) -> Result<bool>
    where
        F: FnOnce(&mut StoredValue) -> Result<()>,
    {
        let mut value = match self.get_value(db_index, key)? {
            Some(v) => v,
            None => return Ok(false),
        };
        f(&mut value)?;
        self.set_value(db_index, key.to_string(), value)?;
        Ok(true)
    }

    pub fn delete_and_get(&self, db_index: usize, key: &str) -> Result<Option<StoredValue>> {
        self.check_db(db_index)?;
        let slot = key_to_slot_with_hash_tag(key.as_bytes());
        let value = self.get_value(db_index, key)?;
        if value.is_none() {
            return Ok(None);
        }
        let ph = physical_raft_storage_key(db_index, key);
        let route_key = ph.clone();
        let mut batch = WriteBatch::new();
        batch.delete(ph);
        if self.try_write_migrating_source_locally(slot, batch.clone())? {
            return Ok(value);
        }
        raft_io_rt()
            .block_on(self.multi.write_batch_for_route_key(&route_key, batch))
            .map_err(|e| map_aidb_write_err(e, slot, &self.multi))?;
        Ok(value)
    }

    pub fn write_batch(&self, db_index: usize, operations: Vec<(String, BatchOp)>) -> Result<()> {
        self.check_db(db_index)?;
        if operations.is_empty() {
            return Ok(());
        }
        let slot = key_to_slot_with_hash_tag(operations[0].0.as_bytes());
        let route_key = physical_raft_storage_key(db_index, &operations[0].0);
        let mut batch = WriteBatch::new();
        for (key, op) in operations {
            let ph = physical_raft_storage_key(db_index, &key);
            match op {
                BatchOp::Set(value) => {
                    let stored = StoredValue::new_string(value);
                    let serializable = stored.to_serializable();
                    let serialized = bincode::serialize(&serializable)
                        .map_err(|e| AikvError::Storage(format!("serialize: {}", e)))?;
                    batch.put(ph.clone(), serialized);
                }
                BatchOp::Delete => {
                    batch.delete(ph.clone());
                }
            }
        }
        raft_io_rt()
            .block_on(self.multi.write_batch_for_route_key(&route_key, batch))
            .map_err(|e| map_aidb_write_err(e, slot, &self.multi))?;
        Ok(())
    }

    pub fn get_from_db(&self, db_index: usize, key: &str) -> Result<Option<Bytes>> {
        match self.get_value(db_index, key)? {
            Some(stored_value) => match stored_value.as_string() {
                Ok(b) => Ok(Some(b.clone())),
                Err(_) => Ok(None),
            },
            None => Ok(None),
        }
    }

    pub fn get(&self, key: &str) -> Result<Option<Bytes>> {
        self.get_from_db(0, key)
    }

    pub fn set_in_db(&self, db_index: usize, key: String, value: Bytes) -> Result<()> {
        let stored_value = StoredValue::new_string(value);
        self.set_value(db_index, key, stored_value)
    }

    pub fn set(&self, key: String, value: Bytes) -> Result<()> {
        self.set_in_db(0, key, value)
    }

    pub fn set_with_expiration_in_db(
        &self,
        db_index: usize,
        key: String,
        value: Bytes,
        expires_at: u64,
    ) -> Result<()> {
        let mut stored_value = StoredValue::new_string(value);
        stored_value.set_expiration(Some(expires_at));
        self.set_value(db_index, key, stored_value)
    }

    pub fn set_expire_in_db(&self, db_index: usize, key: &str, expire_ms: u64) -> Result<bool> {
        let slot = key_to_slot_with_hash_tag(key.as_bytes());
        let mut value = match self.get_value(db_index, key)? {
            Some(v) => v,
            None => return Ok(false),
        };
        let ph = physical_raft_storage_key(db_index, key);
        let route_key = ph.clone();
        let expire_at = now_ms() + expire_ms;
        value.set_expiration(Some(expire_at));
        let serializable = value.to_serializable();
        let serialized = bincode::serialize(&serializable)
            .map_err(|e| AikvError::Storage(format!("serialize: {}", e)))?;
        let mut batch = WriteBatch::new();
        batch.put(ph.clone(), serialized);
        // Clean up legacy __exp__: key in case it still exists
        batch.delete(expiration_meta_key(&ph));
        raft_io_rt()
            .block_on(self.multi.write_batch_for_route_key(&route_key, batch))
            .map_err(|e| map_aidb_write_err(e, slot, &self.multi))?;
        Ok(true)
    }

    pub fn set_expire_at_in_db(
        &self,
        db_index: usize,
        key: &str,
        timestamp_ms: u64,
    ) -> Result<bool> {
        let slot = key_to_slot_with_hash_tag(key.as_bytes());
        let mut value = match self.get_value(db_index, key)? {
            Some(v) => v,
            None => return Ok(false),
        };
        let ph = physical_raft_storage_key(db_index, key);
        let route_key = ph.clone();
        value.set_expiration(Some(timestamp_ms));
        let serializable = value.to_serializable();
        let serialized = bincode::serialize(&serializable)
            .map_err(|e| AikvError::Storage(format!("serialize: {}", e)))?;
        let mut batch = WriteBatch::new();
        batch.put(ph.clone(), serialized);
        // Clean up legacy __exp__: key in case it still exists
        batch.delete(expiration_meta_key(&ph));
        raft_io_rt()
            .block_on(self.multi.write_batch_for_route_key(&route_key, batch))
            .map_err(|e| map_aidb_write_err(e, slot, &self.multi))?;
        Ok(true)
    }

    pub fn get_ttl_in_db(&self, db_index: usize, key: &str) -> Result<i64> {
        let slot = key_to_slot_with_hash_tag(key.as_bytes());
        let ph = physical_raft_storage_key(db_index, key);
        let map_read = |e: aidb::Error| map_aidb_read_err(e, slot, &self.multi);
        let route_slot = key_to_slot_with_hash_tag(&ph);
        match self
            .multi
            .get_in_slot(route_slot, &ph)
            .map_err(|e| map_read(e))?
        {
            Some(serialized) => {
                if let Ok(sv) = bincode::deserialize::<SerializableStoredValue>(&serialized) {
                    let stored_value = StoredValue::from_serializable(sv);
                    if let Some(expires_at) = stored_value.expires_at() {
                        let now = now_ms();
                        if expires_at > now {
                            return Ok((expires_at - now) as i64);
                        } else {
                            return Ok(-2);
                        }
                    }
                    // No embedded expiration, check legacy __exp__:key
                    if let Some(legacy_ts) = self.legacy_expires_at(&ph, &map_read)? {
                        let now = now_ms();
                        if legacy_ts > now {
                            return Ok((legacy_ts - now) as i64);
                        } else {
                            return Ok(-2);
                        }
                    }
                    return Ok(-1);
                }
                Ok(-2)
            }
            None => Ok(-2),
        }
    }

    pub fn get_expire_time_in_db(&self, db_index: usize, key: &str) -> Result<i64> {
        let slot = key_to_slot_with_hash_tag(key.as_bytes());
        let ph = physical_raft_storage_key(db_index, key);
        let map_read = |e: aidb::Error| map_aidb_read_err(e, slot, &self.multi);
        let route_slot = key_to_slot_with_hash_tag(&ph);
        match self
            .multi
            .get_in_slot(route_slot, &ph)
            .map_err(|e| map_read(e))?
        {
            Some(serialized) => {
                if let Ok(sv) = bincode::deserialize::<SerializableStoredValue>(&serialized) {
                    let stored_value = StoredValue::from_serializable(sv);
                    if let Some(expires_at) = stored_value.expires_at() {
                        let now = now_ms();
                        if now >= expires_at {
                            return Ok(-2);
                        }
                        return Ok(expires_at as i64);
                    }
                    // No embedded expiration, check legacy __exp__:key
                    if let Some(legacy_ts) = self.legacy_expires_at(&ph, &map_read)? {
                        let now = now_ms();
                        if now >= legacy_ts {
                            return Ok(-2);
                        }
                        return Ok(legacy_ts as i64);
                    }
                    return Ok(-1);
                }
                Ok(-2)
            }
            None => Ok(-2),
        }
    }

    pub fn persist_in_db(&self, db_index: usize, key: &str) -> Result<bool> {
        let slot = key_to_slot_with_hash_tag(key.as_bytes());
        let ph = physical_raft_storage_key(db_index, key);
        let map_read = |e: aidb::Error| map_aidb_read_err(e, slot, &self.multi);
        let route_slot = key_to_slot_with_hash_tag(&ph);
        let serialized = match self
            .multi
            .get_in_slot(route_slot, &ph)
            .map_err(|e| map_read(e))?
        {
            Some(v) => v,
            None => return Ok(false),
        };
        if let Ok(sv) = bincode::deserialize::<SerializableStoredValue>(&serialized) {
            let mut stored_value = StoredValue::from_serializable(sv);
            if stored_value.expires_at().is_some() {
                stored_value.set_expiration(None);
                let new_serialized = bincode::serialize(&stored_value.to_serializable())
                    .map_err(|e| AikvError::Storage(format!("serialize: {}", e)))?;
                let route_key = ph.clone();
                let mut batch = WriteBatch::new();
                batch.put(ph.clone(), new_serialized);
                // Clean up legacy __exp__: key in case it still exists
                batch.delete(expiration_meta_key(&ph));
                raft_io_rt()
                    .block_on(self.multi.write_batch_for_route_key(&route_key, batch))
                    .map_err(|e| map_aidb_write_err(e, slot, &self.multi))?;
                return Ok(true);
            }
            // No embedded expiration, check legacy __exp__:key
            if let Some(legacy_ts) = self.legacy_expires_at(&ph, &map_read)? {
                if now_ms() >= legacy_ts {
                    return Ok(false);
                }
                // Legacy TTL exists but not expired. Delete legacy key to persist.
                let route_key = ph.clone();
                let mut batch = WriteBatch::new();
                batch.delete(expiration_meta_key(&ph));
                raft_io_rt()
                    .block_on(self.multi.write_batch_for_route_key(&route_key, batch))
                    .map_err(|e| map_aidb_write_err(e, slot, &self.multi))?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn delete_from_db(&self, db_index: usize, key: &str) -> Result<bool> {
        Ok(self.delete_and_get(db_index, key)?.is_some())
    }

    pub fn delete(&self, key: &str) -> Result<bool> {
        self.delete_from_db(0, key)
    }

    /// RESTORE `BUSYKEY` check after `ASKING` while the slot is IMPORTING: [`Self::get_value`]
    /// routes by `meta.slots` (still the source), which returns `MOVED`. Read only the migration
    /// target group's local state machine instead.
    pub fn exists_in_db_importing_restore(&self, db_index: usize, key: &str) -> Result<bool> {
        self.check_db(db_index)?;
        let slot = key_to_slot_with_hash_tag(key.as_bytes());
        let ph = physical_raft_storage_key(db_index, key);
        let Some(meta_raft) = self.multi.meta_raft() else {
            return self.exists_in_db(db_index, key);
        };
        let meta = meta_raft.get_cluster_meta();
        let Some(to_group) = migration_active_to_group(&meta, slot) else {
            return self.exists_in_db(db_index, key);
        };
        let self_node = self.multi.node_id();
        if !node_is_member_of_group(&meta, to_group, self_node) {
            return self.exists_in_db(db_index, key);
        }

        let raw = get_from_local_group_resilient(&self.multi, to_group, &ph).map_err(map_aidb_err)?;
        let Some(serialized) = raw else {
            return Ok(false);
        };
        let serializable: SerializableStoredValue = bincode::deserialize(&serialized)
            .map_err(|e| AikvError::Storage(format!("deserialize: {}", e)))?;
        let stored_value = StoredValue::from_serializable(serializable);

        let expired = match stored_value.expires_at() {
            Some(expires_at) => now_ms() >= expires_at,
            None => {
                // Fall back to legacy __exp__: key
                let ex_key = expiration_meta_key(&ph);
                match get_from_local_group_resilient(&self.multi, to_group, &ex_key).map_err(map_aidb_err)? {
                    Some(bytes) if bytes.len() == 8 => {
                        let exp_at = u64::from_le_bytes(bytes[..8].try_into().unwrap());
                        now_ms() >= exp_at
                    }
                    _ => false,
                }
            }
        };
        Ok(!expired)
    }

    pub fn exists_in_db(&self, db_index: usize, key: &str) -> Result<bool> {
        Ok(self.get_value(db_index, key)?.is_some())
    }

    pub fn exists(&self, key: &str) -> Result<bool> {
        self.exists_in_db(0, key)
    }

    pub fn get_all_keys_in_db(&self, _db_index: usize) -> Result<Vec<String>> {
        Err(Self::unsup("KEYS / get_all_keys"))
    }

    pub fn scan_keys_in_db(
        &self,
        db_index: usize,
        cursor: &str,
        count: usize,
    ) -> Result<(String, Vec<String>)> {
        self.check_db(db_index)?;
        let count = count.max(1).min(1000);

        let mut result_keys: Vec<String> = Vec::new();
        let mut next_cursor: Option<String> = match cursor {
            "" | "0" => None,
            _ => decode_scan_cursor(cursor).or_else(|| Some(cursor.to_string())),
        };

        // Loop to handle db_index filtering — may need multiple scan batches
        // when many keys belong to other databases.
        loop {
            let needed = count - result_keys.len();
            if needed == 0 {
                break;
            }

            let (cursor_out, keys) = raft_io_rt()
                .block_on(self.multi.scan_groups_streaming(
                    next_cursor.as_deref(),
                    needed,
                ))
                .map_err(|e| AikvError::Storage(format!("Scan failed: {}", e)))?;

            // Filter by db_index and add to results
            for key_bytes in &keys {
                // Skip expiration metadata keys
                if key_bytes.starts_with(b"__exp__:") {
                    continue;
                }

                let user_key = match std::str::from_utf8(key_bytes) {
                    Ok(k) => k,
                    Err(_) => continue,
                };

                if !filter_key_by_db_index(user_key, db_index) {
                    continue;
                }

                result_keys.push(user_key.to_string());

                if result_keys.len() >= count {
                    break;
                }
            }

            if result_keys.len() >= count {
                // We have enough keys; save cursor for next call.
                next_cursor = Some(cursor_out);
                break;
            }

            if cursor_out.is_empty() {
                // Exhausted all groups.
                break;
            }

            // Need more keys — continue with updated cursor.
            next_cursor = Some(cursor_out);
        }

        let final_cursor = if result_keys.len() < count {
            String::new() // scan complete
        } else {
            encode_scan_cursor(next_cursor.unwrap_or_default())
        };

        Ok((final_cursor, result_keys))
    }

    pub fn dbsize_in_db(&self, _db_index: usize) -> Result<usize> {
        Ok(self.multi.aggregate_dbsize())
    }

    pub fn keyspace_stats_in_db(&self, db_index: usize) -> Result<(usize, usize, u64)> {
        // In cluster mode, we need to scan and count keys per db_index.
        // This is expensive but necessary for accurate INFO keyspace output.
        let mut key_count = 0usize;
        let groups = self.multi.list_groups();

        for group_id in groups {
            let mut resume_key: Option<Vec<u8>> = None;
            loop {
                let result = raft_io_rt()
                    .block_on(self.multi.scan_group_streaming(
                        group_id, 1000, resume_key.as_deref(),
                    ))
                    .map_err(|e| AikvError::Storage(format!("Keyspace scan failed: {}", e)))?;

                for key_bytes in &result.keys {
                    // Skip expiration metadata keys
                    if key_bytes.starts_with(b"__exp__:") {
                        continue;
                    }

                    let user_key = match std::str::from_utf8(key_bytes) {
                        Ok(k) => k,
                        Err(_) => continue,
                    };

                    // Filter by database index
                    if filter_key_by_db_index(user_key, db_index) {
                        key_count += 1;
                    }
                }

                if result.exhausted {
                    break;
                }
                resume_key = result.last_key;
            }
        }

        Ok((key_count, 0, 0))
    }

    pub fn flush_db(&self, db_index: usize) -> Result<()> {
        self.check_db(db_index)?;
        let groups = self.multi.list_groups();

        for group_id in groups {
            // Use streaming scan with batches to avoid loading all keys at once
            let mut resume_key: Option<Vec<u8>> = None;
            loop {
                let result = raft_io_rt()
                    .block_on(self.multi.scan_group_streaming(
                        group_id, 1000, resume_key.as_deref(),
                    ))
                    .map_err(|e| AikvError::Storage(format!("Flush scan failed: {}", e)))?;

                if result.keys.is_empty() {
                    break;
                }

                let mut batch = WriteBatch::new();
                let mut has_deletes = false;

                for key_bytes in &result.keys {
                    // Skip expiration metadata keys
                    if key_bytes.starts_with(b"__exp__:") {
                        continue;
                    }

                    let user_key = match std::str::from_utf8(key_bytes) {
                        Ok(k) => k,
                        Err(_) => continue,
                    };

                    // Filter by database index
                    if !filter_key_by_db_index(user_key, db_index) {
                        continue;
                    }

                    // Add key and its expiration metadata to batch
                    // apply_batch_internal will add sm: prefix internally
                    let ph = physical_raft_storage_key(db_index, user_key);
                    batch.delete(ph.clone());

                    // Expiration metadata key (apply_batch_internal adds sm: prefix)
                    let mut exp_key = Vec::with_capacity(ph.len() + 10);
                    exp_key.extend_from_slice(b"__exp__:");
                    exp_key.extend_from_slice(&ph);
                    batch.delete(exp_key);
                    has_deletes = true;
                }

                if has_deletes {
                    if let Some(first_op) = batch.ops.first() {
                        let route_key = first_op.key().to_vec();
                        raft_io_rt()
                            .block_on(self.multi.write_batch_for_route_key(&route_key, batch))
                            .map_err(|e| map_aidb_write_err(e, 0, &self.multi))?;
                    }
                }

                // Check if group is exhausted
                if result.exhausted {
                    break;
                }
                resume_key = result.last_key;
            }
        }

        // Reset key counts on all groups to ensure accurate counting after flush.
        // Note: This is necessary because delete operations via Raft WriteBatch
        // (apply_batch_internal) do not update the key counter.
        self.multi.reset_all_key_counts();

        // Flush to persist tombstones to SSTables.
        // We do NOT call clear_all_data() here because that would also wipe
        // Raft metadata (raft:vote, raft:log:*, raft:membership) stored in
        // the same SSTable files, corrupting the Raft cluster.
        // The deleted sm: keys are logically removed (tombstones); physical
        // cleanup of old SSTable files is handled by normal compaction.
        for group_id in self.multi.list_groups() {
            if let Some(storage) = self.multi.storage().get_group(group_id) {
                if let Err(e) = storage.db().flush() {
                    tracing::warn!("flush_db: flush group {} failed: {}", group_id, e);
                }
            }
        }

        Ok(())
    }

    pub fn flush_all(&self) -> Result<()> {
        let groups = self.multi.list_groups();

        for group_id in groups {
            // Use streaming scan with batches
            let mut resume_key: Option<Vec<u8>> = None;
            loop {
                let result = raft_io_rt()
                    .block_on(self.multi.scan_group_streaming(
                        group_id, 1000, resume_key.as_deref(),
                    ))
                    .map_err(|e| AikvError::Storage(format!("Flush scan failed: {}", e)))?;

                if result.keys.is_empty() {
                    break;
                }

                let mut batch = WriteBatch::new();
                let mut has_deletes = false;

                for key_bytes in &result.keys {
                    // Skip __exp__: metadata keys - they'll be cleaned up with their data key
                    if key_bytes.starts_with(b"__exp__:") {
                        continue;
                    }

                    // For flush_all, delete all user keys
                    // Keys returned from scan are logical keys (no sm: prefix)
                    // apply_batch_internal will add sm: prefix internally
                    batch.delete(key_bytes.clone());

                    // Also delete expiration metadata (logical key only, apply_batch_internal adds prefix)
                    let mut exp_key = Vec::with_capacity(key_bytes.len() + 10);
                    exp_key.extend_from_slice(b"__exp__:");
                    exp_key.extend_from_slice(key_bytes);
                    batch.delete(exp_key);
                    has_deletes = true;
                }

                if has_deletes {
                    if let Some(first_op) = batch.ops.first() {
                        let route_key = first_op.key().to_vec();
                        raft_io_rt()
                            .block_on(self.multi.write_batch_for_route_key(&route_key, batch))
                            .map_err(|e| map_aidb_write_err(e, 0, &self.multi))?;
                    }
                }

                // Check if group is exhausted
                if result.exhausted {
                    break;
                }
                resume_key = result.last_key;
            }
        }

        // Reset key counts on all groups to ensure accurate counting after flush.
        self.multi.reset_all_key_counts();

        // Flush to persist tombstones to SSTables.
        // We do NOT call clear_all_data() here because that would also wipe
        // Raft metadata stored in the same SSTable files.
        for group_id in self.multi.list_groups() {
            if let Some(storage) = self.multi.storage().get_group(group_id) {
                if let Err(e) = storage.db().flush() {
                    tracing::warn!("flush_all: flush group {} failed: {}", group_id, e);
                }
            }
        }

        Ok(())
    }

    pub fn swap_db(&self, _db1: usize, _db2: usize) -> Result<()> {
        Err(Self::unsup("SWAPDB"))
    }

    pub fn move_key(&self, _src_db: usize, _dst_db: usize, _key: &str) -> Result<bool> {
        Err(Self::unsup("MOVE"))
    }

    pub fn rename_in_db(&self, _db_index: usize, _old: &str, _new: &str) -> Result<bool> {
        Err(Self::unsup("RENAME"))
    }

    pub fn rename_nx_in_db(&self, _db_index: usize, _old: &str, _new: &str) -> Result<bool> {
        Err(Self::unsup("RENAMENX"))
    }

    pub fn copy_in_db(
        &self,
        _src_db: usize,
        _dst_db: usize,
        _src_key: &str,
        _dst_key: &str,
        _replace: bool,
    ) -> Result<bool> {
        Err(Self::unsup("COPY"))
    }

    pub fn random_key_in_db(&self, _db_index: usize) -> Result<Option<String>> {
        Err(Self::unsup("RANDOMKEY"))
    }

    pub fn export_all_databases(&self) -> Result<Vec<HashMap<String, StoredValue>>> {
        Err(Self::unsup("export"))
    }

    pub fn get_aidb_stats(&self) -> (u64, u64, u64, u64) {
        self.multi.aggregate_aidb_storage_stats()
    }

    /// Backup all local Raft group databases using AiDb's BackupManager.
    pub fn create_backup(&self, backup_base_dir: &std::path::Path) -> Result<Vec<String>> {
        let results = self
            .multi
            .storage()
            .backup_all_groups(backup_base_dir)
            .map_err(|e| AikvError::Persistence(format!("Cluster backup failed: {}", e)))?;
        Ok(results.into_iter().map(|(_, id)| id).collect())
    }

    fn unsup(cmd: &'static str) -> AikvError {
        AikvError::Storage(format!(
            "CLUSTER Raft engine: {} not supported (limited keyspace API)",
            cmd
        ))
    }
}
