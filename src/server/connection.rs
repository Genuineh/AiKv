use crate::command::CommandExecutor;
use crate::error::Result;
use crate::observability::Metrics;
use crate::protocol::{RespParser, RespValue};
use crate::server::monitor::MonitorBroadcaster;
use bytes::Bytes;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::select;
use tracing::{debug, info, warn};

static CLIENT_ID_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Commands that should not be broadcast to MONITOR clients.
/// These are typically internal, debugging, or replication commands.
const MONITOR_EXCLUDED_COMMANDS: &[&str] = &["MONITOR", "DEBUG", "SYNC", "PSYNC"];

/// Protocol version
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ProtocolVersion {
    Resp2,
    Resp3,
}

/// Connection mode
#[derive(Debug, Clone, Copy, PartialEq)]
enum ConnectionMode {
    Normal,
    Monitor,
}

/// Connection handler for a single client
pub struct Connection {
    stream: TcpStream,
    parser: RespParser,
    executor: CommandExecutor,
    protocol_version: ProtocolVersion,
    current_db: usize,
    client_id: usize,
    metrics: Option<Arc<Metrics>>,
    client_addr: String,
    monitor_broadcaster: Option<Arc<MonitorBroadcaster>>,
    mode: ConnectionMode,
    /// Last executed command name (for diagnostics when connection drops)
    last_command: Option<String>,
    #[cfg(feature = "cluster")]
    /// Redis ASKING one-shot flag for next command.
    allow_importing_slot_once: bool,
    /// Whether the connection is in READONLY mode.
    readonly_mode: bool,
}

impl Connection {
    /// Create a new connection handler.
    ///
    /// # Arguments
    /// * `stream` - The TCP stream for this connection
    /// * `executor` - Command executor for processing Redis commands
    /// * `metrics` - Optional metrics collector for connection statistics
    /// * `monitor_broadcaster` - Optional broadcaster for MONITOR command support.
    ///   If None, MONITOR command will return an error. This is typically None
    ///   only in unit tests or when MONITOR support is intentionally disabled.
    pub fn new(
        stream: TcpStream,
        executor: CommandExecutor,
        metrics: Option<Arc<Metrics>>,
        monitor_broadcaster: Option<Arc<MonitorBroadcaster>>,
    ) -> Self {
        let client_id = CLIENT_ID_COUNTER.fetch_add(1, Ordering::SeqCst);
        let peer_addr = stream
            .peer_addr()
            .map(|addr| addr.to_string())
            .unwrap_or_else(|_| "unknown".to_string());

        // Register client
        if let Err(e) = executor
            .server_commands()
            .register_client(client_id, peer_addr.clone())
        {
            warn!("Failed to register client: {}", e);
        }

        Self {
            stream,
            parser: RespParser::new(8192),
            executor,
            protocol_version: ProtocolVersion::Resp2, // Default to RESP2
            current_db: 0,                            // Default to database 0
            client_id,
            metrics,
            client_addr: peer_addr,
            monitor_broadcaster,
            mode: ConnectionMode::Normal,
            last_command: None,
            #[cfg(feature = "cluster")]
            allow_importing_slot_once: false,
            readonly_mode: false,
        }
    }

    /// Handle the connection using a state machine
    pub async fn handle(&mut self) -> Result<()> {
        loop {
            match self.mode {
                ConnectionMode::Normal => {
                    if !self.handle_normal_mode().await? {
                        break;
                    }
                }
                ConnectionMode::Monitor => {
                    if !self.handle_monitor_mode().await? {
                        break;
                    }
                }
            }
        }

        self.cleanup().await;
        Ok(())
    }

