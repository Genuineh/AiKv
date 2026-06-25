//! INFO 格式化: Redis 语义对齐, 单一数据源 (ServerMetrics).

use crate::server::config::ServerSharedState;
use crate::storage::{KvStorage, StorageEngineKind};

const REDIS_COMPAT_VERSION: &str = "8.8";

/// 集群是否已初始化 (CLUSTER_STATE_MGR 已 set).
pub fn is_cluster_initialized() -> bool {
    #[cfg(feature = "cluster")]
    {
        crate::cluster::state::CLUSTER_STATE_MGR.get().is_some()
    }
    #[cfg(not(feature = "cluster"))]
    {
        false
    }
}

pub fn redis_mode() -> &'static str {
    if is_cluster_initialized() {
        "cluster"
    } else {
        "standalone"
    }
}

pub fn cluster_enabled() -> u8 {
    u8::from(is_cluster_initialized())
}

pub struct InfoRenderer<'a> {
    shared: &'a ServerSharedState,
    storage: &'a dyn KvStorage,
}

impl<'a> InfoRenderer<'a> {
    pub fn new(shared: &'a ServerSharedState, storage: &'a dyn KvStorage) -> Self {
        Self { shared, storage }
    }

    pub async fn render(&self, section: Option<&str>) -> String {
        match section {
            None => self.render_default().await,
            Some("all") | Some("everything") => self.render_all().await,
            Some("server") => self.render_server(),
            Some("clients") => self.render_clients(),
            Some("memory") => self.render_memory().await,
            Some("persistence") => self.render_persistence(),
            Some("stats") => self.render_stats(),
            Some("replication") => self.render_replication(),
            Some("cpu") => self.render_cpu(),
            Some("keyspace") => self.render_keyspace().await,
            Some("cluster") => self.render_cluster_section(),
            Some("commandstats") => self.render_commandstats(),
            Some("errorstats") => self.render_errorstats(),
            Some("modules") => self.render_modules(),
            Some(_) => String::new(),
        }
    }

    async fn render_default(&self) -> String {
        let mut out = String::new();
        append_section(&mut out, &self.render_server());
        append_section(&mut out, &self.render_clients());
        append_section(&mut out, &self.render_memory().await);
        append_section(&mut out, &self.render_persistence());
        append_section(&mut out, &self.render_stats());
        append_section(&mut out, &self.render_replication());
        append_section(&mut out, &self.render_cpu());
        #[cfg(feature = "cluster")]
        append_section(&mut out, &self.render_cluster_section());
        append_section(&mut out, &self.render_keyspace().await);
        out
    }

    async fn render_all(&self) -> String {
        let mut out = self.render_default().await;
        append_section(&mut out, &self.render_commandstats());
        append_section(&mut out, &self.render_errorstats());
        append_section(&mut out, &self.render_modules());
        out
    }

    fn render_server(&self) -> String {
        let engine = match self.shared.engine_kind {
            StorageEngineKind::Memory => "memory",
            StorageEngineKind::AiDb => "aidb",
        };
        let persistent = u8::from(self.shared.engine_kind == StorageEngineKind::AiDb);
        format!(
            "# Server\r\n\
       redis_version:{}\r\n\
       redis_compatible_version:{REDIS_COMPAT_VERSION}\r\n\
       redis_mode:{}\r\n\
       tcp_port:{}\r\n\
       uptime_in_seconds:{}\r\n\
       run_id:{}\r\n\
       storage_engine:{engine}\r\n\
       persistent:{persistent}\r\n",
            env!("CARGO_PKG_VERSION"),
            redis_mode(),
            self.shared.tcp_port,
            self.shared.uptime_secs(),
            self.shared.run_id,
        )
    }

    fn render_clients(&self) -> String {
        format!(
            "# Clients\r\n\
       connected_clients:{}\r\n\
       maxclients:{}\r\n\
       blocked_clients:{}\r\n",
            self.shared.metrics().connected_clients(),
            self.shared.connection_config.max_clients,
            self.shared.metrics().blocked_clients(),
        )
    }

    async fn render_memory(&self) -> String {
        let used_memory = self
            .storage
            .memory_usage_bytes()
            .await
            .unwrap_or_else(|_| self.shared.metrics().used_memory_bytes());
        let peak = self.shared.metrics().used_memory_peak_bytes();
        format!(
            "# Memory\r\n\
       used_memory:{used_memory}\r\n\
       used_memory_human:{}\r\n\
       used_memory_peak:{peak}\r\n\
       used_memory_peak_human:{}\r\n",
            human_bytes(used_memory),
            human_bytes(peak),
        )
    }

    fn render_stats(&self) -> String {
        let m = self.shared.metrics();
        format!(
            "# Stats\r\n\
       total_connections_received:{}\r\n\
       total_commands_processed:{}\r\n\
       instantaneous_ops_per_sec:{}\r\n\
       keyspace_hits:{}\r\n\
       keyspace_misses:{}\r\n\
       expired_keys:{}\r\n\
       evicted_keys:{}\r\n\
       rejected_connections:{}\r\n\
       total_error_replies:{}\r\n\
       total_net_input_bytes:{}\r\n\
       total_net_output_bytes:{}\r\n",
            m.connections_total(),
            m.total_commands_processed(),
            m.instantaneous_ops_per_sec(),
            m.keyspace_hits(),
            m.keyspace_misses(),
            m.expired_keys(),
            m.evicted_keys(),
            m.rejected_connections(),
            m.total_error_replies(),
            m.net_input_bytes(),
            m.net_output_bytes(),
        )
    }

