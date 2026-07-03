//! 单连接处理

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::broadcast;
use tokio::time;
use tracing::instrument;

use crate::command;
use crate::error::{Error, Result};
use crate::protocol::{ProtocolVersion, RespParser, RespValue};
use crate::server::config::ServerSharedState;
use crate::storage::dump::{decode as dump_decode, encode as dump_encode};
use crate::storage::StoredValue;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectionMode {
    Normal,
    Monitor,
}

/// 原子事务状态 (ATOM.MULTI / EXEC / DISCARD / WATCH)
struct TransactionState {
    in_multi: bool,
    tx_queue: Vec<(String, Vec<Bytes>)>,
    watched_keys: HashMap<Vec<u8>, u64>,
}

impl TransactionState {
    fn new() -> Self {
        Self {
            in_multi: false,
            tx_queue: Vec::new(),
            watched_keys: HashMap::new(),
        }
    }

    fn reset(&mut self) {
        self.in_multi = false;
        self.tx_queue.clear();
        self.watched_keys.clear();
    }
}

pub struct Connection {
    stream: TcpStream,
    remote: SocketAddr,
    parser: RespParser,
    state: Arc<ServerSharedState>,
    protocol_version: ProtocolVersion,
    /// 客户端是否通过 HELLO 命令显式协商过协议版本。
    /// 仅在协商后才会按 RESP3 格式编码 null（`_` 替代 `$-1`）。
    protocol_negotiated: bool,
    last_active: Instant,
    client_id: usize,
    current_db: usize,
    quit: bool,
    mode: ConnectionMode,
    monitor_rx: Option<broadcast::Receiver<String>>,
    #[cfg(feature = "cluster")]
    /// 集群模式连接级状态 (asking/readonly)
    cluster_state: crate::cluster::connection::ClusterConnectionState,
    /// 原子事务状态 (ATOM.MULTI / EXEC / DISCARD / WATCH)
    tx_state: TransactionState,
}

impl Connection {
    pub async fn handle(stream: TcpStream, remote: SocketAddr, state: Arc<ServerSharedState>) {
        let client_id = state.alloc_client_id();
        state.register_client(client_id, remote);
        let mut conn = Self {
            stream,
            remote,
            parser: RespParser::new(),
            state: state.clone(),
            protocol_version: ProtocolVersion::default(),
            protocol_negotiated: false,
            last_active: Instant::now(),
            client_id,
            current_db: 0,
            quit: false,
            mode: ConnectionMode::Normal,
            monitor_rx: None,
            #[cfg(feature = "cluster")]
            cluster_state: crate::cluster::connection::ClusterConnectionState::new(),
            tx_state: TransactionState::new(),
        };
        tracing::info!(remote = %remote, client_id, "kv.connection.open");
        let _ = conn.run().await;
        state.unregister_client(client_id);
        tracing::info!(remote = %conn.remote, client_id, "kv.connection.close");
    }