    /// Handle normal command mode. Returns false if connection should close.
    async fn handle_normal_mode(&mut self) -> Result<bool> {
        // Read data from the client
        let n = self
            .stream
            .read_buf(self.parser.buffer_mut())
            .await
            .map_err(|e: std::io::Error| {
                warn!(
                    client_id = self.client_id,
                    addr = %self.client_addr,
                    last_command = ?self.last_command.as_deref(),
                    error = %e,
                    "connection read error (client likely closed or network issue)"
                );
                crate::error::AikvError::from(e)
            })?;

        if n == 0 {
            info!(
                client_id = self.client_id,
                addr = %self.client_addr,
                "client closed connection (EOF)"
            );
            return Ok(false);
        }

        // Record bytes received
        if let Some(ref metrics) = self.metrics {
            metrics.connections.record_bytes_received(n as u64);
        }

        // Parse and process commands
        while let Some(value) = self.parser.parse()? {
            let response = self.process_command(value).await;
            self.write_response(response)
                .await
                .map_err(|e: crate::error::AikvError| {
                    warn!(
                        client_id = self.client_id,
                        addr = %self.client_addr,
                        last_command = ?self.last_command.as_deref(),
                        error = %e,
                        "connection write error (client may have closed or timeout)"
                    );
                    e
                })?;

            // Check if mode changed to monitor
            if self.mode == ConnectionMode::Monitor {
                return Ok(true);
            }
        }

        Ok(true)
    }

