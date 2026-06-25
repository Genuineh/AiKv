//! 连接级指标 (Phase 8: Atomic; monitoring 下 OTel 导出)

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
#[cfg(feature = "monitoring")]
use std::sync::Arc;
use std::sync::Mutex;

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct CommandTotals {
    pub(crate) ok: u64,
    pub(crate) err: u64,
    pub(crate) usec: u64,
    pub(crate) rejected: u64,
    pub(crate) slowlog_count: u64,
    pub(crate) slowlog_time_ms_sum: u64,
    pub(crate) slowlog_time_ms_max: u64,
}

/// 客户端可见命令统计 (排除 GOSSIP.tick 等内部伪命令).
fn is_client_command(command: &str) -> bool {
    !command.contains('.')
}

#[derive(Debug)]
pub struct ServerMetrics {
    connections_total: AtomicU64,
    connected_clients: AtomicUsize,
    keyspace_hits: AtomicU64,
    keyspace_misses: AtomicU64,
    lua_execution_duration_us: AtomicU64,
    lua_execution_count: AtomicU64,
    commands_total: Mutex<HashMap<String, CommandTotals>>,
    net_input_bytes: AtomicU64,
    net_output_bytes: AtomicU64,
    expired_keys: AtomicU64,
    rejected_connections: AtomicU64,
    used_memory_bytes: AtomicU64,
    used_memory_peak_bytes: AtomicU64,
    evicted_keys: AtomicU64,
    instantaneous_ops_per_sec: AtomicU64,
    ops_last_commands: AtomicU64,
    ops_last_sample_secs: AtomicU64,
    cluster_messages_sent: AtomicU64,
    cluster_messages_received: AtomicU64,
    blocked_clients: AtomicUsize,
    #[cfg(feature = "monitoring")]
    process_last_cpu_user_seconds_bits: AtomicU64,
    #[cfg(feature = "monitoring")]
    process_last_cpu_sys_seconds_bits: AtomicU64,
    #[cfg(feature = "monitoring")]
    process_last_read_bytes: AtomicU64,
    #[cfg(feature = "monitoring")]
    process_last_write_bytes: AtomicU64,
    #[cfg(feature = "monitoring")]
    otel: Option<Arc<super::otel_metrics::OtelMetrics>>,
}

impl Default for ServerMetrics {
    fn default() -> Self {
        Self {
            connections_total: AtomicU64::new(0),
            connected_clients: AtomicUsize::new(0),
            keyspace_hits: AtomicU64::new(0),
            keyspace_misses: AtomicU64::new(0),
            lua_execution_duration_us: AtomicU64::new(0),
            lua_execution_count: AtomicU64::new(0),
            commands_total: Mutex::new(HashMap::new()),
            net_input_bytes: AtomicU64::new(0),
            net_output_bytes: AtomicU64::new(0),
            expired_keys: AtomicU64::new(0),
            rejected_connections: AtomicU64::new(0),
            used_memory_bytes: AtomicU64::new(0),
            used_memory_peak_bytes: AtomicU64::new(0),
            evicted_keys: AtomicU64::new(0),
            instantaneous_ops_per_sec: AtomicU64::new(0),
            ops_last_commands: AtomicU64::new(0),
            ops_last_sample_secs: AtomicU64::new(0),
            cluster_messages_sent: AtomicU64::new(0),
            cluster_messages_received: AtomicU64::new(0),
            blocked_clients: AtomicUsize::new(0),
            #[cfg(feature = "monitoring")]
            process_last_cpu_user_seconds_bits: AtomicU64::new(0),
            #[cfg(feature = "monitoring")]
            process_last_cpu_sys_seconds_bits: AtomicU64::new(0),
            #[cfg(feature = "monitoring")]
            process_last_read_bytes: AtomicU64::new(0),
            #[cfg(feature = "monitoring")]
            process_last_write_bytes: AtomicU64::new(0),
            #[cfg(feature = "monitoring")]
            otel: None,
        }
    }
}

impl ServerMetrics {
    /// 关联 OTel 指标实例 (仅在 monitoring feature 下生效)。
    #[cfg(feature = "monitoring")]
    pub fn with_otel(mut self, otel: Arc<super::otel_metrics::OtelMetrics>) -> Self {
        self.otel = Some(otel);
        self
    }

    /// OTel 镜像句柄 (refresh / info_catalog sync).
    #[cfg(feature = "monitoring")]
    pub fn otel_handle(&self) -> Option<&Arc<super::otel_metrics::OtelMetrics>> {
        self.otel.as_ref()
    }

