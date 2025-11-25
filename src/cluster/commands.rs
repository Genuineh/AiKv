//! Cluster commands implementation.
//!
//! This module implements Redis Cluster protocol commands,
//! mapping them to AiDb's MultiRaft API.

use crate::cluster::router::SlotRouter;
use crate::error::{AikvError, Result};
use crate::protocol::RespValue;
use bytes::Bytes;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// Total number of slots in Redis Cluster (16384)
const TOTAL_SLOTS: u16 = 16384;
/// Total slots as usize for vector indexing
const TOTAL_SLOTS_USIZE: usize = 16384;

/// Slot state enumeration for CLUSTER SETSLOT command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotState {
    /// Slot is in normal state, assigned to a node
    Normal,
    /// Slot is being migrated out from this node
    Migrating,
    /// Slot is being imported to this node
    Importing,
}

/// Node information for cluster management.
#[derive(Debug, Clone)]
pub struct NodeInfo {
    /// Node ID (40-character hex string internally stored as u64)
    pub id: u64,
    /// Node address (ip:port)
    pub addr: String,
    /// Cluster bus port (typically data port + 10000)
    pub cluster_port: u16,
    /// Whether this node is marked as a master
    pub is_master: bool,
    /// Whether this node is connected
    pub is_connected: bool,
}

impl NodeInfo {
    /// Create a new NodeInfo with the given id and address.
    pub fn new(id: u64, addr: String) -> Self {
        // Parse address to extract port and calculate cluster port
        let cluster_port = if let Some(port_str) = addr.split(':').next_back() {
            port_str.parse::<u16>().unwrap_or(6379) + 10000
        } else {
            16379
        };
        Self {
            id,
            addr,
            cluster_port,
            is_master: true,
            is_connected: true,
        }
    }
}

/// Cluster state management.
#[derive(Debug, Default)]
pub struct ClusterState {
    /// Known nodes in the cluster (node_id -> NodeInfo)
    pub nodes: HashMap<u64, NodeInfo>,
    /// Slot assignments (slot -> node_id)
    pub slot_assignments: Vec<Option<u64>>,
    /// Slot states for migration (slot -> state)
    pub slot_states: HashMap<u16, SlotState>,
    /// Migration targets (slot -> target_node_id) for MIGRATING/IMPORTING
    pub migration_targets: HashMap<u16, u64>,
    /// Current cluster epoch
    pub config_epoch: u64,
}

impl ClusterState {
    /// Create a new ClusterState with default values.
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            slot_assignments: vec![None; TOTAL_SLOTS_USIZE],
            slot_states: HashMap::new(),
            migration_targets: HashMap::new(),
            config_epoch: 0,
        }
    }

    /// Count the number of assigned slots.
    pub fn assigned_slots_count(&self) -> usize {
        self.slot_assignments.iter().filter(|s| s.is_some()).count()
    }

    /// Check if all slots are assigned.
    pub fn all_slots_assigned(&self) -> bool {
        self.assigned_slots_count() == TOTAL_SLOTS_USIZE
    }
}

/// Cluster commands handler.
///
/// Implements Redis Cluster protocol commands:
/// - `CLUSTER KEYSLOT` - Calculate slot for a key
/// - `CLUSTER INFO` - Get cluster information
/// - `CLUSTER NODES` - Get cluster nodes
/// - `CLUSTER SLOTS` - Get slot-to-node mapping
/// - `CLUSTER MYID` - Get current node ID
/// - `CLUSTER MEET` - Add a node to the cluster
/// - `CLUSTER FORGET` - Remove a node from the cluster
/// - `CLUSTER ADDSLOTS` - Assign slots to this node
/// - `CLUSTER DELSLOTS` - Remove slot assignments
/// - `CLUSTER SETSLOT` - Set slot state (NODE/MIGRATING/IMPORTING)
pub struct ClusterCommands {
    router: SlotRouter,
    node_id: Option<u64>,
    /// Shared cluster state
    state: Arc<RwLock<ClusterState>>,
}

impl ClusterCommands {
    /// Create a new ClusterCommands handler.
    pub fn new() -> Self {
        Self {
            router: SlotRouter::new(),
            node_id: None,
            state: Arc::new(RwLock::new(ClusterState::new())),
        }
    }

    /// Create a new ClusterCommands handler with a node ID.
    ///
    /// This is used to set the node ID for commands like CLUSTER MYID.
    pub fn with_node_id(node_id: u64) -> Self {
        let state = Arc::new(RwLock::new(ClusterState::new()));
        // Add self as a node
        {
            let mut state_guard = state.write().unwrap();
            let node_info = NodeInfo::new(node_id, "127.0.0.1:6379".to_string());
            state_guard.nodes.insert(node_id, node_info);
        }
        Self {
            router: SlotRouter::new(),
            node_id: Some(node_id),
            state,
        }
    }

    /// Create a new ClusterCommands handler with shared state.
    ///
    /// This allows multiple handlers to share the same cluster state.
    pub fn with_shared_state(node_id: Option<u64>, state: Arc<RwLock<ClusterState>>) -> Self {
        Self {
            router: SlotRouter::new(),
            node_id,
            state,
        }
    }

    /// Get the shared cluster state.
    pub fn state(&self) -> Arc<RwLock<ClusterState>> {
        Arc::clone(&self.state)
    }

