//! 连接级指标 (Phase 8: Atomic, Phase 17 统一 prometheus 注册)

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
    process_last_cpu_seconds_bits: AtomicU64,
    #[cfg(feature = "monitoring")]
    process_last_read_bytes: AtomicU64,
    #[cfg(feature = "monitoring")]
    process_last_write_bytes: AtomicU64,
    #[cfg(feature = "monitoring")]
    prom: Option<Arc<Metrics>>,
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
            process_last_cpu_seconds_bits: AtomicU64::new(0),
            #[cfg(feature = "monitoring")]
            process_last_read_bytes: AtomicU64::new(0),
            #[cfg(feature = "monitoring")]
            process_last_write_bytes: AtomicU64::new(0),
            #[cfg(feature = "monitoring")]
            prom: None,
        }
    }
}

impl ServerMetrics {
    /// 关联 Prometheus 指标实例 (仅在 monitoring feature 下生效)。
    #[cfg(feature = "monitoring")]
    pub fn with_prometheus(mut self, prom: Arc<Metrics>) -> Self {
        self.prom = Some(prom);
        self
    }

    pub fn on_connect(&self) {
        self.connections_total.fetch_add(1, Ordering::Relaxed);
        self.connected_clients.fetch_add(1, Ordering::Relaxed);
        #[cfg(feature = "monitoring")]
        if let Some(ref p) = self.prom {
            p.kv_connections_total.inc();
            p.kv_connected_clients.inc();
            p.otel.on_connect();
        }
    }

    pub fn on_disconnect(&self) {
        self.connected_clients.fetch_sub(1, Ordering::Relaxed);
        #[cfg(feature = "monitoring")]
        if let Some(ref p) = self.prom {
            p.kv_connected_clients.dec();
            p.otel.on_disconnect();
        }
    }

    /// 客户端进入阻塞命令等待 (BLPOP 等).
    pub fn on_client_blocked(&self) {
        self.blocked_clients.fetch_add(1, Ordering::Relaxed);
        #[cfg(feature = "monitoring")]
        if let Some(ref p) = self.prom {
            p.kv_blocked_clients.inc();
            p.otel.on_client_blocked();
        }
    }