    pub fn on_connect(&self) {
        self.connections_total.fetch_add(1, Ordering::Relaxed);
        self.connected_clients.fetch_add(1, Ordering::Relaxed);
        #[cfg(feature = "monitoring")]
        if let Some(ref otel) = self.otel {
            otel.on_connect();
        }
    }

    pub fn on_disconnect(&self) {
        self.connected_clients.fetch_sub(1, Ordering::Relaxed);
        #[cfg(feature = "monitoring")]
        if let Some(ref otel) = self.otel {
            otel.on_disconnect();
        }
    }

    /// 客户端进入阻塞命令等待 (BLPOP 等).
    pub fn on_client_blocked(&self) {
        self.blocked_clients.fetch_add(1, Ordering::Relaxed);
        #[cfg(feature = "monitoring")]
        if let Some(ref otel) = self.otel {
            otel.on_client_blocked();
        }
    }

    /// 客户端离开阻塞命令等待.
    pub fn on_client_unblocked(&self) {
        self.blocked_clients.fetch_sub(1, Ordering::Relaxed);
        #[cfg(feature = "monitoring")]
        if let Some(ref otel) = self.otel {
            otel.on_client_unblocked();
        }
    }

    pub fn on_command(&self, command: &str, ok: bool) {
        let mut map = self.commands_total.lock().unwrap();
        let entry = map.entry(command.to_ascii_uppercase()).or_default();
        if ok {
            entry.ok += 1;
        } else {
            entry.err += 1;
        }
        #[cfg(feature = "monitoring")]
        if let Some(ref otel) = self.otel {
            otel.on_command(command, ok);
        }
    }

    pub fn on_lua_command(&self, command: &str, ok: bool) {
        let key = format!("LUA.{}", command.to_ascii_lowercase());
        self.on_command(&key, ok);
        #[cfg(feature = "monitoring")]
        if let Some(ref otel) = self.otel {
            otel.on_lua_scripts();
        }
    }

    pub fn on_lua_execution(&self, duration_us: u64) {
        self.lua_execution_duration_us
            .fetch_add(duration_us, Ordering::Relaxed);
        self.lua_execution_count.fetch_add(1, Ordering::Relaxed);
        #[cfg(feature = "monitoring")]
        if let Some(ref otel) = self.otel {
            let secs = duration_us as f64 / 1_000_000.0;
            otel.on_lua_execution(secs);
        }
    }

    pub fn lua_execution_duration_us(&self) -> u64 {
        self.lua_execution_duration_us.load(Ordering::Relaxed)
    }

    pub fn lua_execution_count(&self) -> u64 {
        self.lua_execution_count.load(Ordering::Relaxed)
    }

    pub fn lua_command_ok_count(&self, command: &str) -> u64 {
        let key = format!("LUA.{}", command.to_ascii_lowercase());
        self.command_ok_count(&key)
    }

    /// 记录命令耗时 (微秒); INFO commandstats 与 OTel histogram 共用.
    pub fn on_command_duration(&self, command: &str, duration_us: u64, ok: bool) {
        {
            let mut map = self.commands_total.lock().unwrap();
            let entry = map.entry(command.to_ascii_uppercase()).or_default();
            entry.usec = entry.usec.saturating_add(duration_us);
        }
        #[cfg(not(feature = "monitoring"))]
        let _ = ok;
        #[cfg(feature = "monitoring")]
        if let Some(ref otel) = self.otel {
            otel.on_command_duration(command, duration_us, ok);
        }
    }

    /// 记录慢查询到 OTel counter (仅 monitoring feature)。
    #[cfg(feature = "monitoring")]
    pub fn on_slow_query(&self, command: &str) {
        if let Some(ref otel) = self.otel {
            otel.on_slow_query(command);
        }
    }

    /// INFO commandstats slowlog_* 字段 (Redis 8.8+).
    pub fn on_slowlog_command(&self, command: &str, duration_us: u64) {
        let ms = duration_us / 1000;
        let mut map = self.commands_total.lock().unwrap();
        let entry = map.entry(command.to_ascii_uppercase()).or_default();
        entry.slowlog_count += 1;
        entry.slowlog_time_ms_sum = entry.slowlog_time_ms_sum.saturating_add(ms);
        entry.slowlog_time_ms_max = entry.slowlog_time_ms_max.max(ms);
    }

    /// 设置 uptime 秒数 (仅 monitoring feature)。
    #[cfg(not(feature = "monitoring"))]
    pub fn set_uptime_secs(&self, _secs: u64) {}

