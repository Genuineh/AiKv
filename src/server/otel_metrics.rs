//! OTel metrics instruments (与 Prometheus 双写; aidb 经 Observable 读 prometheus).

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, OnceLock};

use opentelemetry::metrics::{Counter, Histogram, Meter, UpDownCounter};
use opentelemetry::KeyValue;

const CMD_DURATION_BUCKETS: [f64; 14] = [
    0.005, 0.01, 0.025, 0.05, 0.075, 0.1, 0.25, 0.5, 0.75, 1.0, 2.5, 5.0, 7.5, 10.0,
];

#[allow(dead_code)]
const _: [f64; 14] = CMD_DURATION_BUCKETS;

static GLOBAL_OTEL: OnceLock<Arc<OtelMetrics>> = OnceLock::new();
static KV_DB_KEYS: OnceLock<Arc<prometheus::IntGaugeVec>> = OnceLock::new();

/// 注册 Prometheus `aikv_db_keys` 源, 供 OTel Observable 回调读取.
pub fn register_kv_db_keys_source(gauge: Arc<prometheus::IntGaugeVec>) {
    let _ = KV_DB_KEYS.set(gauge);
}

/// Gauge 类指标快照 (Observable 回调读取).
#[derive(Default)]
struct GaugeSnapshot {
    used_memory_bytes: AtomicI64,
    used_memory_peak_bytes: AtomicI64,
    instantaneous_ops_per_sec: AtomicI64,
    blocked_clients: AtomicI64,
    uptime_seconds: AtomicI64,
    process_resident_memory_bytes: AtomicI64,
}

/// OTel 指标集合, 与 `Metrics` 中 prometheus 字段对应.
pub struct OtelMetrics {
    gauges: Arc<GaugeSnapshot>,
    pub commands_total: Counter<u64>,
    pub command_duration_seconds: Histogram<f64>,
    pub connections_total: Counter<u64>,
    pub connected_clients: UpDownCounter<i64>,
    pub rejected_connections_total: Counter<u64>,
    pub keyspace_hits_total: Counter<u64>,
    pub keyspace_misses_total: Counter<u64>,
    pub expired_keys_total: Counter<u64>,
    pub evicted_keys_total: Counter<u64>,
    pub net_input_bytes_total: Counter<u64>,
    pub net_output_bytes_total: Counter<u64>,
    pub slow_queries_total: Counter<u64>,
    pub process_cpu_milliseconds_total: Counter<u64>,
    pub process_read_bytes_total: Counter<u64>,
    pub process_write_bytes_total: Counter<u64>,
    pub lua_scripts_total: Counter<u64>,
    pub lua_execution_duration_seconds: Histogram<f64>,
    pub json_commands_total: Counter<u64>,
    pub aidb_operation_duration_seconds: Histogram<f64>,
    pub aidb_flush_duration_seconds: Histogram<f64>,
    pub aidb_compaction_duration_seconds: Histogram<f64>,
    pub aidb_backup_duration_seconds: Histogram<f64>,
    #[cfg(feature = "cluster")]
    pub cluster_redirects_total: Counter<u64>,
    #[cfg(feature = "cluster")]
    pub gossip_messages_total: Counter<u64>,
    #[cfg(feature = "cluster")]
    pub failover_total: Counter<u64>,
}