    /// 客户端离开阻塞命令等待.
    pub fn on_client_unblocked(&self) {
        self.blocked_clients.fetch_sub(1, Ordering::Relaxed);
        #[cfg(feature = "monitoring")]
        if let Some(ref p) = self.prom {
            p.kv_blocked_clients.dec();
            p.otel.on_client_unblocked();
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
        if let Some(ref p) = self.prom {
            let status = if ok { "ok" } else { "error" };
            p.kv_commands_total
                .with_label_values(&[&command.to_ascii_uppercase(), status])
                .inc();
            p.otel.on_command(command, ok);
        }
    }

    pub fn on_lua_command(&self, command: &str, ok: bool) {
        let key = format!("LUA.{}", command.to_ascii_lowercase());
        self.on_command(&key, ok);
        #[cfg(feature = "monitoring")]
        if let Some(ref p) = self.prom {
            p.kv_lua_scripts_total.inc();
            p.otel.on_lua_scripts();
        }
    }

    pub fn on_lua_execution(&self, duration_us: u64) {
        self.lua_execution_duration_us
            .fetch_add(duration_us, Ordering::Relaxed);
        self.lua_execution_count.fetch_add(1, Ordering::Relaxed);
        #[cfg(feature = "monitoring")]
        if let Some(ref p) = self.prom {
            let secs = duration_us as f64 / 1_000_000.0;
            p.kv_lua_execution_duration_seconds.observe(secs);
            p.otel.on_lua_execution(secs);
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

    /// 记录命令耗时 (微秒); INFO commandstats 与 Prometheus histogram 共用.
    pub fn on_command_duration(&self, command: &str, duration_us: u64, ok: bool) {
        {
            let mut map = self.commands_total.lock().unwrap();
            let entry = map.entry(command.to_ascii_uppercase()).or_default();
            entry.usec = entry.usec.saturating_add(duration_us);
        }
        #[cfg(not(feature = "monitoring"))]
        let _ = ok;
        #[cfg(feature = "monitoring")]
        if let Some(ref p) = self.prom {
            let status = if ok { "ok" } else { "error" };
            let secs = duration_us as f64 / 1_000_000.0;
            p.kv_command_duration_seconds
                .with_label_values(&[&command.to_ascii_uppercase(), status])
                .observe(secs);
        }
    }

    /// OTel histogram (含 exemplar), 须在 active span 内调用.
    #[cfg(feature = "monitoring")]
    pub fn on_command_duration_otel(&self, command: &str, duration_us: u64, ok: bool) {
        if let Some(ref p) = self.prom {
            p.otel.on_command_duration(
                command,
                duration_us as f64 / 1_000_000.0,
                ok,
            );
        }
    }

    /// 记录慢查询到 Prometheus counter (仅 monitoring feature)。
    #[cfg(feature = "monitoring")]
    pub fn on_slow_query(&self, command: &str) {
        if let Some(ref p) = self.prom {
            p.kv_slow_queries_total
                .with_label_values(&[&command.to_ascii_uppercase()])
                .inc();
            p.otel.on_slow_query(command);
        }
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
        if let Some(ref p) = self.prom {
            p.kv_uptime_seconds.set(secs as i64);
            p.otel.set_uptime_secs(secs);
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
        if let Some(ref p) = self.prom {
            p.kv_used_memory_bytes.set(current as i64);
            let prom_peak = p.kv_used_memory_peak_bytes.get();
            if current as i64 > prom_peak {
                p.kv_used_memory_peak_bytes.set(current as i64);
            }
            let peak = p.kv_used_memory_peak_bytes.get().max(0) as u64;
            p.otel.set_memory_bytes(current, peak);
        }
    }

    /// 记录过期 key 驱逐 (仅 monitoring feature)。
    #[cfg(feature = "monitoring")]
    pub fn on_expired_key(&self) {
        self.expired_keys.fetch_add(1, Ordering::Relaxed);
        if let Some(ref p) = self.prom {
            p.kv_expired_keys_total.inc();
            p.otel.on_expired_key();
        }
    }

    /// 记录被拒绝的连接 (仅 monitoring feature)。
    #[cfg(feature = "monitoring")]
    pub fn on_rejected_connection(&self) {
        self.rejected_connections.fetch_add(1, Ordering::Relaxed);
        if let Some(ref p) = self.prom {
            p.kv_rejected_connections_total.inc();
            p.otel.on_rejected_connection();
        }
    }

    /// 批量记录过期 key 驱逐 (仅 monitoring feature)。
    #[cfg(feature = "monitoring")]
    pub fn record_expired_keys(&self, count: u64) {
        if count == 0 {
            return;
        }
        self.expired_keys.fetch_add(count, Ordering::Relaxed);
        if let Some(ref p) = self.prom {
            p.kv_expired_keys_total.inc_by(count);
            p.otel.record_expired_keys(count);
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
            if let Some(ref p) = self.prom {
                p.kv_instantaneous_ops_per_sec.set(ops as i64);
                p.otel.set_instantaneous_ops(ops);
            }
        }
    }

    /// 更新逻辑 DB key 数量 (仅 monitoring feature)。
    #[cfg(feature = "monitoring")]
    pub fn set_db_key_count(&self, db: usize, count: u64) {
        if let Some(ref p) = self.prom {
            p.kv_db_keys
                .with_label_values(&[&db.to_string()])
                .set(count as i64);
        }
    }

    /// 记录网络入站字节。
    #[cfg(feature = "monitoring")]
    pub fn on_net_input_bytes(&self, bytes: u64) {
        if bytes == 0 {
            return;
        }
        self.net_input_bytes.fetch_add(bytes, Ordering::Relaxed);
        if let Some(ref p) = self.prom {
            p.kv_net_input_bytes_total.inc_by(bytes);
            p.otel.on_net_input_bytes(bytes);
        }
    }

    /// 刷新进程 RSS / CPU / 磁盘 IO 指标 (仅 monitoring feature)。
    #[cfg(feature = "monitoring")]
    pub fn refresh_process_metrics(&self) {
        let Some(ref p) = self.prom else {
            return;
        };
        if let Some(bytes) = crate::server::process_metrics::read_resident_memory_bytes() {
            p.kv_process_resident_memory_bytes.set(bytes as i64);
            p.otel.set_process_rss(bytes);
        }
        if let Some(cpu_secs) = crate::server::process_metrics::read_cpu_seconds() {
            let bits = cpu_secs.to_bits();
            let prev_bits = self
                .process_last_cpu_seconds_bits
                .swap(bits, Ordering::Relaxed);
            if prev_bits != 0 {
                let prev = f64::from_bits(prev_bits);
                if cpu_secs > prev {
                    let delta_ms = ((cpu_secs - prev) * 1000.0).round() as u64;
                    if delta_ms > 0 {
                        p.kv_process_cpu_milliseconds_total.inc_by(delta_ms);
                        p.otel.add_process_cpu_ms(delta_ms);
                    }
                }
            }
        }
        if let Some((read_bytes, write_bytes)) = crate::server::process_metrics::read_io_bytes() {
            let prev_read = self
                .process_last_read_bytes
                .swap(read_bytes, Ordering::Relaxed);
            if prev_read > 0 && read_bytes > prev_read {
                let delta = read_bytes - prev_read;
                p.kv_process_read_bytes_total.inc_by(delta);
                p.otel.add_process_io(delta, 0);
            }
            let prev_write = self
                .process_last_write_bytes
                .swap(write_bytes, Ordering::Relaxed);
            if prev_write > 0 && write_bytes > prev_write {
                let delta = write_bytes - prev_write;
                p.kv_process_write_bytes_total
                    .inc_by(delta);
                p.otel.add_process_io(0, delta);
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
        if let Some(ref p) = self.prom {
            p.kv_net_output_bytes_total.inc_by(bytes);
            p.otel.on_net_output_bytes(bytes);
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
        if let Some(ref p) = self.prom {
            p.kv_gossip_messages_total.inc();
            p.otel.on_gossip_message();
        }
    }

    /// 记录 failover 事件 (仅 cluster feature)。
    pub fn on_failover(&self) {
        self.on_command("CLUSTER.failover", true);
        #[cfg(all(feature = "monitoring", feature = "cluster"))]
        if let Some(ref p) = self.prom {
            p.kv_failover_total.inc();
            p.otel.on_failover();
        }
    }

    pub fn on_json_command(&self, command: &str, ok: bool) {
        let key = format!("JSON.{}", command.to_ascii_lowercase());
        self.on_command(&key, ok);
        #[cfg(feature = "monitoring")]
        if let Some(ref p) = self.prom {
            p.kv_json_commands_total
                .with_label_values(&[&command.to_ascii_uppercase()])
                .inc();
            p.otel.on_json_command(command);
        }
    }

    /// 集群重定向计数 (aikv_cluster_redirects_total 语义).
    pub fn on_cluster_redirect(&self, redirect_type: &str) {
        let key = format!("CLUSTER.redirect.{}", redirect_type);
        self.on_command(&key, true);
        #[cfg(all(feature = "monitoring", feature = "cluster"))]
        if let Some(ref p) = self.prom {
            p.kv_cluster_redirects_total
                .with_label_values(&[redirect_type])
                .inc();
            p.otel.on_cluster_redirect(redirect_type);
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
        if let Some(ref p) = self.prom {
            p.kv_keyspace_hits_total.inc();
            p.otel.on_keyspace_hit();
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
        if let Some(ref p) = self.prom {
            p.kv_keyspace_misses_total.inc();
            p.otel.on_keyspace_miss();
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
        if let Some(ref p) = self.prom {
            p.kv_blocked_clients.set(self.blocked_clients() as i64);
            p.otel.sync_blocked_clients(self.blocked_clients());
        }
    }

    #[cfg(not(feature = "monitoring"))]
    pub fn sync_redis_aligned_gauges(&self) {}
}

/// 统一的 Prometheus 指标集合。
/// `monitoring` feature 下启用，通过 `Arc<Metrics>` 在 Connection 间共享。
#[cfg(feature = "monitoring")]
#[derive(Clone)]
pub struct Metrics {
    pub registry: prometheus::Registry,
    pub otel: Arc<super::otel_metrics::OtelMetrics>,
    // --- AiKv 指标 ---
    pub kv_commands_total: prometheus::IntCounterVec,
    pub kv_command_duration_seconds: prometheus::HistogramVec,
    pub kv_connections_total: prometheus::IntCounter,
    pub kv_connected_clients: prometheus::IntGauge,
    pub kv_rejected_connections_total: prometheus::IntCounter,
    pub kv_used_memory_bytes: prometheus::IntGauge,
    pub kv_used_memory_peak_bytes: prometheus::IntGauge,
    pub kv_keyspace_hits_total: prometheus::IntCounter,
    pub kv_keyspace_misses_total: prometheus::IntCounter,
    pub kv_expired_keys_total: prometheus::IntCounter,
    pub kv_evicted_keys_total: prometheus::IntCounter,
    pub kv_instantaneous_ops_per_sec: prometheus::IntGauge,
    pub kv_blocked_clients: prometheus::IntGauge,
    pub kv_db_keys: prometheus::IntGaugeVec,
    pub kv_net_input_bytes_total: prometheus::IntCounter,
    pub kv_net_output_bytes_total: prometheus::IntCounter,
    pub kv_slow_queries_total: prometheus::IntCounterVec,
    pub kv_uptime_seconds: prometheus::IntGauge,
    pub kv_process_resident_memory_bytes: prometheus::IntGauge,
    pub kv_process_cpu_milliseconds_total: prometheus::IntCounter,
    pub kv_process_read_bytes_total: prometheus::IntCounter,
    pub kv_process_write_bytes_total: prometheus::IntCounter,
    // --- JSON/Lua 指标 ---
    pub kv_lua_scripts_total: prometheus::IntCounter,
    pub kv_lua_execution_duration_seconds: prometheus::Histogram,
    pub kv_json_commands_total: prometheus::IntCounterVec,
    // --- 集群指标 (feature-gated) ---
    #[cfg(feature = "cluster")]
    pub kv_cluster_redirects_total: prometheus::IntCounterVec,
    #[cfg(feature = "cluster")]
    pub kv_gossip_messages_total: prometheus::IntCounter,
    #[cfg(feature = "cluster")]
    pub kv_failover_total: prometheus::IntCounter,
}

#[cfg(feature = "monitoring")]
impl Metrics {
    pub fn new() -> prometheus::Result<Self> {
        use prometheus::Opts;
        let registry = prometheus::Registry::new();
        let kv_commands_total = prometheus::IntCounterVec::new(
            Opts::new(
                "aikv_commands_total",
                "Total commands processed, by command name and status",
            ),
            &["command", "status"],
        )?;
        let kv_command_duration_seconds = prometheus::HistogramVec::new(
            prometheus::HistogramOpts::new(
                "aikv_command_duration_seconds",
                "Command duration in seconds",
            ),
            &["command", "status"],
        )?;
        let kv_connections_total =
            prometheus::IntCounter::new("aikv_connections_total", "Total accepted connections")?;
        let kv_connected_clients =
            prometheus::IntGauge::new("aikv_connected_clients", "Current connected clients")?;
        let kv_rejected_connections_total = prometheus::IntCounter::new(
            "aikv_rejected_connections_total",
            "Total rejected connections",
        )?;
        let kv_used_memory_bytes =
            prometheus::IntGauge::new("aikv_used_memory_bytes", "Current memory usage in bytes")?;
        let kv_used_memory_peak_bytes =
            prometheus::IntGauge::new("aikv_used_memory_peak_bytes", "Peak memory usage in bytes")?;
        let kv_keyspace_hits_total =
            prometheus::IntCounter::new("aikv_keyspace_hits_total", "Total keyspace hits")?;
        let kv_keyspace_misses_total =
            prometheus::IntCounter::new("aikv_keyspace_misses_total", "Total keyspace misses")?;
        let kv_expired_keys_total =
            prometheus::IntCounter::new("aikv_expired_keys_total", "Total expired keys")?;
        let kv_evicted_keys_total =
            prometheus::IntCounter::new("aikv_evicted_keys_total", "Total evicted keys")?;
        let kv_instantaneous_ops_per_sec = prometheus::IntGauge::new(
            "aikv_instantaneous_ops_per_sec",
            "Instantaneous operations per second",
        )?;
        let kv_blocked_clients = prometheus::IntGauge::new(
            "aikv_blocked_clients",
            "Clients blocked on blocking commands (BLPOP etc.)",
        )?;
        let kv_db_keys = prometheus::IntGaugeVec::new(
            prometheus::Opts::new("aikv_db_keys", "Approximate key count per logical DB"),
            &["db"],
        )?;
        let kv_net_input_bytes_total = prometheus::IntCounter::new(
            "aikv_net_input_bytes_total",
            "Total bytes received from clients",
        )?;
        let kv_net_output_bytes_total = prometheus::IntCounter::new(
            "aikv_net_output_bytes_total",
            "Total bytes sent to clients",
        )?;
        let kv_slow_queries_total = prometheus::IntCounterVec::new(
            Opts::new("aikv_slow_queries_total", "Total slow queries by command"),
            &["command"],
        )?;
        let kv_uptime_seconds =
            prometheus::IntGauge::new("aikv_uptime_seconds", "Server uptime in seconds")?;
        let kv_process_resident_memory_bytes = prometheus::IntGauge::new(
            "aikv_process_resident_memory_bytes",
            "Process RSS from /proc/self/status",
        )?;
        let kv_process_cpu_milliseconds_total = prometheus::IntCounter::new(
            "aikv_process_cpu_milliseconds_total",
            "Total process CPU time in milliseconds",
        )?;
        let kv_process_read_bytes_total = prometheus::IntCounter::new(
            "aikv_process_read_bytes_total",
            "Total process disk read bytes",
        )?;
        let kv_process_write_bytes_total = prometheus::IntCounter::new(
            "aikv_process_write_bytes_total",
            "Total process disk write bytes",
        )?;
        let kv_lua_scripts_total =
            prometheus::IntCounter::new("aikv_lua_scripts_total", "Total Lua script executions")?;
        let kv_lua_execution_duration_seconds =
            prometheus::Histogram::with_opts(prometheus::HistogramOpts::new(
                "aikv_lua_execution_duration_seconds",
                "Lua script execution duration in seconds",
            ))?;
        let kv_json_commands_total = prometheus::IntCounterVec::new(
            Opts::new(
                "aikv_json_commands_total",
                "Total JSON commands by sub-command",
            ),
            &["command"],
        )?;

        #[cfg(feature = "cluster")]
        let kv_cluster_redirects_total = prometheus::IntCounterVec::new(
            Opts::new(
                "aikv_cluster_redirects_total",
                "Total cluster redirects by type",
            ),
            &["type"],
        )?;
        #[cfg(feature = "cluster")]
        let kv_gossip_messages_total =
            prometheus::IntCounter::new("aikv_gossip_messages_total", "Total gossip messages")?;
        #[cfg(feature = "cluster")]
        let kv_failover_total =
            prometheus::IntCounter::new("aikv_failover_total", "Total failover events")?;

        registry.register(Box::new(kv_commands_total.clone()))?;
        registry.register(Box::new(kv_command_duration_seconds.clone()))?;
        registry.register(Box::new(kv_connections_total.clone()))?;
        registry.register(Box::new(kv_connected_clients.clone()))?;
        registry.register(Box::new(kv_rejected_connections_total.clone()))?;
        registry.register(Box::new(kv_used_memory_bytes.clone()))?;
        registry.register(Box::new(kv_used_memory_peak_bytes.clone()))?;
        registry.register(Box::new(kv_keyspace_hits_total.clone()))?;
        registry.register(Box::new(kv_keyspace_misses_total.clone()))?;
        registry.register(Box::new(kv_expired_keys_total.clone()))?;
        registry.register(Box::new(kv_evicted_keys_total.clone()))?;
        registry.register(Box::new(kv_instantaneous_ops_per_sec.clone()))?;
        registry.register(Box::new(kv_blocked_clients.clone()))?;
        registry.register(Box::new(kv_db_keys.clone()))?;
        registry.register(Box::new(kv_net_input_bytes_total.clone()))?;
        registry.register(Box::new(kv_net_output_bytes_total.clone()))?;
        registry.register(Box::new(kv_slow_queries_total.clone()))?;
        registry.register(Box::new(kv_uptime_seconds.clone()))?;
        registry.register(Box::new(kv_process_resident_memory_bytes.clone()))?;
        registry.register(Box::new(kv_process_cpu_milliseconds_total.clone()))?;
        registry.register(Box::new(kv_process_read_bytes_total.clone()))?;
        registry.register(Box::new(kv_process_write_bytes_total.clone()))?;
        registry.register(Box::new(kv_lua_scripts_total.clone()))?;
        registry.register(Box::new(kv_lua_execution_duration_seconds.clone()))?;
        registry.register(Box::new(kv_json_commands_total.clone()))?;
        #[cfg(feature = "cluster")]
        registry.register(Box::new(kv_cluster_redirects_total.clone()))?;
        #[cfg(feature = "cluster")]
        registry.register(Box::new(kv_gossip_messages_total.clone()))?;
        #[cfg(feature = "cluster")]
        registry.register(Box::new(kv_failover_total.clone()))?;

        // 注册 AiDb 引擎指标到同一 Registry, 使其可通过 /metrics 抓取
        aidb::metrics::register_into(&registry)?;

        let otel = super::otel_metrics::OtelMetrics::new(&opentelemetry::global::meter("aikv"));

        Ok(Metrics {
            registry,
            otel,
            kv_commands_total,
            kv_command_duration_seconds,
            kv_connections_total,
            kv_connected_clients,
            kv_rejected_connections_total,
            kv_used_memory_bytes,
            kv_used_memory_peak_bytes,
            kv_keyspace_hits_total,
            kv_keyspace_misses_total,
            kv_expired_keys_total,
            kv_evicted_keys_total,
            kv_instantaneous_ops_per_sec,
            kv_blocked_clients,
            kv_db_keys,
            kv_net_input_bytes_total,
            kv_net_output_bytes_total,
            kv_slow_queries_total,
            kv_uptime_seconds,
            kv_process_resident_memory_bytes,
            kv_process_cpu_milliseconds_total,
            kv_process_read_bytes_total,
            kv_process_write_bytes_total,
            kv_lua_scripts_total,
            kv_lua_execution_duration_seconds,
            kv_json_commands_total,
            #[cfg(feature = "cluster")]
            kv_cluster_redirects_total,
            #[cfg(feature = "cluster")]
            kv_gossip_messages_total,
            #[cfg(feature = "cluster")]
            kv_failover_total,
        })
    }
}

#[cfg(feature = "monitoring")]
impl std::fmt::Debug for Metrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Metrics").field("registry", &self.registry).finish()
    }
}