    #[cfg(not(feature = "monitoring"))]
    pub fn set_db_key_count(&self, _db: usize, _count: u64) {}

    #[cfg(not(feature = "monitoring"))]
    pub fn on_net_input_bytes(&self, bytes: u64) {
        if bytes > 0 {
            self.net_input_bytes.fetch_add(bytes, Ordering::Relaxed);
        }
    }

    #[cfg(not(feature = "monitoring"))]
    pub fn on_net_output_bytes(&self, bytes: u64) {
        if bytes > 0 {
            self.net_output_bytes.fetch_add(bytes, Ordering::Relaxed);
        }
    }

    #[cfg(not(feature = "monitoring"))]
    pub fn on_rejected_connection(&self) {
        self.rejected_connections.fetch_add(1, Ordering::Relaxed);
    }

    #[cfg(not(feature = "monitoring"))]
    pub fn on_expired_key(&self) {
        self.expired_keys.fetch_add(1, Ordering::Relaxed);
    }

    #[cfg(not(feature = "monitoring"))]
    pub fn record_expired_keys(&self, count: u64) {
        if count > 0 {
            self.expired_keys.fetch_add(count, Ordering::Relaxed);
        }
    }

    #[cfg(not(feature = "monitoring"))]
    pub fn set_memory_bytes(&self, current: u64) {
        self.used_memory_bytes.store(current, Ordering::Relaxed);
        let prev_peak = self.used_memory_peak_bytes.load(Ordering::Relaxed);
        if current > prev_peak {
            self.used_memory_peak_bytes
                .store(current, Ordering::Relaxed);
        }
    }

    #[cfg(not(feature = "monitoring"))]
    pub fn sample_instantaneous_ops(&self) {
        self.sample_instantaneous_ops_inner();
    }

    #[cfg(feature = "monitoring")]
    pub fn set_uptime_secs(&self, secs: u64) {
        if let Some(ref otel) = self.otel {
            otel.set_uptime_secs(secs);
        }
    }

    /// 设置当前内存使用量 (仅 monitoring feature)。
    #[cfg(feature = "monitoring")]
    pub fn set_memory_bytes(&self, current: u64) {
        self.used_memory_bytes.store(current, Ordering::Relaxed);
        let prev_peak = self.used_memory_peak_bytes.load(Ordering::Relaxed);
        if current > prev_peak {
            self.used_memory_peak_bytes
                .store(current, Ordering::Relaxed);
        }
        if let Some(ref otel) = self.otel {
            let peak = self.used_memory_peak_bytes.load(Ordering::Relaxed);
            otel.set_memory_bytes(current, peak);
        }
    }

    /// 记录过期 key 驱逐 (仅 monitoring feature)。
    #[cfg(feature = "monitoring")]
    pub fn on_expired_key(&self) {
        self.expired_keys.fetch_add(1, Ordering::Relaxed);
        if let Some(ref otel) = self.otel {
            otel.on_expired_key();
        }
    }

    /// 记录被拒绝的连接 (仅 monitoring feature)。
    #[cfg(feature = "monitoring")]
    pub fn on_rejected_connection(&self) {
        self.rejected_connections.fetch_add(1, Ordering::Relaxed);
        if let Some(ref otel) = self.otel {
            otel.on_rejected_connection();
        }
    }

    /// 批量记录过期 key 驱逐 (仅 monitoring feature)。
    #[cfg(feature = "monitoring")]
    pub fn record_expired_keys(&self, count: u64) {
        if count == 0 {
            return;
        }
        self.expired_keys.fetch_add(count, Ordering::Relaxed);
        if let Some(ref otel) = self.otel {
            otel.record_expired_keys(count);
        }
    }

    #[cfg(feature = "monitoring")]
    pub fn sample_instantaneous_ops(&self) {
        self.sample_instantaneous_ops_inner();
    }