    #[instrument(
        name = "kv_connection",
        skip(self),
        fields(
            otel.kind = "server",
            remote_addr = %self.remote,
            client.address = %self.remote.ip(),
            network.peer.address = %self.remote.ip(),
            network.peer.port = self.remote.port(),
            server.port = tracing::field::Empty,
            db_index = 0,
        )
    )]
    async fn run(&mut self) -> Result<()> {
        tracing::Span::current().record("db_index", self.current_db as i64);
        tracing::Span::current().record("server.port", self.state.tcp_port as i64);
        let config = Arc::clone(&self.state.connection_config);
        let mut buf = vec![0u8; 16384];

        loop {
            if self.state.shutdown.is_cancelled() {
                break;
            }
            if self.quit {
                break;
            }

            if self.mode == ConnectionMode::Monitor {
                self.run_monitor(&mut buf).await?;
                break;
            }

            if let Some(idle) = config.idle_timeout {
                if self.last_active.elapsed() > idle {
                    break;
                }
            }

            let n = if let Some(timeout) = config.read_timeout {
                match time::timeout(timeout, self.read_buf(&mut buf)).await {
                    Ok(Ok(n)) => n,
                    Ok(Err(e)) => return Err(e.into()),
                    Err(_) => break,
                }
            } else {
                self.read_buf(&mut buf).await?
            };

            if n == 0 {
                break;
            }

            if self.parser.buffer_len() + n > self.parser.max_buffer_size() {
                break;
            }

            self.parser.feed(&buf[..n]);

            loop {
                match self.parse_frame().await {
                    Ok(Some(value)) => {
                        self.process_value(value).await?;
                        tracing::Span::current().record("db_index", self.current_db as i64);
                        self.last_active = Instant::now();
                        if self.quit {
                            break;
                        }
                        if self.mode == ConnectionMode::Monitor {
                            self.run_monitor(&mut buf).await?;
                            return Ok(());
                        }
                    }
                    Ok(None) => break,
                    Err(e) if is_fatal_protocol(&e) => return Err(e),
                    Err(e) => {
                        self.write_error(&e).await?;
                    }
                }
            }
        }
        Ok(())
    }

    async fn run_monitor(&mut self, buf: &mut [u8]) -> Result<()> {
        let mut rx = self
            .monitor_rx
            .take()
            .expect("monitor mode requires monitor_rx");
        let result = self.run_monitor_loop(&mut rx, buf).await;
        self.monitor_rx = Some(rx);
        result
    }

    async fn run_monitor_loop(
        &mut self,
        rx: &mut broadcast::Receiver<String>,
        buf: &mut [u8],
    ) -> Result<()> {
        let config = Arc::clone(&self.state.connection_config);

        loop {
            if self.quit {
                break;
            }

            tokio::select! {
              read_result = self.read_monitor_input(&config, buf) => {
                match read_result {
                  Ok(0) => break,
                  Ok(n) => {
                    self.parser.feed(&buf[..n]);
                    loop {
                      match self.parse_frame().await {
                        Ok(Some(value)) => {
                          self.process_monitor_command(value).await?;
                          self.last_active = Instant::now();
                          if self.quit {
                            return Ok(());
                          }
                        }
                        Ok(None) => break,
                        Err(e) if is_fatal_protocol(&e) => return Err(e),
                        Err(_) => break,
                      }
                    }
                  }
                  Err(e) => return Err(e),
                }
              }
              msg = rx.recv() => {
                match msg {
                  Ok(line) => {
                    self.stream.write_all(line.as_bytes()).await?;
                  }
                  Err(broadcast::error::RecvError::Lagged(_)) => {}
                  Err(broadcast::error::RecvError::Closed) => break,
                }
              }
            }
        }
        Ok(())
    }

    async fn read_monitor_input(
        &mut self,
        config: &Arc<crate::server::config::ConnectionConfig>,
        buf: &mut [u8],
    ) -> Result<usize> {
        if let Some(idle) = config.idle_timeout {
            if self.last_active.elapsed() > idle {
                return Ok(0);
            }
        }
        if let Some(timeout) = config.read_timeout {
            match time::timeout(timeout, self.read_buf(buf)).await {
                Ok(Ok(n)) => Ok(n),
                Ok(Err(e)) => Err(Error::Io(e)),
                Err(_) => Ok(0),
            }
        } else {
            self.read_buf(buf).await.map_err(Error::Io)
        }
    }

    async fn process_monitor_command(&mut self, value: RespValue) -> Result<()> {
        let RespValue::Array(items) = value else {
            return Ok(());
        };
        let Some(items) = items else {
            return Ok(());
        };
        if items.is_empty() {
            return Ok(());
        }
        let Some(cmd_bytes) = extract_bulk(&items[0]) else {
            return Ok(());
        };
        let cmd = String::from_utf8_lossy(&cmd_bytes).to_ascii_uppercase();
        if cmd == "QUIT" {
            self.write_response(RespValue::SimpleString("OK".into()))
                .await?;
            self.quit = true;
        }
        Ok(())
    }

    #[instrument(name = "kv_read", skip(self, buf))]
    async fn read_buf(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.stream.read(buf).await?;
        if n > 0 {
            self.state.metrics().on_net_input_bytes(n as u64);
            tracing::debug!(bytes = n, "kv.read.complete");
        }
        Ok(n)
    }

    #[instrument(name = "kv_parse", skip(self), fields(frame_size, resp_version))]
    async fn parse_frame(&mut self) -> Result<Option<RespValue>> {
        let before = self.parser.buffer_len();
        let result = self.parser.parse();
        if let Ok(Some(_)) = &result {
            let frame_size = before.saturating_sub(self.parser.buffer_len());
            tracing::Span::current().record("frame_size", frame_size);
            tracing::Span::current().record("resp_version", self.protocol_version.as_u8());
            tracing::debug!(frame_size, "kv.parse.complete");
        }
        result
    }

    async fn process_value(&mut self, value: RespValue) -> Result<()> {
        let RespValue::Array(items) = value else {
            return self
                .write_response(RespValue::Error("Protocol error: expected array".into()))
                .await;
        };

        let Some(items) = items else {
            return Ok(());
        };
        if items.is_empty() {
            return Ok(());
        }

        let Some(cmd_bytes) = extract_bulk(&items[0]) else {
            return self
                .write_response(RespValue::Error(
                    "Protocol error: command must be bulk string".into(),
                ))
                .await;
        };
        let cmd = String::from_utf8_lossy(&cmd_bytes).to_ascii_uppercase();
        let args: Result<Vec<Bytes>> = items[1..]
            .iter()
            .map(|v| {
                extract_bulk(v).ok_or_else(|| {
                    Error::Protocol("Protocol error: arguments must be bulk strings".into())
                })
            })
            .collect();
        let args = match args {
            Ok(a) => a,
            Err(e) => return self.write_error(&e).await,
        };

        self.process_command(&cmd, &args).await
    }

    async fn process_command(&mut self, cmd: &str, args: &[Bytes]) -> Result<()> {
        if cmd != "MONITOR" && self.mode != ConnectionMode::Monitor {
            self.broadcast_monitor(cmd, args);
        }

        match cmd {
            "PING" => self.cmd_ping(args).await,
            "ECHO" => self.cmd_echo(args).await,
            "HELLO" => self.cmd_hello(args).await,
            "QUIT" => self.cmd_quit().await,
            "MONITOR" => self.cmd_monitor().await,
            "MULTI" | "ATOM.MULTI" => self.cmd_atom_multi().await,
            "EXEC" | "ATOM.EXEC" => self.cmd_atom_exec(args).await,
            "DISCARD" | "ATOM.DISCARD" => self.cmd_atom_discard().await,
            "WATCH" | "ATOM.WATCH" => self.cmd_atom_watch(args).await,
            "UNWATCH" | "ATOM.UNWATCH" => self.cmd_atom_unwatch().await,
            _ => {
                // If in MULTI mode, queue the command instead of executing
                if self.tx_state.in_multi {
                    return self.cmd_atom_enqueue(cmd, args).await;
                }
                // Handle ASKING/READONLY/READWRITE at connection level (operate on per-conn state)
                #[cfg(feature = "cluster")]
                if cmd.eq_ignore_ascii_case("asking") {
                    self.cluster_state.set_asking(true);
                    return self
                        .write_response(RespValue::SimpleString("OK".into()))
                        .await;
                }
                #[cfg(feature = "cluster")]
                if cmd.eq_ignore_ascii_case("readonly") {
                    self.cluster_state.set_readonly(true);
                    return self
                        .write_response(RespValue::SimpleString("OK".into()))
                        .await;
                }
                #[cfg(feature = "cluster")]
                if cmd.eq_ignore_ascii_case("readwrite") {
                    self.cluster_state.set_readonly(false);
                    return self
                        .write_response(RespValue::SimpleString("OK".into()))
                        .await;
                }

                let track = should_track_observability(cmd);
                let arg_strings: Vec<String> = if track {
                    args.iter()
                        .map(|b| String::from_utf8_lossy(b).into_owned())
                        .collect()
                } else {
                    Vec::new()
                };
                let started = track.then(Instant::now);
                let result = self
                    .state
                    .router()
                    .execute_with_client(
                        cmd,
                        args,
                        &mut self.current_db,
                        Some(self.client_id),
                        Some(self.remote),
                        self.protocol_version,
                        #[cfg(feature = "cluster")]
                        Some(&self.cluster_state),
                    )
                    .await;
                #[cfg(feature = "cluster")]
                {
                    self.cluster_state.reset_asking();
                }
                match result {
                    Ok(resp) => {
                        self.state.set_client_db(self.client_id, self.current_db);
                        if let Some(start) = started {
                            #[cfg(feature = "cluster")]
                            let skip_obs = is_cluster_redirect_response(&resp);
                            #[cfg(not(feature = "cluster"))]
                            let skip_obs = false;
                            if !skip_obs {
                                self.record_command_observability(cmd, &arg_strings, start, true);
                            }
                        }
                        self.write_response(resp).await?;
                        // Track key versions for write commands (WATCH support)
                        self.track_command_keys(cmd, args);
                        if cmd == "SHUTDOWN" {
                            self.quit = true;
                        }
                        Ok(())
                    }
                    Err(e) => {
                        if let Some(start) = started {
                            self.record_command_observability(cmd, &arg_strings, start, false);
                        }
                        tracing::error!(
                          command = cmd,
                          client_id = self.client_id,
                          error = %e,
                          "kv.command.error"
                        );
                        self.write_error(&e).await
                    }
                }
            }
        }
    }

    fn broadcast_monitor(&self, cmd: &str, args: &[Bytes]) {
        let line = format_monitor_line(self.current_db, cmd, args);
        let _ = self.state.monitor_tx.send(line);
    }

    async fn cmd_ping(&mut self, args: &[Bytes]) -> Result<()> {
        if args.len() > 1 {
            return self
                .write_response(RespValue::Error(
                    "ERR wrong number of arguments for 'ping' command".into(),
                ))
                .await;
        }
        if args.is_empty() {
            self.write_response(RespValue::SimpleString("PONG".into()))
                .await
        } else {
            self.write_response(RespValue::BulkString(Some(args[0].clone())))
                .await
        }
    }

    async fn cmd_echo(&mut self, args: &[Bytes]) -> Result<()> {
        if args.len() != 1 {
            return self
                .write_response(RespValue::Error(
                    "ERR wrong number of arguments for 'echo' command".into(),
                ))
                .await;
        }
        self.write_response(RespValue::BulkString(Some(args[0].clone())))
            .await
    }

    async fn cmd_hello(&mut self, args: &[Bytes]) -> Result<()> {
        if args.len() > 1 {
            return self
                .write_response(RespValue::Error(
                    "ERR wrong number of arguments for 'hello' command".into(),
                ))
                .await;
        }

        if args.len() == 1 {
            let ver = String::from_utf8_lossy(&args[0]);
            match ver.as_ref() {
                "2" => self.protocol_version = ProtocolVersion::Resp2,
                "3" => self.protocol_version = ProtocolVersion::Resp3,
                _ => {
                    return self
                        .write_response(RespValue::Error("invalid protocol version".into()))
                        .await;
                }
            }
            self.protocol_negotiated = true;
        }

        let response = hello_map(self.client_id, self.protocol_version);
        self.write_response(response).await
    }

    async fn cmd_quit(&mut self) -> Result<()> {
        self.write_response(RespValue::SimpleString("OK".into()))
            .await?;
        self.quit = true;
        Ok(())
    }

    async fn cmd_monitor(&mut self) -> Result<()> {
        self.write_response(RespValue::SimpleString("OK".into()))
            .await?;
        self.mode = ConnectionMode::Monitor;
        self.monitor_rx = Some(self.state.monitor_tx.subscribe());
        Ok(())
    }

    // ─── ATOM 事务命令 ─────────────────────────────────────────

    async fn cmd_atom_multi(&mut self) -> Result<()> {
        if self.tx_state.in_multi {
            return self
                .write_response(RespValue::Error("ERR MULTI calls can not be nested".into()))
                .await;
        }
        self.tx_state.in_multi = true;
        self.tx_state.tx_queue.clear();
        self.write_response(RespValue::SimpleString("OK".into()))
            .await
    }

    async fn cmd_atom_exec(&mut self, args: &[Bytes]) -> Result<()> {
        if !self.tx_state.in_multi {
            if args.len() == 1 {
                return self.cmd_atom_exec_json_batch(&args[0]).await;
            }
            if args.is_empty() {
                return self
                    .write_response(RespValue::Error("ERR EXEC without MULTI".into()))
                    .await;
            }
            return self
                .write_response(RespValue::Error(
                    "ERR wrong number of arguments for 'exec' command".into(),
                ))
                .await;
        }
        if !args.is_empty() {
            return self
                .write_response(RespValue::Error("ERR EXEC inside MULTI".into()))
                .await;
        }

        // Check WATCH conflicts: if any watched key's version changed, abort
        let conflict = self
            .tx_state
            .watched_keys
            .iter()
            .any(|(key, version)| self.state.get_key_version(key) != *version);

        if conflict {
            self.tx_state.reset();
            return self.write_response(RespValue::BulkString(None)).await;
        }

        // Execute queued commands atomically
        let queue = std::mem::take(&mut self.tx_state.tx_queue);
        self.tx_state.in_multi = false;
        self.tx_state.watched_keys.clear();

        let mut results = Vec::with_capacity(queue.len());

        for (cmd_name, cmd_args) in queue {
            // Track whether this is a write command for key version tracking
            let is_write =
                command::lookup(&cmd_name).is_some_and(|info| info.flags.contains(&"write"));

            let result = self
                .state
                .router()
                .execute_with_client(
                    &cmd_name,
                    &cmd_args,
                    &mut self.current_db,
                    Some(self.client_id),
                    Some(self.remote),
                    self.protocol_version,
                    #[cfg(feature = "cluster")]
                    Some(&self.cluster_state),
                )
                .await;

            match result {
                Ok(resp) => {
                    if is_write {
                        self.track_command_keys(&cmd_name, &cmd_args);
                    }
                    results.push(resp);
                }
                Err(e) => {
                    let msg = format!("{e}");
                    results.push(RespValue::Error(msg));
                }
            }
        }

        self.write_response(RespValue::Array(Some(results))).await
    }

    async fn cmd_atom_discard(&mut self) -> Result<()> {
        if !self.tx_state.in_multi {
            return self
                .write_response(RespValue::Error("ERR DISCARD without MULTI".into()))
                .await;
        }
        self.tx_state.reset();
        self.write_response(RespValue::SimpleString("OK".into()))
            .await
    }

    async fn cmd_atom_watch(&mut self, args: &[Bytes]) -> Result<()> {
        if args.is_empty() {
            return self
                .write_response(RespValue::Error(
                    "ERR wrong number of arguments for 'watch' command".into(),
                ))
                .await;
        }
        // Record current version for each watched key
        for key in args {
            let version = self.state.get_key_version(key);
            self.tx_state.watched_keys.insert(key.to_vec(), version);
        }
        self.write_response(RespValue::SimpleString("OK".into()))
            .await
    }

    async fn cmd_atom_unwatch(&mut self) -> Result<()> {
        self.tx_state.watched_keys.clear();
        self.write_response(RespValue::SimpleString("OK".into()))
            .await
    }

    async fn cmd_atom_exec_json_batch(&mut self, json_arg: &Bytes) -> Result<()> {
        let commands = match parse_json_batch_commands(json_arg) {
            Ok(cmds) => cmds,
            Err(msg) => return self.write_response(RespValue::Error(msg)).await,
        };

        if commands.is_empty() {
            return self
                .write_response(RespValue::Array(Some(Vec::new())))
                .await;
        }

        if let Some(msg) = detect_duplicate_batch_keys(&commands) {
            return self.write_response(RespValue::Error(msg)).await;
        }

        let mut snapshots: Vec<BatchRollbackFrame> = Vec::new();
        let mut snapshotted: HashSet<Vec<u8>> = HashSet::new();
        let mut results: Vec<RespValue> = Vec::with_capacity(commands.len());
        let mut written_cmds: Vec<(String, Vec<Bytes>)> = Vec::new();

        for (cmd_name, cmd_args) in commands {
            if let Err(e) = self
                .snapshot_write_keys(&cmd_name, &cmd_args, &mut snapshots, &mut snapshotted)
                .await
            {
                // 集群拓扑变化 (slot 迁移/重新分片) 时, 快照阶段内部路由的
                // DUMP 可能命中不在本节点的 key, 底层 routed_command 会把
                // MOVED/ASK 包装成 Error::Command 冒泡到这里. 必须原样透传
                // 顶层 MOVED/ASK, 让集群感知客户端 (如 StackExchange.Redis)
                // 按标准协议重定向整个 batch 到正确节点重试, 而不是把它
                // 裹成一个客户端无法识别的通用 "internal error".
                let msg = format_batch_exec_error(&e);
                if let Err(rollback_err) = self.rollback_batch_snapshots(&snapshots).await {
                    tracing::error!(
                      target: "kv.batch",
                      error = %rollback_err,
                      "CRITICAL: JSON batch rollback failed after snapshot error"
                    );
                    return self
                        .write_response(RespValue::Error("ERR batch rollback failed".into()))
                        .await;
                }
                if is_redirect_error_msg(&msg) {
                    return self.write_response(RespValue::Error(msg)).await;
                }
                return self
                    .write_response(RespValue::Error(format!(
                        "ERR internal error during batch snapshot: {msg}"
                    )))
                    .await;
            }

            let result = self
                .state
                .router()
                .execute_with_client(
                    &cmd_name,
                    &cmd_args,
                    &mut self.current_db,
                    Some(self.client_id),
                    Some(self.remote),
                    self.protocol_version,
                    #[cfg(feature = "cluster")]
                    Some(&self.cluster_state),
                )
                .await;

            if let Some(err_msg) = batch_command_failure_message(&result) {
                if let Err(rollback_err) = self.rollback_batch_snapshots(&snapshots).await {
                    tracing::error!(
                      target: "kv.batch",
                      error = %rollback_err,
                      "CRITICAL: JSON batch rollback failed after command error"
                    );
                    return self
                        .write_response(RespValue::Error("ERR batch rollback failed".into()))
                        .await;
                }
                return self.write_response(RespValue::Error(err_msg)).await;
            }

            if let Ok(resp) = result {
                mark_batch_command_keys_written(&cmd_name, &cmd_args, &mut snapshots);
                results.push(resp);
                written_cmds.push((cmd_name, cmd_args));
            }
        }

        for (cmd_name, cmd_args) in &written_cmds {
            self.track_command_keys(cmd_name, cmd_args);
        }

        self.write_response(RespValue::Array(Some(results))).await
    }

    /// 经路由层执行命令 (集群模式下转发至 key 所在分片).
    async fn routed_command(&mut self, cmd: &str, args: Vec<Bytes>) -> Result<RespValue> {
        self.state
            .router()
            .execute_with_client(
                cmd,
                &args,
                &mut self.current_db,
                Some(self.client_id),
                Some(self.remote),
                self.protocol_version,
                #[cfg(feature = "cluster")]
                Some(&self.cluster_state),
            )
            .await
    }

    async fn snapshot_write_keys(
        &mut self,
        cmd: &str,
        args: &[Bytes],
        snapshots: &mut Vec<BatchRollbackFrame>,
        snapshotted: &mut HashSet<Vec<u8>>,
    ) -> Result<()> {
        let Some(info) = command::lookup(cmd) else {
            return Ok(());
        };
        if !info.flags.contains(&"write") {
            return Ok(());
        }
        let indices = command::key_indices(&info, args.len() + 1);
        for idx in indices {
            if idx == 0 {
                continue;
            }
            let arr_idx = idx - 1;
            if arr_idx >= args.len() {
                continue;
            }
            let key = args[arr_idx].to_vec();
            if !snapshotted.insert(key.clone()) {
                continue;
            }
            let previous = self.batch_load_key_snapshot(&key).await?;
            snapshots.push(BatchRollbackFrame {
                key,
                previous,
                written: false,
            });
        }
        Ok(())
    }

    async fn batch_load_key_snapshot(&mut self, key: &[u8]) -> Result<Option<StoredValue>> {
        let resp = self
            .routed_command("DUMP", vec![Bytes::copy_from_slice(key)])
            .await?;
        match resp {
            RespValue::BulkString(None) => Ok(None),
            RespValue::BulkString(Some(payload)) => dump_decode(&payload).map(Some),
            RespValue::Error(msg) => Err(Error::Command(msg)),
            other => Err(Error::Command(format!(
                "unexpected DUMP response: {other:?}"
            ))),
        }
    }

    async fn rollback_batch_snapshots(&mut self, snapshots: &[BatchRollbackFrame]) -> Result<()> {
        for frame in snapshots.iter().rev().filter(|f| f.written) {
            match &frame.previous {
                None => {
                    let resp = self
                        .routed_command("DEL", vec![Bytes::copy_from_slice(&frame.key)])
                        .await?;
                    if let RespValue::Error(msg) = resp {
                        return Err(Error::Command(msg));
                    }
                }
                Some(previous) => {
                    let payload = dump_encode(previous)?;
                    let resp = self
                        .routed_command(
                            "RESTORE",
                            vec![
                                Bytes::copy_from_slice(&frame.key),
                                Bytes::from_static(b"0"),
                                Bytes::from(payload),
                                Bytes::from_static(b"REPLACE"),
                            ],
                        )
                        .await?;
                    if let RespValue::Error(msg) = resp {
                        return Err(Error::Command(msg));
                    }
                }
            }
        }
        Ok(())
    }

    /// 在 MULTI 模式中，将命令排入队列（不立即执行）
    async fn cmd_atom_enqueue(&mut self, cmd: &str, args: &[Bytes]) -> Result<()> {
        self.tx_state
            .tx_queue
            .push((cmd.to_string(), args.to_vec()));
        self.write_response(RespValue::SimpleString("QUEUED".into()))
            .await
    }

    /// 为写命令跟踪 key 版本（WATCH 冲突检测用）
    fn track_command_keys(&self, cmd: &str, args: &[Bytes]) {
        let Some(info) = command::lookup(cmd) else {
            return;
        };
        if !info.flags.contains(&"write") {
            return;
        }
        let indices = command::key_indices(&info, args.len() + 1);
        for idx in indices {
            if idx > 0 {
                let arr_idx = idx - 1;
                if arr_idx < args.len() {
                    self.state.increment_key_version(&args[arr_idx]);
                }
            }
        }
    }

    async fn write_error(&mut self, err: &Error) -> Result<()> {
        let msg = match err {
            Error::Protocol(s) => format!("Protocol error: {s}"),
            Error::Command(s) => s.clone(),
            Error::Io(e) => return Err(Error::Io(std::io::Error::new(e.kind(), e.to_string()))),
            Error::Storage(s) => format!("Internal storage error: {s}"),
            Error::Config(s) => format!("CONFIG error: {s}"),
            #[cfg(feature = "cluster")]
            Error::Cluster(s) => format!("CLUSTER error: {s}"),
        };
        self.write_response(RespValue::Error(msg)).await
    }

    #[instrument(name = "kv_write", skip(self, value), fields(response_size))]
    async fn write_response(&mut self, value: RespValue) -> Result<()> {
        let bytes = self.encode(&value);
        tracing::Span::current().record("response_size", bytes.len());
        self.stream.write_all(&bytes).await?;
        self.state.metrics().on_net_output_bytes(bytes.len() as u64);
        tracing::debug!(response_size = bytes.len(), "kv.write.complete");
        Ok(())
    }

    #[instrument(name = "kv_encode", skip(self, value), fields(value_type))]
    fn encode(&self, value: &RespValue) -> Bytes {
        // 仅 RESP3 协商后才做 null 适配, 否则直接序列化 (免克隆, F-034)
        let bytes = if self.protocol_negotiated && self.protocol_version == ProtocolVersion::Resp3 {
            self.adapt_null_to_resp3(value).serialize()
        } else {
            value.serialize()
        };
        tracing::debug!(encoded_size = bytes.len(), "kv.encode.complete");
        bytes
    }

    /// 递归检查 RespValue 树中是否包含 RESP2 风格的 null
    /// (BulkString(None) 或 Array(None))，用于免克隆短路 (F-034)。
    fn contains_resp_null(value: &RespValue) -> bool {
        match value {
            RespValue::BulkString(None) | RespValue::Array(None) => true,
            RespValue::Array(Some(items)) => items.iter().any(Self::contains_resp_null),
            RespValue::Map(pairs) => pairs
                .iter()
                .any(|(k, v)| Self::contains_resp_null(k) || Self::contains_resp_null(v)),
            RespValue::Set(items) => items.iter().any(Self::contains_resp_null),
            RespValue::Push(items) => items.iter().any(Self::contains_resp_null),
            RespValue::Attribute { attributes, data } => {
                attributes
                    .iter()
                    .any(|(k, v)| Self::contains_resp_null(k) || Self::contains_resp_null(v))
                    || Self::contains_resp_null(data)
            }
            _ => false,
        }
    }

    /// RESP3 模式下将 RESP2 风格的 null 表示转为 RESP3 原生 Null.
    /// redis-py 8.0 的 RESP3 解析器对 `$-1\r\n` / `*-1\r\n` 处理有兼容性问题,
    /// 需使用 RESP3 原生 `_\r\n` (Null) 替代.
    /// RESP3 模式下将 RESP2 风格的 null 表示转为 RESP3 原生 Null。
    /// redis-py 8.0 的 RESP3 解析器对 `$-1\r\n` / `*-1\r\n` 处理有兼容性问题，
    /// 需使用 RESP3 原生 `_\r\n` (Null) 替代。
    fn adapt_null_to_resp3(&self, value: &RespValue) -> RespValue {
        match value {
            RespValue::BulkString(None) | RespValue::Array(None) => RespValue::Null,
            RespValue::Array(Some(items)) => {
                if !items.iter().any(Self::contains_resp_null) {
                    return value.clone();
                }
                RespValue::Array(Some(
                    items.iter().map(|v| self.adapt_null_to_resp3(v)).collect(),
                ))
            }
            RespValue::Map(pairs) => {
                if !pairs
                    .iter()
                    .any(|(k, v)| Self::contains_resp_null(k) || Self::contains_resp_null(v))
                {
                    return value.clone();
                }
                RespValue::Map(
                    pairs
                        .iter()
                        .map(|(k, v)| (self.adapt_null_to_resp3(k), self.adapt_null_to_resp3(v)))
                        .collect(),
                )
            }
            RespValue::Set(items) => {
                if !items.iter().any(Self::contains_resp_null) {
                    return value.clone();
                }
                RespValue::Set(items.iter().map(|v| self.adapt_null_to_resp3(v)).collect())
            }
            RespValue::Push(items) => {
                if !items.iter().any(Self::contains_resp_null) {
                    return value.clone();
                }
                RespValue::Push(items.iter().map(|v| self.adapt_null_to_resp3(v)).collect())
            }
            RespValue::Attribute { attributes, data } => {
                if !attributes
                    .iter()
                    .any(|(k, v)| Self::contains_resp_null(k) || Self::contains_resp_null(v))
                    && !Self::contains_resp_null(data)
                {
                    return value.clone();
                }
                RespValue::Attribute {
                    attributes: attributes
                        .iter()
                        .map(|(k, v)| (self.adapt_null_to_resp3(k), self.adapt_null_to_resp3(v)))
                        .collect(),
                    data: Box::new(self.adapt_null_to_resp3(data)),
                }
            }
            other => other.clone(),
        }
    }

    fn record_command_observability(&self, cmd: &str, args: &[String], start: Instant, ok: bool) {
        let duration_us = start.elapsed().as_micros() as u64;
        self.state.latency_stats.record(cmd, duration_us);
        self.state.slow_query_log.record(
            cmd,
            args,
            duration_us,
            &self.remote.to_string(),
            self.current_db as u16,
        );
        self.state
            .metrics()
            .on_command_duration(cmd, duration_us, ok);
        if duration_us >= self.state.slow_query_log.threshold_us() {
            self.state.metrics().on_slowlog_command(cmd, duration_us);
            #[cfg(feature = "monitoring")]
            self.state.metrics().on_slow_query(cmd);
        }
    }
}