impl OtelMetrics {
    pub fn new(meter: &Meter) -> Arc<Self> {
        let gauges = Arc::new(GaugeSnapshot::default());
        let otel = Arc::new(Self {
            gauges: Arc::clone(&gauges),
            commands_total: meter
                .u64_counter("aikv_commands_total")
                .with_description("Total commands processed, by command name and status")
                .init(),
            command_duration_seconds: meter
                .f64_histogram("aikv_command_duration_seconds")
                .with_description("Command duration in seconds")
                .init(),
            connections_total: meter
                .u64_counter("aikv_connections_total")
                .with_description("Total accepted connections")
                .init(),
            connected_clients: meter
                .i64_up_down_counter("aikv_connected_clients")
                .with_description("Current connected clients")
                .init(),
            rejected_connections_total: meter
                .u64_counter("aikv_rejected_connections_total")
                .with_description("Total rejected connections")
                .init(),
            keyspace_hits_total: meter
                .u64_counter("aikv_keyspace_hits_total")
                .with_description("Total keyspace hits")
                .init(),
            keyspace_misses_total: meter
                .u64_counter("aikv_keyspace_misses_total")
                .with_description("Total keyspace misses")
                .init(),
            expired_keys_total: meter
                .u64_counter("aikv_expired_keys_total")
                .with_description("Total expired keys")
                .init(),
            evicted_keys_total: meter
                .u64_counter("aikv_evicted_keys_total")
                .with_description("Total evicted keys")
                .init(),
            net_input_bytes_total: meter
                .u64_counter("aikv_net_input_bytes_total")
                .with_description("Total bytes received from clients")
                .init(),
            net_output_bytes_total: meter
                .u64_counter("aikv_net_output_bytes_total")
                .with_description("Total bytes sent to clients")
                .init(),
            slow_queries_total: meter
                .u64_counter("aikv_slow_queries_total")
                .with_description("Total slow queries by command")
                .init(),
            process_cpu_milliseconds_total: meter
                .u64_counter("aikv_process_cpu_milliseconds_total")
                .with_description("Total process CPU time in milliseconds")
                .init(),
            process_read_bytes_total: meter
                .u64_counter("aikv_process_read_bytes_total")
                .with_description("Total process disk read bytes")
                .init(),
            process_write_bytes_total: meter
                .u64_counter("aikv_process_write_bytes_total")
                .with_description("Total process disk write bytes")
                .init(),
            lua_scripts_total: meter
                .u64_counter("aikv_lua_scripts_total")
                .with_description("Total Lua script executions")
                .init(),
            lua_execution_duration_seconds: meter
                .f64_histogram("aikv_lua_execution_duration_seconds")
                .with_description("Lua script execution duration in seconds")
                .init(),
            json_commands_total: meter
                .u64_counter("aikv_json_commands_total")
                .with_description("Total JSON commands by sub-command")
                .init(),
            aidb_operation_duration_seconds: meter
                .f64_histogram("aidb_operation_duration_seconds")
                .with_description("DB operation duration in seconds")
                .init(),
            aidb_flush_duration_seconds: meter
                .f64_histogram("aidb_flush_duration_seconds")
                .with_description("MemTable flush duration in seconds")
                .init(),
            aidb_compaction_duration_seconds: meter
                .f64_histogram("aidb_compaction_duration_seconds")
                .with_description("Compaction phase duration in seconds")
                .init(),
            aidb_backup_duration_seconds: meter
                .f64_histogram("aidb_backup_duration_seconds")
                .with_description("Backup operation duration in seconds")
                .init(),
            #[cfg(feature = "cluster")]
            cluster_redirects_total: meter
                .u64_counter("aikv_cluster_redirects_total")
                .with_description("Total cluster redirects by type")
                .init(),
            #[cfg(feature = "cluster")]
            gossip_messages_total: meter
                .u64_counter("aikv_gossip_messages_total")
                .with_description("Total gossip messages")
                .init(),
            #[cfg(feature = "cluster")]
            failover_total: meter
                .u64_counter("aikv_failover_total")
                .with_description("Total failover events")
                .init(),
        });

        register_aikv_observable_gauges(meter, &gauges);
        mirror_aikv_db_keys_gauge(meter);
        register_aidb_observable_gauges(meter);
        let _ = GLOBAL_OTEL.set(Arc::clone(&otel));
        install_aidb_histogram_hooks();
        otel
    }

    fn cmd_attrs(command: &str, status: &str) -> [KeyValue; 2] {
        [
            KeyValue::new("command", command.to_string()),
            KeyValue::new("status", status.to_string()),
        ]
    }

    pub fn on_connect(&self) {
        self.connections_total.add(1, &[]);
        self.connected_clients.add(1, &[]);
    }

    pub fn on_disconnect(&self) {
        self.connected_clients.add(-1, &[]);
    }