    fn sample_instantaneous_ops_inner(&self) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let cmds = self.total_commands_processed();
        let prev_cmds = self.ops_last_commands.swap(cmds, Ordering::Relaxed);
        let prev_time = self.ops_last_sample_secs.swap(now, Ordering::Relaxed);
        if prev_time > 0 && now > prev_time {
            let ops = (cmds.saturating_sub(prev_cmds)) / (now - prev_time);
            self.instantaneous_ops_per_sec.store(ops, Ordering::Relaxed);
            #[cfg(feature = "monitoring")]
            if let Some(ref otel) = self.otel {
                otel.set_instantaneous_ops(ops);
            }
        }
    }

    /// 更新逻辑 DB key 数量 (仅 monitoring feature)。
    #[cfg(feature = "monitoring")]
    pub fn set_db_key_count(&self, db: usize, count: u64) {
        if let Some(ref otel) = self.otel {
            otel.set_db_key_count(db, count);
        }
    }

    /// 记录网络入站字节。
    #[cfg(feature = "monitoring")]
    pub fn on_net_input_bytes(&self, bytes: u64) {
        if bytes == 0 {
            return;
        }
        self.net_input_bytes.fetch_add(bytes, Ordering::Relaxed);
        if let Some(ref otel) = self.otel {
            otel.on_net_input_bytes(bytes);
        }
    }

    /// 刷新进程 RSS / CPU / 磁盘 IO 指标 (仅 monitoring feature)。
    #[cfg(feature = "monitoring")]
    pub fn refresh_process_metrics(&self) {
        let Some(ref otel) = self.otel else {
            return;
        };
        if let Some(bytes) = crate::server::process_metrics::read_resident_memory_bytes() {
            otel.set_process_rss(bytes);
        }
        if let Some((user_secs, sys_secs)) =
            crate::server::process_metrics::read_cpu_user_sys_seconds()
        {
            let user_bits = user_secs.to_bits();
            let sys_bits = sys_secs.to_bits();
            let prev_user = self
                .process_last_cpu_user_seconds_bits
                .swap(user_bits, Ordering::Relaxed);
            let prev_sys = self
                .process_last_cpu_sys_seconds_bits
                .swap(sys_bits, Ordering::Relaxed);
            if prev_user != 0 && prev_sys != 0 {
                let prev_user_secs = f64::from_bits(prev_user);
                let prev_sys_secs = f64::from_bits(prev_sys);
                let user_delta = if user_secs > prev_user_secs {
                    user_secs - prev_user_secs
                } else {
                    0.0
                };
                let sys_delta = if sys_secs > prev_sys_secs {
                    sys_secs - prev_sys_secs
                } else {
                    0.0
                };
                if user_delta > 0.0 || sys_delta > 0.0 {
                    otel.add_process_cpu_delta(user_delta, sys_delta);
                }
            }
        }
        if let Some((read_bytes, write_bytes)) = crate::server::process_metrics::read_io_bytes() {
            let prev_read = self
                .process_last_read_bytes
                .swap(read_bytes, Ordering::Relaxed);
            if prev_read > 0 && read_bytes > prev_read {
                let delta = read_bytes - prev_read;
                otel.add_process_io(delta, 0);
            }
            let prev_write = self
                .process_last_write_bytes
                .swap(write_bytes, Ordering::Relaxed);
            if prev_write > 0 && write_bytes > prev_write {
                let delta = write_bytes - prev_write;
                otel.add_process_io(0, delta);
            }
        }
    }

    /// 记录网络出站字节。
    #[cfg(feature = "monitoring")]
    pub fn on_net_output_bytes(&self, bytes: u64) {
        if bytes == 0 {
            return;
        }
        self.net_output_bytes.fetch_add(bytes, Ordering::Relaxed);
        if let Some(ref otel) = self.otel {
            otel.on_net_output_bytes(bytes);
        }
    }

    /// 记录 gossip 拓扑刷新 (MetaRaft 同步).
    pub fn on_gossip_refresh(&self, known_nodes: usize) {
        self.cluster_messages_sent.fetch_add(1, Ordering::Relaxed);
        if known_nodes > 0 {
            self.cluster_messages_received
                .fetch_add(known_nodes as u64, Ordering::Relaxed);
        }
        self.on_command("GOSSIP.tick", true);
        #[cfg(all(feature = "monitoring", feature = "cluster"))]
        if let Some(ref otel) = self.otel {
            otel.on_gossip_message();
        }
    }

    /// 记录 failover 事件 (仅 cluster feature)。
    pub fn on_failover(&self) {
        self.on_command("CLUSTER.failover", true);
        #[cfg(all(feature = "monitoring", feature = "cluster"))]
        if let Some(ref otel) = self.otel {
            otel.on_failover();
        }
    }

    pub fn on_json_command(&self, command: &str, ok: bool) {
        let key = format!("JSON.{}", command.to_ascii_lowercase());
        self.on_command(&key, ok);
        #[cfg(feature = "monitoring")]
        if let Some(ref otel) = self.otel {
            otel.on_json_command(command);
        }
    }

    /// 集群重定向计数 (aikv_cluster_redirects_total 语义).
    pub fn on_cluster_redirect(&self, redirect_type: &str) {
        let key = format!("CLUSTER.redirect.{}", redirect_type);
        self.on_command(&key, true);
        #[cfg(all(feature = "monitoring", feature = "cluster"))]
        if let Some(ref otel) = self.otel {
            otel.on_cluster_redirect(redirect_type);
        }
    }

    pub fn json_command_ok_count(&self, command: &str) -> u64 {
        let key = format!("JSON.{}", command.to_ascii_lowercase());
        self.command_ok_count(&key)
    }

    pub fn on_bgsave_complete(&self, ok: bool) {
        self.on_command("BGSAVE", ok);
    }

    pub fn on_keyspace_hit(&self) {
        self.keyspace_hits.fetch_add(1, Ordering::Relaxed);
        #[cfg(feature = "monitoring")]
        if let Some(ref otel) = self.otel {
            otel.on_keyspace_hit();
        }
    }

    pub fn net_input_bytes(&self) -> u64 {
        self.net_input_bytes.load(Ordering::Relaxed)
    }

    pub fn net_output_bytes(&self) -> u64 {
        self.net_output_bytes.load(Ordering::Relaxed)
    }

    pub fn on_keyspace_miss(&self) {
        self.keyspace_misses.fetch_add(1, Ordering::Relaxed);
        #[cfg(feature = "monitoring")]
        if let Some(ref otel) = self.otel {
            otel.on_keyspace_miss();
        }
    }

    pub fn keyspace_hits(&self) -> u64 {
        self.keyspace_hits.load(Ordering::Relaxed)
    }

    pub fn keyspace_misses(&self) -> u64 {
        self.keyspace_misses.load(Ordering::Relaxed)
    }

    pub fn connections_total(&self) -> u64 {
        self.connections_total.load(Ordering::Relaxed)
    }

    pub fn connected_clients(&self) -> usize {
        self.connected_clients.load(Ordering::Relaxed)
    }

    pub fn total_commands_processed(&self) -> u64 {
        self.commands_total
            .lock()
            .unwrap()
            .values()
            .map(|t| t.ok + t.err)
            .sum()
    }

    pub fn total_error_replies(&self) -> u64 {
        self.commands_total
            .lock()
            .unwrap()
            .values()
            .map(|t| t.err)
            .sum()
    }

    pub(crate) fn client_command_totals(&self) -> Vec<(String, CommandTotals)> {
        let mut out: Vec<_> = self
            .commands_total
            .lock()
            .unwrap()
            .iter()
            .filter(|(cmd, _)| is_client_command(cmd))
            .map(|(cmd, totals)| (cmd.clone(), *totals))
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    pub fn command_ok_count(&self, command: &str) -> u64 {
        self.commands_total
            .lock()
            .unwrap()
            .get(&command.to_ascii_uppercase())
            .map(|t| t.ok)
            .unwrap_or(0)
    }

    pub fn expired_keys(&self) -> u64 {
        self.expired_keys.load(Ordering::Relaxed)
    }

    pub fn rejected_connections(&self) -> u64 {
        self.rejected_connections.load(Ordering::Relaxed)
    }

    pub fn used_memory_bytes(&self) -> u64 {
        self.used_memory_bytes.load(Ordering::Relaxed)
    }

    pub fn used_memory_peak_bytes(&self) -> u64 {
        self.used_memory_peak_bytes.load(Ordering::Relaxed)
    }

    pub fn evicted_keys(&self) -> u64 {
        self.evicted_keys.load(Ordering::Relaxed)
    }

    pub fn instantaneous_ops_per_sec(&self) -> u64 {
        self.instantaneous_ops_per_sec.load(Ordering::Relaxed)
    }

    pub fn cluster_messages_sent(&self) -> u64 {
        self.cluster_messages_sent.load(Ordering::Relaxed)
    }

    pub fn cluster_messages_received(&self) -> u64 {
        self.cluster_messages_received.load(Ordering::Relaxed)
    }

    pub fn blocked_clients(&self) -> usize {
        self.blocked_clients.load(Ordering::Relaxed)
    }

    /// 同步 Redis 对齐 gauge (blocked_clients 等).
    #[cfg(feature = "monitoring")]
    pub fn sync_redis_aligned_gauges(&self) {
        if let Some(ref otel) = self.otel {
            otel.sync_blocked_clients(self.blocked_clients());
        }
    }

    #[cfg(not(feature = "monitoring"))]
    pub fn sync_redis_aligned_gauges(&self) {}
}