fn should_track_observability(cmd: &str) -> bool {
    !matches!(
        cmd,
        "SLOWLOG" | "MONITOR" | "PING" | "ECHO" | "HELLO" | "QUIT"
    )
}

#[cfg(feature = "cluster")]
fn is_cluster_redirect_response(resp: &RespValue) -> bool {
    matches!(resp, RespValue::Error(msg) if is_redirect_error_msg(msg))
}

fn format_monitor_line(db: usize, cmd: &str, args: &[Bytes]) -> String {
    let sec = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mut line = format!("+{sec} [db {db}] \"{cmd}\"");
    for arg in args {
        let s = String::from_utf8_lossy(arg);
        line.push_str(&format!(" \"{s}\""));
    }
    line.push_str("\r\n");
    line
}

fn hello_map(client_id: usize, proto: ProtocolVersion) -> RespValue {
    #[cfg(feature = "cluster")]
    let mode = match crate::cluster::state::CLUSTER_STATE_MGR.get() {
        Some(_) => "cluster",
        None => "standalone",
    };
    #[cfg(not(feature = "cluster"))]
    let mode = "standalone";

    #[cfg(feature = "cluster")]
    let role = match crate::cluster::state::CLUSTER_STATE_MGR.get() {
        Some(mgr) => {
            let meta = mgr.meta_raft.get_cluster_meta();
            crate::cluster::replication::node_replication_role(&meta, mgr.node_id).to_string()
        }
        None => "master".to_string(),
    };
    #[cfg(not(feature = "cluster"))]
    let role = "master".to_string();

    // 对齐 Redis 7 原生 HELLO 响应格式:
    //   RESP3 Map — proto/id 为 Integer, 其余为 BulkString
    //   RESP2 Array — 全部 BulkString (RESP2 无 Map, 用 flat array 交替 key-value)
    let version = env!("CARGO_PKG_VERSION");
    match proto {
        ProtocolVersion::Resp3 => RespValue::Map(vec![
            (
                RespValue::BulkString(Some(Bytes::from_static(b"server"))),
                RespValue::BulkString(Some(Bytes::from_static(b"aikv"))),
            ),
            (
                RespValue::BulkString(Some(Bytes::from_static(b"version"))),
                RespValue::BulkString(Some(Bytes::from(version))),
            ),
            (
                RespValue::BulkString(Some(Bytes::from_static(b"proto"))),
                RespValue::Integer(proto.as_u8() as i64),
            ),
            (
                RespValue::BulkString(Some(Bytes::from_static(b"id"))),
                RespValue::Integer(client_id as i64),
            ),
            (
                RespValue::BulkString(Some(Bytes::from_static(b"mode"))),
                RespValue::BulkString(Some(Bytes::from(mode))),
            ),
            (
                RespValue::BulkString(Some(Bytes::from_static(b"role"))),
                RespValue::BulkString(Some(Bytes::from(role))),
            ),
        ]),
        ProtocolVersion::Resp2 => RespValue::Array(Some(vec![
            RespValue::BulkString(Some(Bytes::from_static(b"server"))),
            RespValue::BulkString(Some(Bytes::from_static(b"aikv"))),
            RespValue::BulkString(Some(Bytes::from_static(b"version"))),
            RespValue::BulkString(Some(Bytes::from(version))),
            RespValue::BulkString(Some(Bytes::from_static(b"proto"))),
            RespValue::BulkString(Some(Bytes::from(format!("{}", proto.as_u8())))),
            RespValue::BulkString(Some(Bytes::from_static(b"id"))),
            RespValue::BulkString(Some(Bytes::from(format!("{client_id}")))),
            RespValue::BulkString(Some(Bytes::from_static(b"mode"))),
            RespValue::BulkString(Some(Bytes::from(mode))),
            RespValue::BulkString(Some(Bytes::from_static(b"role"))),
            RespValue::BulkString(Some(Bytes::from(role))),
        ])),
    }
}