    fn render_replication(&self) -> String {
        let (role, slaves) = replication_snapshot();
        format!(
            "# Replication\r\n\
       role:{role}\r\n\
       connected_slaves:{slaves}\r\n\
       master_replid:{}\r\n\
       master_repl_offset:0\r\n\
       second_repl_offset:-1\r\n\
       repl_backlog_active:0\r\n\
       repl_backlog_size:1048576\r\n\
       repl_backlog_first_byte_offset:0\r\n\
       repl_backlog_histlen:0\r\n",
            self.shared.run_id,
        )
    }

    fn render_cpu(&self) -> String {
        let (user, sys) = cpu_seconds();
        format!(
            "# CPU\r\n\
       used_cpu_sys:{sys:.6}\r\n\
       used_cpu_user:{user:.6}\r\n\
       used_cpu_sys_children:0.000000\r\n\
       used_cpu_user_children:0.000000\r\n"
        )
    }

    fn render_persistence(&self) -> String {
        let in_progress = u8::from(self.shared.bgsave_in_progress());
        let last = self.shared.last_save_time();
        let status = self.shared.last_bgsave_status.read().unwrap().clone();
        format!(
            "# Persistence\r\n\
       loading:0\r\n\
       rdb_changes_since_last_save:0\r\n\
       rdb_bgsave_in_progress:{in_progress}\r\n\
       rdb_last_save_time:{last}\r\n\
       rdb_last_bgsave_time:{last}\r\n\
       rdb_last_bgsave_status:{status}\r\n\
       rdb_current_bgsave_time_sec:-1\r\n\
       rdb_last_cow_size:0\r\n\
       aof_enabled:0\r\n\
       aof_rewrite_in_progress:0\r\n\
       aof_rewrite_scheduled:0\r\n\
       aof_last_rewrite_time_sec:-1\r\n\
       aof_current_rewrite_time_sec:-1\r\n\
       aof_last_bgrewrite_status:ok\r\n\
       aof_last_write_status:ok\r\n"
        )
    }

    async fn render_keyspace(&self) -> String {
        use crate::storage::types::DB_COUNT;

        let mut out = String::from("# Keyspace\r\n");
        let db_count = self.storage.db_count().await.unwrap_or(0).min(DB_COUNT);
        for db in 0..db_count {
            if let Ok(stats) = self.storage.keyspace_stats(db).await {
                out.push_str(&format!(
                    "db{db}:keys={},expires={},avg_ttl={}\r\n",
                    stats.keys, stats.expires, stats.avg_ttl
                ));
            }
        }
        out
    }

    fn render_cluster_section(&self) -> String {
        let m = self.shared.metrics();
        format!(
            "# Cluster\r\n\
       cluster_enabled:{}\r\n\
       cluster_stats_messages_sent:{}\r\n\
       cluster_stats_messages_received:{}\r\n",
            cluster_enabled(),
            m.cluster_messages_sent(),
            m.cluster_messages_received(),
        )
    }

    fn render_commandstats(&self) -> String {
        let mut out = String::from("# Commandstats\r\n");
        for (cmd, totals) in self.shared.metrics().client_command_totals() {
            let calls = totals.ok + totals.err;
            if calls == 0 {
                continue;
            }
            let usec_per_call = if calls > 0 {
                totals.usec as f64 / calls as f64
            } else {
                0.0
            };
            out.push_str(&format!(
                "cmdstat_{}:calls={calls},usec={},usec_per_call={usec_per_call:.2},\
                 rejected_calls={},failed_calls={},\
                 slowlog_count={},slowlog_time_ms_sum={},slowlog_time_ms_max={}\r\n",
                cmd.to_ascii_lowercase(),
                totals.usec,
                totals.rejected,
                totals.err,
                totals.slowlog_count,
                totals.slowlog_time_ms_sum,
                totals.slowlog_time_ms_max,
            ));
        }
        out
    }

    fn render_errorstats(&self) -> String {
        let mut out = String::from("# Errorstats\r\n");
        for (cmd, totals) in self.shared.metrics().client_command_totals() {
            if totals.err == 0 {
                continue;
            }
            out.push_str(&format!(
                "errorstat_{}:count={}\r\n",
                cmd.to_ascii_lowercase(),
                totals.err,
            ));
        }
        out
    }

    fn render_modules(&self) -> String {
        "# Modules\r\n".to_string()
    }
}

fn append_section(out: &mut String, section: &str) {
    if section.is_empty() {
        return;
    }
    if !out.is_empty() {
        out.push_str("\r\n");
    }
    out.push_str(section);
}

fn human_bytes(bytes: u64) -> String {
    if bytes >= 1_048_576 {
        format!("{:.2}M", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.2}K", bytes as f64 / 1024.0)
    } else {
        format!("{bytes}B")
    }
}

fn replication_snapshot() -> (&'static str, u64) {
    #[cfg(feature = "cluster")]
    {
        if let Some(mgr) = crate::cluster::state::CLUSTER_STATE_MGR.get() {
            let meta = mgr.meta_raft.get_cluster_meta();
            return (
                crate::cluster::replication::node_replication_role(&meta, mgr.node_id),
                crate::cluster::replication::connected_slaves_count(&meta, mgr.node_id),
            );
        }
    }
    ("master", 0)
}

fn cpu_seconds() -> (f64, f64) {
    if let Some((user, sys)) = crate::server::process_metrics::read_cpu_user_sys_seconds() {
        return (user, sys);
    }
    (0.0, 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redis_mode_standalone_without_cluster_mgr() {
        assert_eq!(redis_mode(), "standalone");
        assert_eq!(cluster_enabled(), 0);
    }
}