    /// Execute a CLUSTER command.
    ///
    /// # Arguments
    ///
    /// * `args` - Command arguments (subcommand and its arguments)
    ///
    /// # Returns
    ///
    /// The command result as a RespValue
    pub fn execute(&self, args: &[Bytes]) -> Result<RespValue> {
        if args.is_empty() {
            return Err(AikvError::WrongArgCount("CLUSTER".to_string()));
        }

        let subcommand = String::from_utf8_lossy(&args[0]).to_uppercase();
        match subcommand.as_str() {
            "KEYSLOT" => self.keyslot(&args[1..]),
            "INFO" => self.info(&args[1..]),
            "NODES" => self.nodes(&args[1..]),
            "SLOTS" => self.slots(&args[1..]),
            "MYID" => self.myid(&args[1..]),
            "MEET" => self.meet(&args[1..]),
            "FORGET" => self.forget(&args[1..]),
            "ADDSLOTS" => self.addslots(&args[1..]),
            "DELSLOTS" => self.delslots(&args[1..]),
            "SETSLOT" => self.setslot(&args[1..]),
            "HELP" => self.help(),
            _ => Err(AikvError::InvalidCommand(format!(
                "Unknown CLUSTER subcommand: {}",
                subcommand
            ))),
        }
    }

    /// CLUSTER KEYSLOT key
    ///
    /// Returns the hash slot of the specified key.
    ///
    /// # Arguments
    ///
    /// * `args` - Should contain exactly one argument: the key
    ///
    /// # Returns
    ///
    /// An integer representing the slot number (0-16383)
    fn keyslot(&self, args: &[Bytes]) -> Result<RespValue> {
        if args.len() != 1 {
            return Err(AikvError::WrongArgCount("CLUSTER KEYSLOT".to_string()));
        }

        let key = &args[0];
        let slot = self.router.key_to_slot(key);

        Ok(RespValue::Integer(slot as i64))
    }

    /// CLUSTER INFO
    ///
    /// Returns information about the cluster state.
    fn info(&self, _args: &[Bytes]) -> Result<RespValue> {
        let state = self.state.read().unwrap();

        let assigned_slots = state.assigned_slots_count();
        let cluster_state = if state.all_slots_assigned() && !state.nodes.is_empty() {
            "ok"
        } else {
            "fail"
        };
        let known_nodes = state.nodes.len();
        // Count nodes with assigned slots (masters)
        let cluster_size = state
            .slot_assignments
            .iter()
            .filter_map(|s| *s)
            .collect::<std::collections::HashSet<_>>()
            .len();

        let info = format!(
            "\
cluster_state:{}\r\n\
cluster_slots_assigned:{}\r\n\
cluster_slots_ok:{}\r\n\
cluster_slots_pfail:0\r\n\
cluster_slots_fail:0\r\n\
cluster_known_nodes:{}\r\n\
cluster_size:{}\r\n\
cluster_current_epoch:{}\r\n\
cluster_my_epoch:{}\r\n\
cluster_stats_messages_sent:0\r\n\
cluster_stats_messages_received:0\r\n",
            cluster_state,
            assigned_slots,
            assigned_slots,
            known_nodes.max(1), // At least 1 (self)
            cluster_size,
            state.config_epoch,
            state.config_epoch,
        );

        Ok(RespValue::bulk_string(Bytes::from(info)))
    }