struct BatchRollbackFrame {
    key: Vec<u8>,
    previous: Option<StoredValue>,
    /// 仅回滚本 batch 已成功写入的 key, 避免失败事务用旧 snapshot 覆盖并发写入.
    written: bool,
}

fn mark_batch_command_keys_written(
    cmd: &str,
    args: &[Bytes],
    snapshots: &mut [BatchRollbackFrame],
) {
    let Some(info) = command::lookup(cmd) else {
        return;
    };
    if !info.flags.contains(&"write") {
        return;
    }
    let indices = command::key_indices(&info, args.len() + 1);
    for idx in indices {
        if idx == 0 {
            continue;
        }
        let arr_idx = idx - 1;
        if arr_idx >= args.len() {
            continue;
        }
        let key = args[arr_idx].as_ref();
        for frame in snapshots.iter_mut() {
            if frame.key.as_slice() == key {
                frame.written = true;
            }
        }
    }
}

fn batch_command_failure_message(result: &Result<RespValue>) -> Option<String> {
    match result {
        Err(err) => Some(format_batch_exec_error(err)),
        Ok(RespValue::Error(msg)) => Some(msg.clone()),
        _ => None,
    }
}

/// 判断 batch 内部命令的错误消息是否为集群重定向 (MOVED/ASK),
/// 与 [`is_cluster_redirect_response`] 对 `RespValue` 的判断逻辑保持一致.
fn is_redirect_error_msg(msg: &str) -> bool {
    msg.starts_with("MOVED ") || msg.starts_with("ASK ")
}