    pub fn on_client_blocked(&self) {
        self.gauges
            .blocked_clients
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn on_client_unblocked(&self) {
        self.gauges
            .blocked_clients
            .fetch_sub(1, Ordering::Relaxed);
    }

    pub fn on_command(&self, command: &str, ok: bool) {
        let status = if ok { "ok" } else { "error" };
        let cmd = command.to_ascii_uppercase();
        self.commands_total
            .add(1, &Self::cmd_attrs(&cmd, status));
    }

    pub fn on_command_duration(&self, command: &str, duration_secs: f64, ok: bool) {
        let status = if ok { "ok" } else { "error" };
        let cmd = command.to_ascii_uppercase();
        self.command_duration_seconds.record(
            duration_secs,
            &Self::cmd_attrs(&cmd, status),
        );
    }

    pub fn on_lua_scripts(&self) {
        self.lua_scripts_total.add(1, &[]);
    }

    pub fn on_lua_execution(&self, duration_secs: f64) {
        self.lua_execution_duration_seconds
            .record(duration_secs, &[]);
    }

    pub fn on_slow_query(&self, command: &str) {
        self.slow_queries_total
            .add(1, &[KeyValue::new("command", command.to_ascii_uppercase())]);
    }

    pub fn on_json_command(&self, command: &str) {
        self.json_commands_total
            .add(1, &[KeyValue::new("command", command.to_ascii_uppercase())]);
    }

    pub fn on_rejected_connection(&self) {
        self.rejected_connections_total.add(1, &[]);
    }

    pub fn on_keyspace_hit(&self) {
        self.keyspace_hits_total.add(1, &[]);
    }

    pub fn on_keyspace_miss(&self) {
        self.keyspace_misses_total.add(1, &[]);
    }

    pub fn record_expired_keys(&self, count: u64) {
        if count > 0 {
            self.expired_keys_total.add(count, &[]);
        }
    }

    pub fn on_expired_key(&self) {
        self.expired_keys_total.add(1, &[]);
    }

    pub fn set_uptime_secs(&self, secs: u64) {
        self.gauges
            .uptime_seconds
            .store(secs as i64, Ordering::Relaxed);
    }

    pub fn set_memory_bytes(&self, current: u64, peak: u64) {
        self.gauges
            .used_memory_bytes
            .store(current as i64, Ordering::Relaxed);
        self.gauges
            .used_memory_peak_bytes
            .store(peak as i64, Ordering::Relaxed);
    }

    pub fn set_instantaneous_ops(&self, ops: u64) {
        self.gauges
            .instantaneous_ops_per_sec
            .store(ops as i64, Ordering::Relaxed);
    }

    pub fn on_net_input_bytes(&self, bytes: u64) {
        if bytes > 0 {
            self.net_input_bytes_total.add(bytes, &[]);
        }
    }

    pub fn on_net_output_bytes(&self, bytes: u64) {
        if bytes > 0 {
            self.net_output_bytes_total.add(bytes, &[]);
        }
    }

    pub fn set_process_rss(&self, bytes: u64) {
        self.gauges
            .process_resident_memory_bytes
            .store(bytes as i64, Ordering::Relaxed);
    }

    pub fn add_process_cpu_ms(&self, delta_ms: u64) {
        if delta_ms > 0 {
            self.process_cpu_milliseconds_total.add(delta_ms, &[]);
        }
    }

    pub fn add_process_io(&self, read_delta: u64, write_delta: u64) {
        if read_delta > 0 {
            self.process_read_bytes_total.add(read_delta, &[]);
        }
        if write_delta > 0 {
            self.process_write_bytes_total.add(write_delta, &[]);
        }
    }

    pub fn sync_blocked_clients(&self, count: usize) {
        self.gauges
            .blocked_clients
            .store(count as i64, Ordering::Relaxed);
    }

    #[cfg(feature = "cluster")]
    pub fn on_cluster_redirect(&self, redirect_type: &str) {
        self.cluster_redirects_total
            .add(1, &[KeyValue::new("type", redirect_type.to_string())]);
    }

    #[cfg(feature = "cluster")]
    pub fn on_gossip_message(&self) {
        self.gossip_messages_total.add(1, &[]);
    }

    #[cfg(feature = "cluster")]
    pub fn on_failover(&self) {
        self.failover_total.add(1, &[]);
    }
}

fn register_aikv_observable_gauges(meter: &Meter, gauges: &Arc<GaugeSnapshot>) {
    let g = Arc::clone(gauges);
    meter
        .i64_observable_gauge("aikv_used_memory_bytes")
        .with_callback(move |inst| {
            inst.observe(g.used_memory_bytes.load(Ordering::Relaxed), &[]);
        })
        .init();

    let g = Arc::clone(gauges);
    meter
        .i64_observable_gauge("aikv_used_memory_peak_bytes")
        .with_callback(move |inst| {
            inst.observe(g.used_memory_peak_bytes.load(Ordering::Relaxed), &[]);
        })
        .init();

    let g = Arc::clone(gauges);
    meter
        .i64_observable_gauge("aikv_instantaneous_ops_per_sec")
        .with_callback(move |inst| {
            inst.observe(g.instantaneous_ops_per_sec.load(Ordering::Relaxed), &[]);
        })
        .init();

    let g = Arc::clone(gauges);
    meter
        .i64_observable_gauge("aikv_blocked_clients")
        .with_callback(move |inst| {
            inst.observe(g.blocked_clients.load(Ordering::Relaxed), &[]);
        })
        .init();

    let g = Arc::clone(gauges);
    meter
        .i64_observable_gauge("aikv_uptime_seconds")
        .with_callback(move |inst| {
            inst.observe(g.uptime_seconds.load(Ordering::Relaxed), &[]);
        })
        .init();

    let g = Arc::clone(gauges);
    meter
        .i64_observable_gauge("aikv_process_resident_memory_bytes")
        .with_callback(move |inst| {
            inst.observe(
                g.process_resident_memory_bytes.load(Ordering::Relaxed),
                &[],
            );
        })
        .init();
}

fn mirror_aikv_db_keys_gauge(meter: &Meter) {
    use prometheus::core::Collector;

    meter
        .i64_observable_gauge("aikv_db_keys")
        .with_description("Approximate key count per logical DB")
        .with_callback(|inst| {
            let Some(gauge) = KV_DB_KEYS.get() else {
                return;
            };
            for mf in gauge.collect() {
                for m in mf.get_metric() {
                    let db = label_value(m, "db");
                    inst.observe(
                        m.get_gauge().get_value() as i64,
                        &[KeyValue::new("db", db)],
                    );
                }
            }
        })
        .init();
}

pub fn register_aidb_observable_gauges(meter: &Meter) {
    use aidb::metrics as am;

    meter
        .f64_observable_gauge("aidb_wal_size_bytes")
        .with_callback(|inst| inst.observe(am::WAL_SIZE.get(), &[]))
        .init();

    meter
        .i64_observable_gauge("aidb_memtable_size_bytes")
        .with_callback(|inst| {
            inst.observe(
                am::MEMTABLE_SIZE.with_label_values(&["active"]).get(),
                &[KeyValue::new("state", "active")],
            );
            inst.observe(
                am::MEMTABLE_SIZE.with_label_values(&["frozen"]).get(),
                &[KeyValue::new("state", "frozen")],
            );
        })
        .init();

    meter
        .i64_observable_gauge("aidb_block_cache_size_bytes")
        .with_callback(|inst| inst.observe(am::BLOCK_CACHE_SIZE_BYTES.get() as i64, &[]))
        .init();

    meter
        .i64_observable_gauge("aidb_sequence")
        .with_callback(|inst| inst.observe(am::SEQUENCE.get(), &[]))
        .init();

    meter
        .i64_observable_gauge("aidb_total_key_count")
        .with_callback(|inst| inst.observe(am::TOTAL_KEY_COUNT.get(), &[]))
        .init();

    meter
        .i64_observable_gauge("aidb_backup_size_bytes")
        .with_callback(|inst| inst.observe(am::BACKUP_SIZE_BYTES.get(), &[]))
        .init();

    mirror_aidb_labeled_counters(meter);
    mirror_aidb_sstable_gauges(meter);
    mirror_aidb_simple_counters(meter);
    #[cfg(feature = "cluster")]
    mirror_aidb_raft_counters(meter);
}

fn mirror_aidb_simple_counters(meter: &Meter) {
    use aidb::metrics as am;

    meter
        .u64_observable_counter("aidb_block_cache_hits_total")
        .with_callback(|inst| inst.observe(am::BLOCK_CACHE_HITS_TOTAL.get(), &[]))
        .init();

    meter
        .u64_observable_counter("aidb_block_cache_misses_total")
        .with_callback(|inst| inst.observe(am::BLOCK_CACHE_MISSES_TOTAL.get(), &[]))
        .init();

    meter
        .u64_observable_counter("aidb_bloom_false_positive_total")
        .with_callback(|inst| inst.observe(am::BLOOM_FALSE_POSITIVE_TOTAL.get(), &[]))
        .init();

    meter
        .u64_observable_counter("aidb_flush_total")
        .with_callback(|inst| inst.observe(am::FLUSH_TOTAL.get(), &[]))
        .init();
}

fn mirror_aidb_labeled_counters(meter: &Meter) {
    use aidb::metrics as am;
    use prometheus::core::Collector;

    meter
        .u64_observable_counter("aidb_operations_total")
        .with_callback(|inst| {
            for mf in am::OPERATIONS_TOTAL.collect() {
                for m in mf.get_metric() {
                    let op = label_value(m, "op");
                    inst.observe(m.get_counter().get_value() as u64, &[KeyValue::new("op", op)]);
                }
            }
        })
        .init();

    meter
        .u64_observable_counter("aidb_compaction_total")
        .with_callback(|inst| {
            for mf in am::COMPACTION_TOTAL.collect() {
                for m in mf.get_metric() {
                    let t = label_value(m, "type");
                    inst.observe(m.get_counter().get_value() as u64, &[KeyValue::new("type", t)]);
                }
            }
        })
        .init();

    meter
        .u64_observable_counter("aidb_backup_total")
        .with_callback(|inst| {
            for mf in am::BACKUP_TOTAL.collect() {
                for m in mf.get_metric() {
                    let op = label_value(m, "op");
                    inst.observe(m.get_counter().get_value() as u64, &[KeyValue::new("op", op)]);
                }
            }
        })
        .init();
}

fn mirror_aidb_sstable_gauges(meter: &Meter) {
    use aidb::metrics as am;
    use prometheus::core::Collector;

    meter
        .i64_observable_gauge("aidb_sstable_count")
        .with_callback(|inst| {
            for mf in am::SSTABLE_COUNT.collect() {
                for m in mf.get_metric() {
                    let level = label_value(m, "level");
                    inst.observe(m.get_gauge().get_value() as i64, &[KeyValue::new("level", level)]);
                }
            }
        })
        .init();

    meter
        .i64_observable_gauge("aidb_sstable_size_bytes")
        .with_callback(|inst| {
            for mf in am::SSTABLE_SIZE_BYTES.collect() {
                for m in mf.get_metric() {
                    let level = label_value(m, "level");
                    inst.observe(m.get_gauge().get_value() as i64, &[KeyValue::new("level", level)]);
                }
            }
        })
        .init();
}

#[cfg(feature = "cluster")]
fn mirror_aidb_raft_counters(meter: &Meter) {
    use aidb::cluster::metrics as rm;
    use prometheus::core::Collector;

    meter
        .u64_observable_counter("aidb_raft_rpc_total")
        .with_callback(|inst| {
            for mf in rm::RAFT_RPC_TOTAL.collect() {
                for m in mf.get_metric() {
                    let t = label_value(m, "type");
                    let d = label_value(m, "direction");
                    inst.observe(
                        m.get_counter().get_value() as u64,
                        &[
                            KeyValue::new("type", t),
                            KeyValue::new("direction", d),
                        ],
                    );
                }
            }
        })
        .init();

    meter
        .u64_observable_counter("aidb_raft_log_entries_total")
        .with_callback(|inst| inst.observe(rm::RAFT_LOG_ENTRIES_TOTAL.get(), &[]))
        .init();
}

fn label_value(m: &prometheus::proto::Metric, name: &str) -> String {
    m.get_label()
        .iter()
        .find(|l| l.get_name() == name)
        .map(|l| l.get_value().to_string())
        .unwrap_or_default()
}

fn install_aidb_histogram_hooks() {
    aidb::metrics::set_otel_histogram_hooks(aidb::metrics::OtelHistogramHooks {
        operation_duration: Some(hook_aidb_op_duration),
        flush_duration: Some(hook_aidb_flush_duration),
        compaction_duration: Some(hook_aidb_compaction_duration),
        backup_duration: Some(hook_aidb_backup_duration),
    });
}

fn hook_aidb_op_duration(op: &str, secs: f64) {
    if let Some(o) = GLOBAL_OTEL.get() {
        o.aidb_operation_duration_seconds
            .record(secs, &[KeyValue::new("op", op.to_string())]);
    }
}

fn hook_aidb_flush_duration(secs: f64) {
    if let Some(o) = GLOBAL_OTEL.get() {
        o.aidb_flush_duration_seconds.record(secs, &[]);
    }
}

fn hook_aidb_compaction_duration(phase: &str, secs: f64) {
    if let Some(o) = GLOBAL_OTEL.get() {
        o.aidb_compaction_duration_seconds
            .record(secs, &[KeyValue::new("phase", phase.to_string())]);
    }
}

fn hook_aidb_backup_duration(secs: f64) {
    if let Some(o) = GLOBAL_OTEL.get() {
        o.aidb_backup_duration_seconds.record(secs, &[]);
    }
}

/// 契约测试: 核心 aikv_* 指标名.
pub const AIKV_METRIC_NAMES: &[&str] = &[
    "aikv_commands_total",
    "aikv_command_duration_seconds",
    "aikv_connections_total",
    "aikv_connected_clients",
    "aikv_used_memory_bytes",
    "aikv_keyspace_hits_total",
    "aikv_uptime_seconds",
    "aikv_db_keys",
];