    /// CLUSTER NODES
    ///
    /// Returns the cluster nodes information in Redis format.
    fn nodes(&self, _args: &[Bytes]) -> Result<RespValue> {
        let state = self.state.read().unwrap();
        let my_node_id = self.node_id.unwrap_or(0);
        let mut output = String::new();

        // Build slot ranges for each node
        let mut node_slots: HashMap<u64, Vec<(u16, u16)>> = HashMap::new();
        let mut current_start: Option<u16> = None;
        let mut current_node: Option<u64> = None;

        for (slot, &node) in state.slot_assignments.iter().enumerate() {
            let slot = slot as u16;
            match (current_start, current_node, node) {
                (Some(_start), Some(curr), Some(n)) if curr == n => {
                    // Continue current range
                }
                (Some(start), Some(curr), _) => {
                    // End current range
                    node_slots.entry(curr).or_default().push((start, slot - 1));
                    current_start = node.map(|_| slot);
                    current_node = node;
                }
                (None, None, Some(n)) => {
                    current_start = Some(slot);
                    current_node = Some(n);
                }
                _ => {
                    current_start = node.map(|_| slot);
                    current_node = node;
                }
            }
        }
        // Handle last range
        if let (Some(start), Some(curr)) = (current_start, current_node) {
            node_slots
                .entry(curr)
                .or_default()
                .push((start, TOTAL_SLOTS - 1));
        }

        // If no nodes in state, output self
        if state.nodes.is_empty() {
            output.push_str(&format!(
                "{:040x} 127.0.0.1:6379@16379 myself,master - 0 0 0 connected\r\n",
                my_node_id
            ));
        } else {
            for (node_id, info) in &state.nodes {
                let myself = if *node_id == my_node_id {
                    "myself,"
                } else {
                    ""
                };
                let role = if info.is_master { "master" } else { "slave" };
                let status = if info.is_connected {
                    "connected"
                } else {
                    "disconnected"
                };

                // Format slots
                let slots_str = node_slots
                    .get(node_id)
                    .map(|ranges| {
                        ranges
                            .iter()
                            .map(|(start, end)| {
                                if start == end {
                                    format!("{}", start)
                                } else {
                                    format!("{}-{}", start, end)
                                }
                            })
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .unwrap_or_default();

                // Format: <node-id> <ip:port@cluster-port> <flags> <master-id> <ping-sent> <pong-recv> <config-epoch> <link-state> <slot> ...
                output.push_str(&format!(
                    "{:040x} {}@{} {}{} - 0 0 {} {} {}\r\n",
                    node_id,
                    info.addr,
                    info.cluster_port,
                    myself,
                    role,
                    state.config_epoch,
                    status,
                    slots_str
                ));
            }
        }

        Ok(RespValue::bulk_string(Bytes::from(output)))
    }

    /// CLUSTER SLOTS
    ///
    /// Returns the slot-to-node mapping.
    fn slots(&self, _args: &[Bytes]) -> Result<RespValue> {
        let state = self.state.read().unwrap();
        let mut result = Vec::new();

        // Group consecutive slots assigned to the same node
        let mut ranges: Vec<(u16, u16, u64)> = Vec::new();
        let mut current_start: Option<u16> = None;
        let mut current_node: Option<u64> = None;

        for (slot, &node) in state.slot_assignments.iter().enumerate() {
            let slot = slot as u16;
            match (current_start, current_node, node) {
                (Some(_), Some(curr), Some(n)) if curr == n => {
                    // Continue current range
                }
                (Some(start), Some(curr), _) => {
                    // End current range and push
                    ranges.push((start, slot - 1, curr));
                    current_start = node.map(|_| slot);
                    current_node = node;
                }
                (None, None, Some(n)) => {
                    current_start = Some(slot);
                    current_node = Some(n);
                }
                _ => {
                    current_start = node.map(|_| slot);
                    current_node = node;
                }
            }
        }
        // Handle last range
        if let (Some(start), Some(curr)) = (current_start, current_node) {
            ranges.push((start, TOTAL_SLOTS - 1, curr));
        }

        // Build RESP response for each range
        for (start, end, node_id) in ranges {
            let node_info = state.nodes.get(&node_id);
            let (ip, port) = if let Some(info) = node_info {
                let parts: Vec<&str> = info.addr.split(':').collect();
                let ip = parts.first().unwrap_or(&"127.0.0.1").to_string();
                let port = parts
                    .get(1)
                    .and_then(|p| p.parse::<i64>().ok())
                    .unwrap_or(6379);
                (ip, port)
            } else {
                ("127.0.0.1".to_string(), 6379)
            };

            // Format: [start, end, [ip, port, node_id], ...]
            let node_entry = RespValue::Array(Some(vec![
                RespValue::bulk_string(Bytes::from(ip)),
                RespValue::Integer(port),
                RespValue::bulk_string(Bytes::from(format!("{:040x}", node_id))),
            ]));

            let slot_entry = RespValue::Array(Some(vec![
                RespValue::Integer(start as i64),
                RespValue::Integer(end as i64),
                node_entry,
            ]));

            result.push(slot_entry);
        }

        Ok(RespValue::Array(Some(result)))
    }

    /// CLUSTER MYID
    ///
    /// Returns the current node's ID.
    fn myid(&self, _args: &[Bytes]) -> Result<RespValue> {
        let node_id = self.node_id.unwrap_or(0);
        Ok(RespValue::bulk_string(Bytes::from(format!(
            "{:040x}",
            node_id
        ))))
    }

    /// CLUSTER MEET ip port [cluster-port]
    ///
    /// Add a node to the cluster by specifying its address.
    ///
    /// # Arguments
    ///
    /// * `args` - Should contain at least: ip, port. Optionally: cluster-port
    ///
    /// # Returns
    ///
    /// OK on success
    fn meet(&self, args: &[Bytes]) -> Result<RespValue> {
        if args.len() < 2 {
            return Err(AikvError::WrongArgCount("CLUSTER MEET".to_string()));
        }

        let ip = String::from_utf8_lossy(&args[0]).to_string();
        let port = String::from_utf8_lossy(&args[1])
            .parse::<u16>()
            .map_err(|_| AikvError::InvalidArgument("Invalid port number".to_string()))?;

        let cluster_port = if args.len() > 2 {
            String::from_utf8_lossy(&args[2])
                .parse::<u16>()
                .map_err(|_| {
                    AikvError::InvalidArgument("Invalid cluster port number".to_string())
                })?
        } else {
            port + 10000
        };

        let addr = format!("{}:{}", ip, port);

        // Generate a node ID based on address hash
        // In a real implementation, this would use the node's actual ID
        let node_id = {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut hasher = DefaultHasher::new();
            addr.hash(&mut hasher);
            hasher.finish()
        };

        // Add node to cluster state
        let mut state = self.state.write().unwrap();
        let mut node_info = NodeInfo::new(node_id, addr);
        node_info.cluster_port = cluster_port;
        state.nodes.insert(node_id, node_info);
        state.config_epoch += 1;

        Ok(RespValue::simple_string("OK"))
    }

    /// CLUSTER FORGET node-id
    ///
    /// Remove a node from the cluster.
    ///
    /// # Arguments
    ///
    /// * `args` - Should contain exactly one argument: the node ID (40-char hex)
    ///
    /// # Returns
    ///
    /// OK on success, error if node not found or is self
    fn forget(&self, args: &[Bytes]) -> Result<RespValue> {
        if args.len() != 1 {
            return Err(AikvError::WrongArgCount("CLUSTER FORGET".to_string()));
        }

        let node_id_str = String::from_utf8_lossy(&args[0]).to_string();
        let node_id = u64::from_str_radix(&node_id_str, 16)
            .map_err(|_| AikvError::InvalidArgument("Invalid node ID".to_string()))?;

        // Cannot forget self
        if Some(node_id) == self.node_id {
            return Err(AikvError::InvalidArgument(
                "I tried hard but I can't forget myself".to_string(),
            ));
        }

        let mut state = self.state.write().unwrap();

        // Check if node exists
        if !state.nodes.contains_key(&node_id) {
            return Err(AikvError::InvalidArgument(format!(
                "Unknown node {}",
                node_id_str
            )));
        }

        // Remove the node
        state.nodes.remove(&node_id);

        // Remove any slot assignments to this node
        for slot in state.slot_assignments.iter_mut() {
            if *slot == Some(node_id) {
                *slot = None;
            }
        }

        state.config_epoch += 1;

        Ok(RespValue::simple_string("OK"))
    }

    /// CLUSTER ADDSLOTS slot [slot ...]
    ///
    /// Assign slots to the current node.
    ///
    /// # Arguments
    ///
    /// * `args` - One or more slot numbers to assign
    ///
    /// # Returns
    ///
    /// OK on success
    fn addslots(&self, args: &[Bytes]) -> Result<RespValue> {
        if args.is_empty() {
            return Err(AikvError::WrongArgCount("CLUSTER ADDSLOTS".to_string()));
        }

        let my_node_id = self.node_id.ok_or_else(|| {
            AikvError::InvalidCommand("Node ID not set for this cluster node".to_string())
        })?;

        // Parse and validate all slots first
        let mut slots_to_add = Vec::new();
        for arg in args {
            let slot = String::from_utf8_lossy(arg)
                .parse::<u16>()
                .map_err(|_| AikvError::InvalidArgument("Invalid slot number".to_string()))?;

            if slot >= TOTAL_SLOTS {
                return Err(AikvError::InvalidArgument(format!(
                    "Invalid slot {} (out of range 0-{})",
                    slot,
                    TOTAL_SLOTS - 1
                )));
            }
            slots_to_add.push(slot);
        }

        let mut state = self.state.write().unwrap();

        // Check if any slot is already assigned
        for &slot in &slots_to_add {
            if let Some(assigned_to) = state.slot_assignments[slot as usize] {
                if assigned_to != my_node_id {
                    return Err(AikvError::InvalidArgument(format!(
                        "Slot {} is already busy",
                        slot
                    )));
                }
            }
        }

        // Assign all slots
        for slot in slots_to_add {
            state.slot_assignments[slot as usize] = Some(my_node_id);
        }
        state.config_epoch += 1;

        Ok(RespValue::simple_string("OK"))
    }

    /// CLUSTER DELSLOTS slot [slot ...]
    ///
    /// Remove slot assignments from the current node.
    ///
    /// # Arguments
    ///
    /// * `args` - One or more slot numbers to remove
    ///
    /// # Returns
    ///
    /// OK on success
    fn delslots(&self, args: &[Bytes]) -> Result<RespValue> {
        if args.is_empty() {
            return Err(AikvError::WrongArgCount("CLUSTER DELSLOTS".to_string()));
        }

        let my_node_id = self.node_id;

        // Parse and validate all slots first
        let mut slots_to_del = Vec::new();
        for arg in args {
            let slot = String::from_utf8_lossy(arg)
                .parse::<u16>()
                .map_err(|_| AikvError::InvalidArgument("Invalid slot number".to_string()))?;

            if slot >= TOTAL_SLOTS {
                return Err(AikvError::InvalidArgument(format!(
                    "Invalid slot {} (out of range 0-{})",
                    slot,
                    TOTAL_SLOTS - 1
                )));
            }
            slots_to_del.push(slot);
        }

        let mut state = self.state.write().unwrap();

        // Check if slots are assigned to this node (or unassigned)
        for &slot in &slots_to_del {
            if let Some(assigned_to) = state.slot_assignments[slot as usize] {
                if my_node_id.is_some() && Some(assigned_to) != my_node_id {
                    return Err(AikvError::InvalidArgument(format!(
                        "Slot {} is not owned by this node",
                        slot
                    )));
                }
            }
        }

        // Remove all slot assignments
        for slot in slots_to_del {
            state.slot_assignments[slot as usize] = None;
            // Also clear any migration state
            state.slot_states.remove(&slot);
            state.migration_targets.remove(&slot);
        }
        state.config_epoch += 1;

        Ok(RespValue::simple_string("OK"))
    }

    /// CLUSTER SETSLOT slot IMPORTING|MIGRATING|NODE|STABLE [node-id]
    ///
    /// Set slot state for migration or assign to a node.
    ///
    /// # Arguments
    ///
    /// * `args` - slot, subcommand (IMPORTING/MIGRATING/NODE/STABLE), and optionally node-id
    ///
    /// # Returns
    ///
    /// OK on success
    fn setslot(&self, args: &[Bytes]) -> Result<RespValue> {
        if args.len() < 2 {
            return Err(AikvError::WrongArgCount("CLUSTER SETSLOT".to_string()));
        }

        let slot = String::from_utf8_lossy(&args[0])
            .parse::<u16>()
            .map_err(|_| AikvError::InvalidArgument("Invalid slot number".to_string()))?;

        if slot >= TOTAL_SLOTS {
            return Err(AikvError::InvalidArgument(format!(
                "Invalid slot {} (out of range 0-{})",
                slot,
                TOTAL_SLOTS - 1
            )));
        }

        let subcommand = String::from_utf8_lossy(&args[1]).to_uppercase();

        match subcommand.as_str() {
            "IMPORTING" => {
                // CLUSTER SETSLOT <slot> IMPORTING <node-id>
                // Set slot as importing from another node
                if args.len() < 3 {
                    return Err(AikvError::WrongArgCount(
                        "CLUSTER SETSLOT IMPORTING".to_string(),
                    ));
                }
                let source_node_id_str = String::from_utf8_lossy(&args[2]).to_string();
                let source_node_id = u64::from_str_radix(&source_node_id_str, 16)
                    .map_err(|_| AikvError::InvalidArgument("Invalid node ID".to_string()))?;

                let mut state = self.state.write().unwrap();
                state.slot_states.insert(slot, SlotState::Importing);
                state.migration_targets.insert(slot, source_node_id);
                state.config_epoch += 1;

                Ok(RespValue::simple_string("OK"))
            }
            "MIGRATING" => {
                // CLUSTER SETSLOT <slot> MIGRATING <node-id>
                // Set slot as migrating to another node
                if args.len() < 3 {
                    return Err(AikvError::WrongArgCount(
                        "CLUSTER SETSLOT MIGRATING".to_string(),
                    ));
                }
                let target_node_id_str = String::from_utf8_lossy(&args[2]).to_string();
                let target_node_id = u64::from_str_radix(&target_node_id_str, 16)
                    .map_err(|_| AikvError::InvalidArgument("Invalid node ID".to_string()))?;

                let mut state = self.state.write().unwrap();
                state.slot_states.insert(slot, SlotState::Migrating);
                state.migration_targets.insert(slot, target_node_id);
                state.config_epoch += 1;

                Ok(RespValue::simple_string("OK"))
            }
            "NODE" => {
                // CLUSTER SETSLOT <slot> NODE <node-id>
                // Assign slot to a specific node
                if args.len() < 3 {
                    return Err(AikvError::WrongArgCount("CLUSTER SETSLOT NODE".to_string()));
                }
                let target_node_id_str = String::from_utf8_lossy(&args[2]).to_string();
                let target_node_id = u64::from_str_radix(&target_node_id_str, 16)
                    .map_err(|_| AikvError::InvalidArgument("Invalid node ID".to_string()))?;

                let mut state = self.state.write().unwrap();

                // Check if target node is known
                if !state.nodes.contains_key(&target_node_id)
                    && self.node_id != Some(target_node_id)
                {
                    return Err(AikvError::InvalidArgument(format!(
                        "Unknown node {}",
                        target_node_id_str
                    )));
                }

                // Assign the slot to the node
                state.slot_assignments[slot as usize] = Some(target_node_id);
                // Clear migration state
                state.slot_states.remove(&slot);
                state.migration_targets.remove(&slot);
                state.config_epoch += 1;

                Ok(RespValue::simple_string("OK"))
            }
            "STABLE" => {
                // CLUSTER SETSLOT <slot> STABLE
                // Clear migration state, slot remains assigned to current node
                let mut state = self.state.write().unwrap();
                state.slot_states.remove(&slot);
                state.migration_targets.remove(&slot);
                state.config_epoch += 1;

                Ok(RespValue::simple_string("OK"))
            }
            _ => Err(AikvError::InvalidArgument(format!(
                "Unknown SETSLOT subcommand: {}",
                subcommand
            ))),
        }
    }

    /// CLUSTER HELP
    ///
    /// Returns help text for CLUSTER commands.
    fn help(&self) -> Result<RespValue> {
        let help_lines = vec![
            RespValue::bulk_string(Bytes::from("CLUSTER KEYSLOT <key>")),
            RespValue::bulk_string(Bytes::from("    Return the hash slot for <key>.")),
            RespValue::bulk_string(Bytes::from("CLUSTER INFO")),
            RespValue::bulk_string(Bytes::from("    Return information about the cluster.")),
            RespValue::bulk_string(Bytes::from("CLUSTER NODES")),
            RespValue::bulk_string(Bytes::from(
                "    Return information about the cluster nodes.",
            )),
            RespValue::bulk_string(Bytes::from("CLUSTER SLOTS")),
            RespValue::bulk_string(Bytes::from(
                "    Return information about slot-to-node mapping.",
            )),
            RespValue::bulk_string(Bytes::from("CLUSTER MYID")),
            RespValue::bulk_string(Bytes::from("    Return the node ID.")),
            RespValue::bulk_string(Bytes::from("CLUSTER MEET <ip> <port> [<bus-port>]")),
            RespValue::bulk_string(Bytes::from("    Add a node to the cluster.")),
            RespValue::bulk_string(Bytes::from("CLUSTER FORGET <node-id>")),
            RespValue::bulk_string(Bytes::from("    Remove a node from the cluster.")),
            RespValue::bulk_string(Bytes::from("CLUSTER ADDSLOTS <slot> [<slot> ...]")),
            RespValue::bulk_string(Bytes::from("    Assign slots to this node.")),
            RespValue::bulk_string(Bytes::from("CLUSTER DELSLOTS <slot> [<slot> ...]")),
            RespValue::bulk_string(Bytes::from("    Remove slot assignments.")),
            RespValue::bulk_string(Bytes::from(
                "CLUSTER SETSLOT <slot> IMPORTING|MIGRATING|NODE|STABLE [<node-id>]",
            )),
            RespValue::bulk_string(Bytes::from("    Set slot state or assign to node.")),
        ];

        Ok(RespValue::Array(Some(help_lines)))
    }

    /// Generate a -MOVED error response.
    ///
    /// This is used when a client sends a command for a key that belongs
    /// to a different node in the cluster.
    ///
    /// # Arguments
    ///
    /// * `slot` - The slot number the key belongs to
    /// * `addr` - The address of the node that owns the slot (e.g., "127.0.0.1:6379")
    ///
    /// # Returns
    ///
    /// A RESP error value with the MOVED redirect
    pub fn moved_error(slot: u16, addr: &str) -> RespValue {
        RespValue::Error(format!("MOVED {} {}", slot, addr))
    }

    /// Generate an -ASK error response.
    ///
    /// This is used during slot migration when a key is being moved
    /// from one node to another.
    ///
    /// # Arguments
    ///
    /// * `slot` - The slot number the key belongs to
    /// * `addr` - The address of the target node
    ///
    /// # Returns
    ///
    /// A RESP error value with the ASK redirect
    pub fn ask_error(slot: u16, addr: &str) -> RespValue {
        RespValue::Error(format!("ASK {} {}", slot, addr))
    }

    /// Check if a key should be redirected to another node.
    ///
    /// # Arguments
    ///
    /// * `key` - The key to check
    /// * `local_slots` - The slots owned by this node (if available)
    ///
    /// # Returns
    ///
    /// None if the key should be handled locally, or Some(slot, addr) if redirected
    #[allow(unused_variables)]
    pub fn check_redirect(&self, key: &[u8], local_slots: &[bool]) -> Option<(u16, String)> {
        let slot = self.router.key_to_slot(key);

        // TODO: Implement actual redirect logic when cluster routing is available
        #[cfg(feature = "cluster")]
        {
            if let Some(addr) = self.router.get_slot_leader_address(slot) {
                return Some((slot, addr));
            }
        }

        // For now, no redirect needed
        None
    }
}

impl Default for ClusterCommands {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cluster_keyslot() {
        let cmd = ClusterCommands::new();

        // Test KEYSLOT command
        let result = cmd.execute(&[Bytes::from("KEYSLOT"), Bytes::from("foo")]);
        assert!(result.is_ok());

        if let Ok(RespValue::Integer(slot)) = result {
            assert!((0..16384).contains(&slot));
        } else {
            panic!("Expected integer response");
        }
    }

    #[test]
    fn test_cluster_keyslot_hash_tag() {
        let cmd = ClusterCommands::new();

        // Keys with hash tags should return valid slots
        let result1 = cmd.execute(&[Bytes::from("KEYSLOT"), Bytes::from("{user}name")]);
        let result2 = cmd.execute(&[Bytes::from("KEYSLOT"), Bytes::from("{user}age")]);

        let slot1 = match result1 {
            Ok(RespValue::Integer(s)) => s,
            _ => panic!("Expected integer"),
        };
        let slot2 = match result2 {
            Ok(RespValue::Integer(s)) => s,
            _ => panic!("Expected integer"),
        };

        // Both slots should be in valid range
        assert!((0..16384).contains(&slot1));
        assert!((0..16384).contains(&slot2));

        // Note: Hash tag handling depends on AiDb implementation when cluster feature is enabled
        // When not using cluster feature, our fallback implementation handles hash tags
        #[cfg(not(feature = "cluster"))]
        {
            assert_eq!(slot1, slot2);
        }
    }

    #[test]
    fn test_cluster_info() {
        let cmd = ClusterCommands::new();
        let result = cmd.execute(&[Bytes::from("INFO")]);
        assert!(result.is_ok());
    }

    #[test]
    fn test_cluster_nodes() {
        let cmd = ClusterCommands::new();
        let result = cmd.execute(&[Bytes::from("NODES")]);
        assert!(result.is_ok());
    }

    #[test]
    fn test_cluster_myid() {
        let cmd = ClusterCommands::new();
        let result = cmd.execute(&[Bytes::from("MYID")]);
        assert!(result.is_ok());
    }

    #[test]
    fn test_cluster_help() {
        let cmd = ClusterCommands::new();
        let result = cmd.execute(&[Bytes::from("HELP")]);
        assert!(result.is_ok());
    }

    #[test]
    fn test_cluster_unknown_subcommand() {
        let cmd = ClusterCommands::new();
        let result = cmd.execute(&[Bytes::from("UNKNOWN")]);
        assert!(result.is_err());
    }

    #[test]
    fn test_moved_error() {
        let error = ClusterCommands::moved_error(12345, "127.0.0.1:7000");
        if let RespValue::Error(msg) = error {
            assert!(msg.contains("MOVED"));
            assert!(msg.contains("12345"));
            assert!(msg.contains("127.0.0.1:7000"));
        } else {
            panic!("Expected error response");
        }
    }

    #[test]
    fn test_ask_error() {
        let error = ClusterCommands::ask_error(12345, "127.0.0.1:7001");
        if let RespValue::Error(msg) = error {
            assert!(msg.contains("ASK"));
            assert!(msg.contains("12345"));
            assert!(msg.contains("127.0.0.1:7001"));
        } else {
            panic!("Expected error response");
        }
    }

    #[test]
    fn test_cluster_meet() {
        let cmd = ClusterCommands::with_node_id(1);

        // Test MEET command
        let result = cmd.execute(&[
            Bytes::from("MEET"),
            Bytes::from("192.168.1.100"),
            Bytes::from("6380"),
        ]);
        assert!(result.is_ok());

        // Verify node was added
        let state = cmd.state();
        let state = state.read().unwrap();
        assert!(state.nodes.len() >= 2); // Self + new node
    }

    #[test]
    fn test_cluster_meet_with_cluster_port() {
        let cmd = ClusterCommands::with_node_id(1);

        // Test MEET with explicit cluster port
        let result = cmd.execute(&[
            Bytes::from("MEET"),
            Bytes::from("192.168.1.100"),
            Bytes::from("6380"),
            Bytes::from("16380"),
        ]);
        assert!(result.is_ok());
    }

    #[test]
    fn test_cluster_meet_wrong_args() {
        let cmd = ClusterCommands::with_node_id(1);

        // Missing port
        let result = cmd.execute(&[Bytes::from("MEET"), Bytes::from("192.168.1.100")]);
        assert!(result.is_err());

        // No args
        let result = cmd.execute(&[Bytes::from("MEET")]);
        assert!(result.is_err());
    }

    #[test]
    fn test_cluster_forget() {
        let cmd = ClusterCommands::with_node_id(1);

        // First add a node
        cmd.execute(&[
            Bytes::from("MEET"),
            Bytes::from("192.168.1.100"),
            Bytes::from("6380"),
        ])
        .unwrap();

        // Get the node ID of the added node
        let node_id: u64 = {
            let state = cmd.state();
            let state = state.read().unwrap();
            *state.nodes.keys().find(|&&id| id != 1).unwrap()
        };

        // Forget the node
        let result = cmd.execute(&[
            Bytes::from("FORGET"),
            Bytes::from(format!("{:040x}", node_id)),
        ]);
        assert!(result.is_ok());

        // Verify node was removed
        let state = cmd.state();
        let state = state.read().unwrap();
        assert!(!state.nodes.contains_key(&node_id));
    }

    #[test]
    fn test_cluster_forget_self() {
        let cmd = ClusterCommands::with_node_id(1);

        // Try to forget self - should fail
        let result = cmd.execute(&[Bytes::from("FORGET"), Bytes::from(format!("{:040x}", 1u64))]);
        assert!(result.is_err());
    }

    #[test]
    fn test_cluster_forget_unknown() {
        let cmd = ClusterCommands::with_node_id(1);

        // Try to forget unknown node
        let result = cmd.execute(&[
            Bytes::from("FORGET"),
            Bytes::from("0000000000000000000000000000000000000999"),
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn test_cluster_addslots() {
        let cmd = ClusterCommands::with_node_id(1);

        // Add some slots
        let result = cmd.execute(&[
            Bytes::from("ADDSLOTS"),
            Bytes::from("0"),
            Bytes::from("1"),
            Bytes::from("2"),
        ]);
        assert!(result.is_ok());

        // Verify slots were added
        let state = cmd.state();
        let state = state.read().unwrap();
        assert_eq!(state.slot_assignments[0], Some(1));
        assert_eq!(state.slot_assignments[1], Some(1));
        assert_eq!(state.slot_assignments[2], Some(1));
    }

    #[test]
    fn test_cluster_addslots_already_assigned() {
        let cmd = ClusterCommands::with_node_id(1);

        // Add slot 0
        cmd.execute(&[Bytes::from("ADDSLOTS"), Bytes::from("0")])
            .unwrap();

        // Create another node and try to add the same slot
        let cmd2 = ClusterCommands::with_shared_state(Some(2), cmd.state());
        let result = cmd2.execute(&[Bytes::from("ADDSLOTS"), Bytes::from("0")]);
        assert!(result.is_err()); // Should fail - slot already busy
    }

    #[test]
    fn test_cluster_addslots_invalid_slot() {
        let cmd = ClusterCommands::with_node_id(1);

        // Try to add invalid slot
        let result = cmd.execute(&[Bytes::from("ADDSLOTS"), Bytes::from("99999")]);
        assert!(result.is_err());
    }

    #[test]
    fn test_cluster_delslots() {
        let cmd = ClusterCommands::with_node_id(1);

        // Add then delete slots
        cmd.execute(&[
            Bytes::from("ADDSLOTS"),
            Bytes::from("0"),
            Bytes::from("1"),
            Bytes::from("2"),
        ])
        .unwrap();

        let result = cmd.execute(&[Bytes::from("DELSLOTS"), Bytes::from("0"), Bytes::from("1")]);
        assert!(result.is_ok());

        // Verify slots were removed
        let state = cmd.state();
        let state = state.read().unwrap();
        assert_eq!(state.slot_assignments[0], None);
        assert_eq!(state.slot_assignments[1], None);
        assert_eq!(state.slot_assignments[2], Some(1)); // This one should still be assigned
    }

    #[test]
    fn test_cluster_setslot_node() {
        let cmd = ClusterCommands::with_node_id(1);

        // Assign slot 100 to node 1
        let result = cmd.execute(&[
            Bytes::from("SETSLOT"),
            Bytes::from("100"),
            Bytes::from("NODE"),
            Bytes::from(format!("{:040x}", 1u64)),
        ]);
        assert!(result.is_ok());

        // Verify slot was assigned
        let state = cmd.state();
        let state = state.read().unwrap();
        assert_eq!(state.slot_assignments[100], Some(1));
    }

    #[test]
    fn test_cluster_setslot_migrating() {
        let cmd = ClusterCommands::with_node_id(1);

        // Add a slot first
        cmd.execute(&[Bytes::from("ADDSLOTS"), Bytes::from("100")])
            .unwrap();

        // Set slot as migrating
        let target_node_id = 2u64;
        let result = cmd.execute(&[
            Bytes::from("SETSLOT"),
            Bytes::from("100"),
            Bytes::from("MIGRATING"),
            Bytes::from(format!("{:040x}", target_node_id)),
        ]);
        assert!(result.is_ok());

        // Verify migration state
        let state = cmd.state();
        let state = state.read().unwrap();
        assert_eq!(state.slot_states.get(&100), Some(&SlotState::Migrating));
        assert_eq!(state.migration_targets.get(&100), Some(&target_node_id));
    }

    #[test]
    fn test_cluster_setslot_importing() {
        let cmd = ClusterCommands::with_node_id(1);

        // Set slot as importing
        let source_node_id = 2u64;
        let result = cmd.execute(&[
            Bytes::from("SETSLOT"),
            Bytes::from("100"),
            Bytes::from("IMPORTING"),
            Bytes::from(format!("{:040x}", source_node_id)),
        ]);
        assert!(result.is_ok());

        // Verify import state
        let state = cmd.state();
        let state = state.read().unwrap();
        assert_eq!(state.slot_states.get(&100), Some(&SlotState::Importing));
        assert_eq!(state.migration_targets.get(&100), Some(&source_node_id));
    }

    #[test]
    fn test_cluster_setslot_stable() {
        let cmd = ClusterCommands::with_node_id(1);

        // Set up a migration first
        cmd.execute(&[Bytes::from("ADDSLOTS"), Bytes::from("100")])
            .unwrap();
        cmd.execute(&[
            Bytes::from("SETSLOT"),
            Bytes::from("100"),
            Bytes::from("MIGRATING"),
            Bytes::from(format!("{:040x}", 2u64)),
        ])
        .unwrap();

        // Clear migration with STABLE
        let result = cmd.execute(&[
            Bytes::from("SETSLOT"),
            Bytes::from("100"),
            Bytes::from("STABLE"),
        ]);
        assert!(result.is_ok());

        // Verify migration state was cleared
        let state = cmd.state();
        let state = state.read().unwrap();
        assert!(!state.slot_states.contains_key(&100));
        assert!(!state.migration_targets.contains_key(&100));
    }

    #[test]
    fn test_cluster_setslot_invalid_subcommand() {
        let cmd = ClusterCommands::with_node_id(1);

        let result = cmd.execute(&[
            Bytes::from("SETSLOT"),
            Bytes::from("100"),
            Bytes::from("INVALID"),
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn test_cluster_slots_after_addslots() {
        let cmd = ClusterCommands::with_node_id(1);

        // Add some slots
        cmd.execute(&[
            Bytes::from("ADDSLOTS"),
            Bytes::from("0"),
            Bytes::from("1"),
            Bytes::from("2"),
            Bytes::from("100"),
            Bytes::from("101"),
        ])
        .unwrap();

        // Get SLOTS response
        let result = cmd.execute(&[Bytes::from("SLOTS")]);
        assert!(result.is_ok());

        if let Ok(RespValue::Array(Some(slots))) = result {
            // Should have 2 ranges: 0-2 and 100-101
            assert_eq!(slots.len(), 2);
        } else {
            panic!("Expected array response");
        }
    }

    #[test]
    fn test_cluster_info_with_slots() {
        let cmd = ClusterCommands::with_node_id(1);

        // Add all slots (16384)
        {
            let state = cmd.state();
            let mut state = state.write().unwrap();
            for i in 0..16384u16 {
                state.slot_assignments[i as usize] = Some(1);
            }
        }

        // Check cluster info shows ok state
        let result = cmd.execute(&[Bytes::from("INFO")]);
        assert!(result.is_ok());

        if let Ok(RespValue::BulkString(Some(info))) = result {
            let info_str = String::from_utf8_lossy(&info);
            assert!(info_str.contains("cluster_state:ok"));
            assert!(info_str.contains("cluster_slots_assigned:16384"));
        } else {
            panic!("Expected bulk string response");
        }
    }

    #[test]
    fn test_cluster_nodes_format() {
        let cmd = ClusterCommands::with_node_id(1);

        // Add some slots
        cmd.execute(&[Bytes::from("ADDSLOTS"), Bytes::from("0"), Bytes::from("1")])
            .unwrap();

        // Get NODES response
        let result = cmd.execute(&[Bytes::from("NODES")]);
        assert!(result.is_ok());

        if let Ok(RespValue::BulkString(Some(nodes))) = result {
            let nodes_str = String::from_utf8_lossy(&nodes);
            // Should contain myself flag
            assert!(nodes_str.contains("myself"));
            // Should contain master flag
            assert!(nodes_str.contains("master"));
            // Should contain connected
            assert!(nodes_str.contains("connected"));
            // Should contain slot range
            assert!(nodes_str.contains("0-1"));
        } else {
            panic!("Expected bulk string response");
        }
    }
}