fn format_batch_exec_error(err: &Error) -> String {
    match err {
        Error::Protocol(s) => format!("Protocol error: {s}"),
        Error::Command(s) => s.clone(),
        Error::Storage(s) => format!("Internal storage error: {s}"),
        Error::Config(s) => format!("CONFIG error: {s}"),
        #[cfg(feature = "cluster")]
        Error::Cluster(s) => format!("CLUSTER error: {s}"),
        Error::Io(e) => format!("IO error: {e}"),
    }
}

fn detect_duplicate_batch_keys(commands: &[(String, Vec<Bytes>)]) -> Option<String> {
    let mut seen = HashSet::new();
    for (cmd, args) in commands {
        for key in collect_command_keys(cmd, args) {
            if !seen.insert(key) {
                return Some("ERR duplicate key in batch".into());
            }
        }
    }
    None
}

fn collect_command_keys(cmd: &str, args: &[Bytes]) -> Vec<Vec<u8>> {
    let Some(info) = command::lookup(cmd) else {
        return Vec::new();
    };
    command::key_indices(&info, args.len() + 1)
        .into_iter()
        .filter_map(|idx| {
            if idx == 0 {
                return None;
            }
            let arr_idx = idx - 1;
            (arr_idx < args.len()).then(|| args[arr_idx].to_vec())
        })
        .collect()
}