    /// Handle monitor mode - stream all commands to this client.
    /// Returns false if connection should close.
    async fn handle_monitor_mode(&mut self) -> Result<bool> {
        let broadcaster = match &self.monitor_broadcaster {
            Some(b) => b.clone(),
            None => {
                warn!("Monitor mode enabled but no broadcaster available");
                self.mode = ConnectionMode::Normal;
                return Ok(true);
            }
        };

        let mut receiver = broadcaster.subscribe();

        loop {
            select! {
                // Receive monitor messages
                msg = receiver.recv() => {
                    match msg {
                        Ok(monitor_msg) => {
                            // Format and send the monitor message
                            let formatted = monitor_msg.format();
                            let response = RespValue::simple_string(formatted);
                            if let Err(e) = self.write_response(response).await {
                                debug!("Monitor client write error: {}", e);
                                return Ok(false);
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            // We missed some messages due to slow reading
                            debug!("Monitor client {} lagged behind by {} messages", self.client_id, n);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            // Broadcaster closed
                            return Ok(false);
                        }
                    }
                }
                // Check for client input (QUIT, RESET, or disconnect)
                result = self.stream.read_buf(self.parser.buffer_mut()) => {
                    match result {
                        Ok(0) => {
                            // Client disconnected
                            broadcaster.unregister_monitor(self.client_id).await;
                            return Ok(false);
                        }
                        Ok(_) => {
                            // Client sent data - check for QUIT or RESET
                            while let Some(value) = self.parser.parse()? {
                                if let RespValue::Array(Some(arr)) = &value {
                                    if !arr.is_empty() {
                                        if let RespValue::BulkString(Some(cmd)) = &arr[0] {
                                            let command = String::from_utf8_lossy(cmd).to_uppercase();
                                            if command == "QUIT" {
                                                broadcaster.unregister_monitor(self.client_id).await;
                                                self.write_response(RespValue::ok()).await?;
                                                return Ok(false);
                                            } else if command == "RESET" {
                                                broadcaster.unregister_monitor(self.client_id).await;
                                                self.mode = ConnectionMode::Normal;
                                                self.write_response(RespValue::simple_string("RESET")).await?;
                                                return Ok(true);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            debug!("Monitor client read error: {}", e);
                            broadcaster.unregister_monitor(self.client_id).await;
                            return Ok(false);
                        }
                    }
                }
            }
        }
    }

    /// Cleanup on connection close
    async fn cleanup(&mut self) {
        // Unregister client
        if let Err(e) = self
            .executor
            .server_commands()
            .unregister_client(self.client_id)
        {
            warn!("Failed to unregister client: {}", e);
        }

        // Unregister from monitor if in monitor mode
        if self.mode == ConnectionMode::Monitor {
            if let Some(ref broadcaster) = self.monitor_broadcaster {
                broadcaster.unregister_monitor(self.client_id).await;
            }
        }
    }

    async fn process_command(&mut self, value: RespValue) -> RespValue {
        let start = Instant::now();

        match value {
            RespValue::Array(Some(arr)) if !arr.is_empty() => {
                // Extract command and arguments
                let command = match &arr[0] {
                    RespValue::BulkString(Some(cmd)) => String::from_utf8_lossy(cmd).to_string(),
                    _ => {
                        return RespValue::error("ERR invalid command format");
                    }
                };

                let command_upper = command.to_uppercase();
                self.last_command = Some(command_upper.clone());

                // Handle HELLO command for protocol version negotiation
                if command_upper == "HELLO" {
                    return self.handle_hello(&arr[1..]);
                }

                // Handle MONITOR command
                if command_upper == "MONITOR" {
                    return self.handle_monitor().await;
                }

                #[cfg(feature = "cluster")]
                if command_upper == "ASKING" {
                    self.allow_importing_slot_once = true;
                    debug!(
                        diag_event = "cluster_client_asking_marked",
                        client = %self.client_addr,
                        "ASKING accepted; next command may access importing slot"
                    );
                    return RespValue::ok();
                }

                let args: Vec<Bytes> = arr[1..]
                    .iter()
                    .filter_map(|v| match v {
                        RespValue::BulkString(Some(b)) => Some(b.clone()),
                        _ => None,
                    })
                    .collect();

                // Broadcast to monitors (except excluded internal/debugging commands)
                if !MONITOR_EXCLUDED_COMMANDS.contains(&command_upper.as_str()) {
                    self.broadcast_to_monitors(&command_upper, &args);
                }

                // Handle async CLUSTER commands before synchronous execution
                #[cfg(feature = "cluster")]
                if command_upper == "CLUSTER" && !args.is_empty() {
                    let subcommand = String::from_utf8_lossy(&args[0]).to_uppercase();
                    // These are async cluster management commands
                    if matches!(
                        subcommand.as_str(),
                        "MEET"
                            | "FORGET"
                            | "ADDSLOTS"
                            | "ADDSLOTSRANGE"
                            | "DELSLOTS"
                            | "SETSLOT"
                            | "REPLICATE"
                            | "ADDREPLICATION"
                            | "FAILOVER"
                            | "METARAFT"
                    ) {
                        if let Some(cluster_cmds) = self.executor.cluster_commands() {
                            let result = self
                                .handle_async_cluster_command(cluster_cmds, &subcommand, &args[1..])
                                .await;

                            // Record metrics
                            if let Some(ref metrics) = self.metrics {
                                let duration = start.elapsed();
                                match &result {
                                    Ok(_) => {
                                        metrics.commands.record_command(
                                            &format!("CLUSTER {}", subcommand),
                                            duration,
                                        );
                                        debug!(
                                            command = %format!("CLUSTER {}", subcommand),
                                            duration_us = duration.as_micros(),
                                            client = %self.client_addr,
                                            db = self.current_db,
                                            "Async cluster command executed"
                                        );
                                    }
                                    Err(_) => {
                                        metrics
                                            .commands
                                            .record_error(&format!("CLUSTER {}", subcommand));
                                    }
                                }
                            }

                            return match result {
                                Ok(resp) => resp,
                                Err(e) => {
                                    let cmd = format!("CLUSTER {}", subcommand);
                                    Self::log_command_error(&cmd, &self.client_addr, &e);
                                    Self::format_error_response(e)
                                }
                            };
                        } else {
                            return RespValue::error("ERR Cluster not initialized. Please initialize cluster node first.");
                        }
                    }
                }

                #[cfg(feature = "cluster")]
                if command_upper == "READONLY" {
                    self.readonly_mode = true;
                }
                #[cfg(feature = "cluster")]
                if command_upper == "READWRITE" {
                    self.readonly_mode = false;
                }

                // executor.execute() is synchronous and may block for an extended period
                // (e.g. INFO keyspace scans all MemTables). block_in_place allows the Tokio
                // runtime to continue scheduling other tasks on this worker thread while we wait.
                let result = tokio::task::block_in_place(|| {
                    self.executor
                        .execute(
                            &command,
                            &args,
                            &mut self.current_db,
                            self.client_id,
                            #[cfg(feature = "cluster")]
                            self.allow_importing_slot_once,
                            #[cfg(feature = "cluster")]
                            self.readonly_mode,
                        )
                });
                #[cfg(feature = "cluster")]
                {
                    self.allow_importing_slot_once = false;
                }

                let duration = start.elapsed();

                // Record metrics
                if let Some(ref metrics) = self.metrics {
                    match &result {
                        Ok(resp) => {
                            metrics.commands.record_command(&command, duration);
                            // Keyspace hit/miss for GET
                            if command.eq_ignore_ascii_case("GET") {
                                match resp {
                                    RespValue::BulkString(Some(_)) => metrics.memory.record_hit(),
                                    RespValue::BulkString(None) => metrics.memory.record_miss(),
                                    _ => {}
                                }
                            }
                            debug!(
                                command = %command,
                                duration_us = duration.as_micros(),
                                client = %self.client_addr,
                                db = self.current_db,
                                "Command executed"
                            );
                        }
                        Err(_) => {
                            metrics.commands.record_error(&command);
                        }
                    }
                }

                // Record slow query if over threshold (Redis SLOWLOG semantics)
                {
                    let args_str: Vec<String> = args
                        .iter()
                        .map(|b| String::from_utf8_lossy(b).into_owned())
                        .collect();
                    self.executor.server_commands().slow_query_log().record(
                        &command,
                        &args_str,
                        duration,
                        Some(self.client_addr.clone()),
                    );
                }

                match result {
                    Ok(resp) => resp,
                    Err(e) => {
                        Self::log_command_error(&command, &self.client_addr, &e);
                        Self::format_error_response(e)
                    }
                }
            }
            _ => RespValue::error("ERR invalid command format"),
        }
    }

    /// 可观测性：带固定 `diag_event` 字段，便于 Loki / export_logs.sh 过滤。
    fn log_command_error(command: &str, client_addr: &str, err: &crate::error::AikvError) {
        use crate::error::AikvError;
        match err {
            AikvError::Moved(slot, addr) => {
                debug!(
                    diag_event = "cluster_client_moved",
                    command = %command,
                    client = %client_addr,
                    slot = *slot,
                    target = %addr,
                    "returning MOVED to client"
                );
            }
            AikvError::Ask(slot, addr) => {
                debug!(
                    diag_event = "cluster_client_ask",
                    command = %command,
                    client = %client_addr,
                    slot = *slot,
                    target = %addr,
                    "returning ASK to client"
                );
            }
            AikvError::Storage(msg) => {
                warn!(
                    diag_event = "cluster_command_storage_err",
                    command = %command,
                    client = %client_addr,
                    error = %msg,
                    "command failed: storage"
                );
            }
            AikvError::Internal(msg) => {
                warn!(
                    diag_event = "cluster_command_internal_err",
                    command = %command,
                    client = %client_addr,
                    error = %msg,
                    "command failed: internal"
                );
            }
            AikvError::Persistence(msg) | AikvError::Protocol(msg) => {
                warn!(
                    diag_event = "cluster_command_io_protocol_err",
                    command = %command,
                    client = %client_addr,
                    error = %msg,
                    "command failed: persistence/protocol"
                );
            }
            _ => {
                debug!(
                    diag_event = "cluster_command_err_other",
                    command = %command,
                    client = %client_addr,
                    error = %err,
                    "command failed"
                );
            }
        }
    }

    /// Format an error into a RESP error response.
    ///
    /// Cluster-specific errors (MOVED, ASK, CROSSSLOT) have special formats
    /// that Redis clients expect.
    fn format_error_response(e: crate::error::AikvError) -> RespValue {
        use crate::error::AikvError;
        match e {
            // Cluster redirection errors - format without "ERR " prefix
            AikvError::Moved(slot, addr) => RespValue::error(format!("MOVED {} {}", slot, addr)),
            AikvError::Ask(slot, addr) => RespValue::error(format!("ASK {} {}", slot, addr)),
            AikvError::CrossSlot => {
                RespValue::error("CROSSSLOT Keys in request don't hash to the same slot")
            }
            // All other errors use the standard "ERR " prefix
            _ => RespValue::error(format!("ERR {}", e)),
        }
    }

    /// Broadcast command to all monitoring clients
    fn broadcast_to_monitors(&self, command: &str, args: &[Bytes]) {
        if let Some(ref broadcaster) = self.monitor_broadcaster {
            if broadcaster.has_monitors() {
                let args_str: Vec<String> = args
                    .iter()
                    .map(|b| String::from_utf8_lossy(b).to_string())
                    .collect();
                broadcaster.broadcast_command(
                    self.current_db,
                    &self.client_addr,
                    command,
                    &args_str,
                );
            }
        }
    }

    /// Handle async cluster commands
    #[cfg(feature = "cluster")]
    async fn handle_async_cluster_command(
        &self,
        cluster_cmds: &crate::cluster::ClusterCommands,
        subcommand: &str,
        args: &[Bytes],
    ) -> Result<RespValue> {
        use crate::error::AikvError;

        match subcommand {
            "MEET" => {
                // CLUSTER MEET ip port [node-id]
                if args.len() < 2 || args.len() > 3 {
                    return Err(AikvError::WrongArgCount("CLUSTER MEET".to_string()));
                }

                let ip = String::from_utf8_lossy(&args[0]).to_string();
                let port = String::from_utf8_lossy(&args[1])
                    .parse::<u16>()
                    .map_err(|_| AikvError::Invalid("Invalid port".to_string()))?;

                let node_id = if args.len() == 3 {
                    let id_str = String::from_utf8_lossy(&args[2]);
                    Some(
                        u64::from_str_radix(&id_str, 16)
                            .map_err(|_| AikvError::Invalid("Invalid node ID".to_string()))?,
                    )
                } else {
                    None
                };

                cluster_cmds.cluster_meet(ip, port, node_id).await
            }
            "FORGET" => {
                // CLUSTER FORGET node-id
                if args.len() != 1 {
                    return Err(AikvError::WrongArgCount("CLUSTER FORGET".to_string()));
                }

                let id_str = String::from_utf8_lossy(&args[0]);
                let node_id = u64::from_str_radix(&id_str, 16)
                    .map_err(|_| AikvError::Invalid("Invalid node ID".to_string()))?;

                cluster_cmds.cluster_forget(node_id).await
            }
            "ADDSLOTS" => {
                // CLUSTER ADDSLOTS slot [slot ...]
                if args.is_empty() {
                    return Err(AikvError::WrongArgCount("CLUSTER ADDSLOTS".to_string()));
                }

                let mut slots = Vec::new();
                for arg in args {
                    let slot = String::from_utf8_lossy(arg)
                        .parse::<u16>()
                        .map_err(|_| AikvError::Invalid("Invalid slot".to_string()))?;

                    if slot >= 16384 {
                        return Err(AikvError::Invalid(format!("Slot out of range: {}", slot)));
                    }
                    slots.push(slot);
                }

                cluster_cmds.cluster_addslots(slots).await
            }
            "ADDSLOTSRANGE" => {
                // CLUSTER ADDSLOTSRANGE start end [node_id]
                // Efficiently add a range of slots to the specified node (or current node if not specified)
                if args.len() < 2 || args.len() > 3 {
                    return Err(AikvError::WrongArgCount(
                        "CLUSTER ADDSLOTSRANGE".to_string(),
                    ));
                }

                let start = String::from_utf8_lossy(&args[0])
                    .parse::<u16>()
                    .map_err(|_| AikvError::Invalid("Invalid start slot".to_string()))?;
                let end = String::from_utf8_lossy(&args[1])
                    .parse::<u16>()
                    .map_err(|_| AikvError::Invalid("Invalid end slot".to_string()))?;

                if start > end || end >= 16384 {
                    return Err(AikvError::Invalid(format!(
                        "Invalid slot range: {}-{}",
                        start, end
                    )));
                }

                let target_node_id = if args.len() == 3 {
                    let id_str = String::from_utf8_lossy(&args[2]);
                    // Try decimal first, then hex
                    id_str
                        .parse::<u64>()
                        .or_else(|_| u64::from_str_radix(&id_str, 16))
                        .map_err(|_| AikvError::Invalid("Invalid node ID".to_string()))?
                } else {
                    0 // 0 means current node
                };

                cluster_cmds
                    .cluster_addslotsrange(start, end, target_node_id)
                    .await
            }
            "DELSLOTS" => {
                // CLUSTER DELSLOTS slot [slot ...]
                if args.is_empty() {
                    return Err(AikvError::WrongArgCount("CLUSTER DELSLOTS".to_string()));
                }

                let mut slots = Vec::new();
                for arg in args {
                    let slot = String::from_utf8_lossy(arg)
                        .parse::<u16>()
                        .map_err(|_| AikvError::Invalid("Invalid slot".to_string()))?;

                    if slot >= 16384 {
                        return Err(AikvError::Invalid(format!("Slot out of range: {}", slot)));
                    }
                    slots.push(slot);
                }

                cluster_cmds.cluster_delslots(slots).await
            }
            "SETSLOT" => {
                // CLUSTER SETSLOT slot MIGRATING|IMPORTING|STABLE|NODE [node-id] [requester-node-id-internal]
                if args.len() < 2 || args.len() > 4 {
                    return Err(AikvError::WrongArgCount("CLUSTER SETSLOT".to_string()));
                }
                let slot = String::from_utf8_lossy(&args[0])
                    .parse::<u16>()
                    .map_err(|_| AikvError::Invalid("Invalid slot".to_string()))?;
                if slot >= 16384 {
                    return Err(AikvError::Invalid(format!("Slot out of range: {}", slot)));
                }
                let mode = String::from_utf8_lossy(&args[1]).to_uppercase();
                let node_id = if args.len() >= 3 {
                    let id_str = String::from_utf8_lossy(&args[2]);
                    Some(
                        id_str
                            .parse::<u64>()
                            .or_else(|_| u64::from_str_radix(&id_str, 16))
                            .map_err(|_| AikvError::Invalid("Invalid node ID".to_string()))?,
                    )
                } else {
                    None
                };
                let requester_node_id = if args.len() == 4 {
                    let id_str = String::from_utf8_lossy(&args[3]);
                    Some(
                        id_str
                            .parse::<u64>()
                            .or_else(|_| u64::from_str_radix(&id_str, 16))
                            .map_err(|_| {
                                AikvError::Invalid("Invalid requester node ID".to_string())
                            })?,
                    )
                } else {
                    None
                };
                cluster_cmds
                    .cluster_setslot(slot, &mode, node_id, requester_node_id)
                    .await
            }
            "REPLICATE" => {
                // CLUSTER REPLICATE node-id
                if args.len() != 1 {
                    return Err(AikvError::WrongArgCount("CLUSTER REPLICATE".to_string()));
                }

                let id_str = String::from_utf8_lossy(&args[0]);
                let master_id = u64::from_str_radix(&id_str, 16)
                    .map_err(|_| AikvError::Invalid("Invalid node ID".to_string()))?;

                cluster_cmds.cluster_replicate(master_id).await
            }
            "ADDREPLICATION" => {
                // CLUSTER ADDREPLICATION replica_node_id master_node_id
                // This command is sent to the leader to add a replica to a master's group
                if args.len() != 2 {
                    return Err(AikvError::WrongArgCount(
                        "CLUSTER ADDREPLICATION".to_string(),
                    ));
                }

                let replica_id_str = String::from_utf8_lossy(&args[0]);
                let replica_id = replica_id_str
                    .parse::<u64>()
                    .or_else(|_| u64::from_str_radix(&replica_id_str, 16))
                    .map_err(|_| AikvError::Invalid("Invalid replica node ID".to_string()))?;

                let master_id_str = String::from_utf8_lossy(&args[1]);
                let master_id = master_id_str
                    .parse::<u64>()
                    .or_else(|_| u64::from_str_radix(&master_id_str, 16))
                    .map_err(|_| AikvError::Invalid("Invalid master node ID".to_string()))?;

                cluster_cmds
                    .cluster_add_replication(replica_id, master_id)
                    .await
            }
            "FAILOVER" => {
                // CLUSTER FAILOVER [FORCE|TAKEOVER] [target-node-id]
                let mut mode = crate::cluster::FailoverMode::Default;
                let mut target_node_id = None;

                if !args.is_empty() {
                    let first = String::from_utf8_lossy(&args[0]).to_uppercase();
                    match first.as_str() {
                        "FORCE" => mode = crate::cluster::FailoverMode::Force,
                        "TAKEOVER" => mode = crate::cluster::FailoverMode::Takeover,
                        _ => {
                            // Backward-compatible extension: allow target node as first arg.
                            let node_id = first
                                .parse::<u64>()
                                .or_else(|_| u64::from_str_radix(&first, 16))
                                .map_err(|_| {
                                    AikvError::Invalid(format!(
                                        "Unknown CLUSTER FAILOVER option or target node ID: {}",
                                        first
                                    ))
                                })?;
                            target_node_id = Some(node_id);
                        }
                    }
                }

                if args.len() >= 2 {
                    let node_id_str = String::from_utf8_lossy(&args[1]);
                    let node_id = node_id_str
                        .parse::<u64>()
                        .or_else(|_| u64::from_str_radix(&node_id_str, 16))
                        .map_err(|_| {
                            AikvError::Invalid("Invalid CLUSTER FAILOVER target node ID".to_string())
                        })?;
                    target_node_id = Some(node_id);
                }

                if args.len() > 2 {
                    return Err(AikvError::WrongArgCount("CLUSTER FAILOVER".to_string()));
                }

                cluster_cmds.cluster_failover(mode, target_node_id).await
            }
            "METARAFT" => {
                // CLUSTER METARAFT subcommand [args...]
                if args.is_empty() {
                    return Err(AikvError::WrongArgCount("CLUSTER METARAFT".to_string()));
                }

                let metaraft_subcmd = String::from_utf8_lossy(&args[0]).to_uppercase();
                match metaraft_subcmd.as_str() {
                    "ADDLEARNER" => {
                        // CLUSTER METARAFT ADDLEARNER node_id addr
                        if args.len() != 3 {
                            return Err(AikvError::WrongArgCount(
                                "CLUSTER METARAFT ADDLEARNER".to_string(),
                            ));
                        }

                        let node_id_str = String::from_utf8_lossy(&args[1]);
                        let node_id = node_id_str
                            .parse::<u64>()
                            .or_else(|_| u64::from_str_radix(&node_id_str, 16))
                            .map_err(|_| {
                                AikvError::Invalid(
                                    "Invalid node ID: must be a positive integer or hex string"
                                        .to_string(),
                                )
                            })?;
                        let addr = String::from_utf8_lossy(&args[2]).to_string();

                        cluster_cmds
                            .cluster_metaraft_addlearner(node_id, addr)
                            .await
                    }
                    "PROMOTE" => {
                        // CLUSTER METARAFT PROMOTE node_id [node_id ...]
                        if args.len() < 2 {
                            return Err(AikvError::WrongArgCount(
                                "CLUSTER METARAFT PROMOTE".to_string(),
                            ));
                        }

                        let mut voters = Vec::new();
                        for arg in &args[1..] {
                            let node_id_str = String::from_utf8_lossy(arg);
                            let node_id = node_id_str
                                .parse::<u64>()
                                .or_else(|_| u64::from_str_radix(&node_id_str, 16))
                                .map_err(|_| {
                                    AikvError::Invalid(
                                        "Invalid node ID: must be a positive integer or hex string"
                                            .to_string(),
                                    )
                                })?;
                            voters.push(node_id);
                        }

                        cluster_cmds.cluster_metaraft_promote(voters).await
                    }
                    "MEMBERS" => {
                        // CLUSTER METARAFT MEMBERS
                        if args.len() != 1 {
                            return Err(AikvError::WrongArgCount(
                                "CLUSTER METARAFT MEMBERS".to_string(),
                            ));
                        }

                        cluster_cmds.cluster_metaraft_members().await
                    }
                    "STATUS" => {
                        // CLUSTER METARAFT STATUS
                        if args.len() != 1 {
                            return Err(AikvError::WrongArgCount(
                                "CLUSTER METARAFT STATUS".to_string(),
                            ));
                        }

                        cluster_cmds.cluster_metaraft_status().await
                    }
                    "SETSTATUS" => {
                        // CLUSTER METARAFT SETSTATUS node_id status
                        if args.len() != 3 && args.len() != 4 {
                            return Err(AikvError::WrongArgCount(
                                "CLUSTER METARAFT SETSTATUS".to_string(),
                            ));
                        }

                        let node_id_str = String::from_utf8_lossy(&args[1]);
                        let node_id = node_id_str
                            .parse::<u64>()
                            .or_else(|_| u64::from_str_radix(&node_id_str, 16))
                            .map_err(|_| {
                                AikvError::Invalid(
                                    "Invalid node ID: must be a positive integer or hex string"
                                        .to_string(),
                                )
                            })?;

                        let status = match String::from_utf8_lossy(&args[2]).to_uppercase().as_str() {
                            "ONLINE" => aidb::cluster::NodeStatus::Online,
                            "OFFLINE" => aidb::cluster::NodeStatus::Offline,
                            "JOINING" => aidb::cluster::NodeStatus::Joining,
                            "LEAVING" => aidb::cluster::NodeStatus::Leaving,
                            _ => {
                                return Err(AikvError::Invalid(
                                    "Invalid node status: expected ONLINE/OFFLINE/JOINING/LEAVING"
                                        .to_string(),
                                ))
                            }
                        };

                        let is_forwarded = if args.len() == 4 {
                            String::from_utf8_lossy(&args[3]).eq_ignore_ascii_case("__FORWARDED__")
                        } else {
                            false
                        };
                        if args.len() == 4 && !is_forwarded {
                            return Err(AikvError::Invalid(
                                "Invalid forwarding flag for CLUSTER METARAFT SETSTATUS".to_string(),
                            ));
                        }

                        cluster_cmds
                            .cluster_metaraft_setstatus(node_id, status, is_forwarded)
                            .await
                    }
                    _ => Err(AikvError::InvalidCommand(format!(
                        "Unknown CLUSTER METARAFT subcommand: {}",
                        metaraft_subcmd
                    ))),
                }
            }
            _ => Err(AikvError::InvalidCommand(format!(
                "Unknown async CLUSTER subcommand: {}",
                subcommand
            ))),
        }
    }

    /// Handle MONITOR command
    async fn handle_monitor(&mut self) -> RespValue {
        if let Some(ref broadcaster) = self.monitor_broadcaster {
            broadcaster
                .register_monitor(self.client_id, self.client_addr.clone())
                .await;
            self.mode = ConnectionMode::Monitor;
            RespValue::ok()
        } else {
            RespValue::error("ERR MONITOR not supported")
        }
    }

    fn handle_hello(&mut self, args: &[RespValue]) -> RespValue {
        if args.is_empty() {
            return RespValue::error("ERR wrong number of arguments for 'hello' command");
        }

        // Parse protocol version
        let version_str = match &args[0] {
            RespValue::BulkString(Some(v)) => String::from_utf8_lossy(v).to_string(),
            _ => return RespValue::error("ERR invalid protocol version"),
        };

        let version = match version_str.as_str() {
            "2" => ProtocolVersion::Resp2,
            "3" => ProtocolVersion::Resp3,
            _ => return RespValue::error("NOPROTO unsupported protocol version"),
        };

        self.protocol_version = version;

        // Build response based on protocol version
        match self.protocol_version {
            ProtocolVersion::Resp2 => {
                // RESP2 response: array
                RespValue::array(vec![
                    RespValue::bulk_string("server"),
                    RespValue::bulk_string("aikv"),
                    RespValue::bulk_string("version"),
                    RespValue::bulk_string("0.1.0"),
                    RespValue::bulk_string("proto"),
                    RespValue::integer(2),
                ])
            }
            ProtocolVersion::Resp3 => {
                // RESP3 response: map
                RespValue::map(vec![
                    (
                        RespValue::simple_string("server"),
                        RespValue::simple_string("aikv"),
                    ),
                    (
                        RespValue::simple_string("version"),
                        RespValue::simple_string("0.1.0"),
                    ),
                    (RespValue::simple_string("proto"), RespValue::integer(3)),
                ])
            }
        }
    }

    async fn write_response(&mut self, response: RespValue) -> Result<()> {
        let data = response.serialize();

        // Record bytes sent
        if let Some(ref metrics) = self.metrics {
            metrics.connections.record_bytes_sent(data.len() as u64);
        }

        self.stream.write_all(&data).await?;
        self.stream.flush().await?;
        Ok(())
    }
}
