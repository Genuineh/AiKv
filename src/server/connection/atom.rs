//! ATOM 事务 (MULTI/EXEC/WATCH) 与 JSON batch.

use std::collections::HashSet;

use bytes::Bytes;

use crate::command;
use crate::error::{Error, Result};
use crate::protocol::RespValue;
use crate::storage::dump::{decode as dump_decode, encode as dump_encode};
use crate::storage::StoredValue;

use super::Connection;

impl Connection {
    pub(super) async fn cmd_atom_multi(&mut self) -> Result<()> {
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

    pub(super) async fn cmd_atom_exec(&mut self, args: &[Bytes]) -> Result<()> {
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

    pub(super) async fn cmd_atom_discard(&mut self) -> Result<()> {
        if !self.tx_state.in_multi {
            return self
                .write_response(RespValue::Error("ERR DISCARD without MULTI".into()))
                .await;
        }
        self.tx_state.reset();
        self.write_response(RespValue::SimpleString("OK".into()))
            .await
    }

    pub(super) async fn cmd_atom_watch(&mut self, args: &[Bytes]) -> Result<()> {
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

    pub(super) async fn cmd_atom_unwatch(&mut self) -> Result<()> {
        self.tx_state.watched_keys.clear();
        self.write_response(RespValue::SimpleString("OK".into()))
            .await
    }

    pub(super) async fn cmd_atom_exec_json_batch(&mut self, json_arg: &Bytes) -> Result<()> {
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
                // MOVED/ASK/TRYAGAIN 包装成 Error::Command 冒泡到这里. 必须原样透传
                // 顶层错误, 让集群感知客户端 (如 StackExchange.Redis)
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

    /// 在 MULTI 模式中,将命令排入队列(不立即执行)
    pub(super) async fn cmd_atom_enqueue(&mut self, cmd: &str, args: &[Bytes]) -> Result<()> {
        self.tx_state
            .tx_queue
            .push((cmd.to_string(), args.to_vec()));
        self.write_response(RespValue::SimpleString("QUEUED".into()))
            .await
    }

    /// 为写命令跟踪 key 版本(WATCH 冲突检测用)
    pub(super) fn track_command_keys(&self, cmd: &str, args: &[Bytes]) {
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

/// 判断 batch 内部命令的错误消息是否为集群重定向 (MOVED/ASK) 或写冻结 (TRYAGAIN),
/// 与 [`is_cluster_redirect_response`] 对 `RespValue` 的判断逻辑保持一致.
pub(super) fn is_redirect_error_msg(msg: &str) -> bool {
    msg.starts_with("MOVED ") || msg.starts_with("ASK ") || msg.starts_with("TRYAGAIN ")
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