fn parse_json_batch_commands(
    json_arg: &Bytes,
) -> std::result::Result<Vec<(String, Vec<Bytes>)>, String> {
    let value: serde_json::Value =
        serde_json::from_slice(json_arg).map_err(|e| format!("ERR invalid JSON batch: {e}"))?;
    let rows = value
        .as_array()
        .ok_or_else(|| "ERR invalid JSON batch: expected array of arrays".to_string())?;
    let mut commands = Vec::with_capacity(rows.len());
    for row in rows {
        let items = row
            .as_array()
            .ok_or_else(|| "ERR invalid JSON batch: expected array of arrays".to_string())?;
        if items.len() < 2 {
            return Err("ERR invalid command in batch".into());
        }
        let cmd_name = json_batch_arg_string(&items[0])?;
        let mut args = Vec::with_capacity(items.len().saturating_sub(1));
        for item in &items[1..] {
            args.push(Bytes::from(json_batch_arg_string(item)?));
        }
        commands.push((cmd_name, args));
    }
    Ok(commands)
}

fn json_batch_arg_string(value: &serde_json::Value) -> std::result::Result<String, String> {
    match value {
        serde_json::Value::String(s) => Ok(s.clone()),
        serde_json::Value::Number(n) => Ok(n.to_string()),
        serde_json::Value::Bool(b) => Ok(b.to_string()),
        serde_json::Value::Null => Ok(String::new()),
        _ => Err("ERR invalid JSON batch: command arguments must be scalars".into()),
    }
}

fn extract_bulk(value: &RespValue) -> Option<Bytes> {
    match value {
        RespValue::BulkString(Some(b)) => Some(b.clone()),
        RespValue::BulkString(None) => Some(Bytes::new()),
        _ => None,
    }
}

fn is_fatal_protocol(err: &Error) -> bool {
    match err {
        Error::Protocol(msg) => {
            msg.contains("depth")
                || msg.contains("too large")
                || msg.contains("buffer size")
                || msg.contains("line too long")
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_map_uses_bulkstring_for_all_values() {
        // 对齐 Redis 7 原生: RESP3 Map 中 proto/id 是 Integer, 其余是 BulkString.

        // 测试 RESP3 Map 响应
        let map = hello_map(42, ProtocolVersion::Resp3);
        let RespValue::Map(pairs) = map else {
            panic!("RESP3 HELLO 应返回 Map");
        };
        for (key, value) in &pairs {
            assert!(
                matches!(key, RespValue::BulkString(Some(_))),
                "Map key 必须是 BulkString，得到: {key:?}"
            );
            let RespValue::BulkString(Some(key_bytes)) = key else {
                unreachable!()
            };
            let key_str = String::from_utf8_lossy(key_bytes);
            match key_str.as_ref() {
                "proto" | "id" => {
                    assert!(
                        matches!(value, RespValue::Integer(_)),
                        "字段 '{key_str}' 必须是 Integer (Redis 7 兼容), 得到: {value:?}"
                    );
                }
                _ => {
                    assert!(
                        matches!(value, RespValue::BulkString(Some(_))),
                        "字段 '{key_str}' 必须是 BulkString, 得到: {value:?}"
                    );
                }
            }
        }

        // 测试 RESP2 Array 响应: 全部 BulkString (RESP2 无原生 Integer 语义)
        let arr = hello_map(42, ProtocolVersion::Resp2);
        let RespValue::Array(Some(items)) = arr else {
            panic!("RESP2 HELLO 应返回 Array");
        };
        assert_eq!(items.len() % 2, 0, "Array 项数应为偶数 (交替 key-value)");
        for item in &items {
            assert!(
                matches!(item, RespValue::BulkString(Some(_))),
                "RESP2 Array 中的每一项必须是 BulkString，得到: {item:?}"
            );
        }
    }
}
