//! OTel metrics instruments (`aikv_*` 唯一出口, 经 OTLP 导出).
//!
//! TODO(exemplars): metrics↔traces 跳转需 OTel Rust SDK exemplar 采集; 当前 0.32 仍不支持.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

use opentelemetry::metrics::{Counter, Gauge, Histogram, Meter, UpDownCounter};
use opentelemetry::KeyValue;

const CMD_DURATION_BUCKETS: [f64; 14] = [
    0.005, 0.01, 0.025, 0.05, 0.075, 0.1, 0.25, 0.5, 0.75, 1.0, 2.5, 5.0, 7.5, 10.0,
];

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

/// OTel 指标集合.
pub struct OtelMetrics {
    gauges: Arc<GaugeSnapshot>,
    db_keys_gauge: Gauge<f64>,
    commands_total: Counter<u64>,
    command_duration_seconds: Histogram<f64>,
    connections_total: Counter<u64>,
    connected_clients: UpDownCounter<i64>,
    rejected_connections_total: Counter<u64>,
    keyspace_hits_total: Counter<u64>,
    keyspace_misses_total: Counter<u64>,
    expired_keys_total: Counter<u64>,
    evicted_keys_total: Counter<u64>,
    net_input_bytes_total: Counter<u64>,
    net_output_bytes_total: Counter<u64>,
    slow_queries_total: Counter<u64>,
    process_cpu_milliseconds_total: Counter<u64>,
    process_read_bytes_total: Counter<u64>,
    process_write_bytes_total: Counter<u64>,
    lua_scripts_total: Counter<u64>,
    lua_execution_duration_seconds: Histogram<f64>,
    json_commands_total: Counter<u64>,
    #[cfg(feature = "cluster")]
    cluster_redirects_total: Counter<u64>,
    #[cfg(feature = "cluster")]
    gossip_messages_total: Counter<u64>,
    #[cfg(feature = "cluster")]
    failover_total: Counter<u64>,
}

impl std::fmt::Debug for OtelMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OtelMetrics")
    }
}

static GLOBAL_OTEL: OnceLock<RwLock<Option<Arc<OtelMetrics>>>> = OnceLock::new();

fn global_otel() -> &'static RwLock<Option<Arc<OtelMetrics>>> {
    GLOBAL_OTEL.get_or_init(|| RwLock::new(None))
}

impl OtelMetrics {
    /// 初始化 global `aikv` meter instruments (幂等).
    pub fn init_global(meter: Meter) -> Arc<Self> {
        let cell = global_otel();
        if let Some(existing) = cell.read().unwrap().as_ref() {
            return Arc::clone(existing);
        }
        let otel = Self::new(meter);
        *cell.write().unwrap() = Some(Arc::clone(&otel));
        otel
    }

    /// 测试用: 用当前 global meter provider 重建 instruments.
    pub(crate) fn install_global(meter: Meter) -> Arc<Self> {
        let otel = Self::new(meter);
        *global_otel().write().unwrap() = Some(Arc::clone(&otel));
        otel
    }

    pub(crate) fn new(meter: Meter) -> Arc<Self> {
        let gauges = Arc::new(GaugeSnapshot::default());
        let otel = Arc::new(Self {
            db_keys_gauge: meter
                .f64_gauge("aikv_db_keys")
                .with_description("Approximate key count per logical DB")
                .build(),
            gauges: Arc::clone(&gauges),
            commands_total: meter
                .u64_counter("aikv_commands_total")
                .with_description("Total commands processed, by command name and status")
                .build(),
            command_duration_seconds: meter
                .f64_histogram("aikv_command_duration_seconds")
                .with_description("Command duration in seconds")
                .with_boundaries(CMD_DURATION_BUCKETS.to_vec())
                .build(),
            connections_total: meter
                .u64_counter("aikv_connections_total")
                .with_description("Total accepted connections")
                .build(),
            connected_clients: meter
                .i64_up_down_counter("aikv_connected_clients")
                .with_description("Current connected clients")
                .build(),
            rejected_connections_total: meter
                .u64_counter("aikv_rejected_connections_total")
                .with_description("Total rejected connections")
                .build(),
            keyspace_hits_total: meter
                .u64_counter("aikv_keyspace_hits_total")
                .with_description("Total keyspace hits")
                .build(),
            keyspace_misses_total: meter
                .u64_counter("aikv_keyspace_misses_total")
                .with_description("Total keyspace misses")
                .build(),
            expired_keys_total: meter
                .u64_counter("aikv_expired_keys_total")
                .with_description("Total expired keys")
                .build(),
            evicted_keys_total: meter
                .u64_counter("aikv_evicted_keys_total")
                .with_description("Total evicted keys")
                .build(),
            net_input_bytes_total: meter
                .u64_counter("aikv_net_input_bytes_total")
                .with_description("Total bytes received from clients")
                .build(),
            net_output_bytes_total: meter
                .u64_counter("aikv_net_output_bytes_total")
                .with_description("Total bytes sent to clients")
                .build(),
            slow_queries_total: meter
                .u64_counter("aikv_slow_queries_total")
                .with_description("Total slow queries by command")
                .build(),
            process_cpu_milliseconds_total: meter
                .u64_counter("aikv_process_cpu_milliseconds_total")
                .with_description("Total process CPU time in milliseconds")
                .build(),
            process_read_bytes_total: meter
                .u64_counter("aikv_process_read_bytes_total")
                .with_description("Total process disk read bytes")
                .build(),
            process_write_bytes_total: meter
                .u64_counter("aikv_process_write_bytes_total")
                .with_description("Total process disk write bytes")
                .build(),
            lua_scripts_total: meter
                .u64_counter("aikv_lua_scripts_total")
                .with_description("Total Lua script executions")
                .build(),
            lua_execution_duration_seconds: meter
                .f64_histogram("aikv_lua_execution_duration_seconds")
                .with_description("Lua script execution duration in seconds")
                .build(),
            json_commands_total: meter
                .u64_counter("aikv_json_commands_total")
                .with_description("Total JSON commands by sub-command")
                .build(),
            #[cfg(feature = "cluster")]
            cluster_redirects_total: meter
                .u64_counter("aikv_cluster_redirects_total")
                .with_description("Total cluster redirects by type")
                .build(),
            #[cfg(feature = "cluster")]
            gossip_messages_total: meter
                .u64_counter("aikv_gossip_messages_total")
                .with_description("Total gossip messages")
                .build(),
            #[cfg(feature = "cluster")]
            failover_total: meter
                .u64_counter("aikv_failover_total")
                .with_description("Total failover events")
                .build(),
        });

        register_aikv_observable_gauges(&meter, &gauges);
        otel
    }

    fn cmd_attrs(command: &str, status: &str) -> [KeyValue; 2] {
        [
            KeyValue::new("command", command.to_string()),
            KeyValue::new("status", status.to_string()),
        ]
    }

    pub fn set_db_key_count(&self, db: usize, count: u64) {
        self.db_keys_gauge.record(
            count as f64,
            &[KeyValue::new("db", db.to_string())],
        );
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

    pub fn on_command_duration(&self, command: &str, duration_us: u64, ok: bool) {
        let status = if ok { "ok" } else { "error" };
        let cmd = command.to_ascii_uppercase();
        let secs = duration_us as f64 / 1_000_000.0;
        self.command_duration_seconds
            .record(secs, &Self::cmd_attrs(&cmd, status));
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
        .build();

    let g = Arc::clone(gauges);
    meter
        .i64_observable_gauge("aikv_used_memory_peak_bytes")
        .with_callback(move |inst| {
            inst.observe(g.used_memory_peak_bytes.load(Ordering::Relaxed), &[]);
        })
        .build();

    let g = Arc::clone(gauges);
    meter
        .i64_observable_gauge("aikv_instantaneous_ops_per_sec")
        .with_callback(move |inst| {
            inst.observe(g.instantaneous_ops_per_sec.load(Ordering::Relaxed), &[]);
        })
        .build();

    let g = Arc::clone(gauges);
    meter
        .i64_observable_gauge("aikv_blocked_clients")
        .with_callback(move |inst| {
            inst.observe(g.blocked_clients.load(Ordering::Relaxed), &[]);
        })
        .build();

    let g = Arc::clone(gauges);
    meter
        .i64_observable_gauge("aikv_uptime_seconds")
        .with_callback(move |inst| {
            inst.observe(g.uptime_seconds.load(Ordering::Relaxed), &[]);
        })
        .build();

    let g = Arc::clone(gauges);
    meter
        .i64_observable_gauge("aikv_process_resident_memory_bytes")
        .with_callback(move |inst| {
            inst.observe(
                g.process_resident_memory_bytes.load(Ordering::Relaxed),
                &[],
            );
        })
        .build();
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

#[cfg(feature = "monitoring")]
pub mod testutil {
    use std::sync::{Arc, OnceLock};

    use opentelemetry::global;
    use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData};
    use opentelemetry_sdk::metrics::{InMemoryMetricExporter, SdkMeterProvider};

    use super::OtelMetrics;

    static TEST_EXPORTER: OnceLock<InMemoryMetricExporter> = OnceLock::new();
    static TEST_PROVIDER: OnceLock<Arc<SdkMeterProvider>> = OnceLock::new();

    pub fn init_in_memory() -> (InMemoryMetricExporter, Arc<OtelMetrics>) {
        if TEST_EXPORTER.get().is_none() {
            let exporter = InMemoryMetricExporter::default();
            let provider = SdkMeterProvider::builder()
                .with_periodic_exporter(exporter.clone())
                .build();
            global::set_meter_provider(provider.clone());
            let provider = Arc::new(provider);
            let _ = TEST_PROVIDER.set(Arc::clone(&provider));
            let _ = TEST_EXPORTER.set(exporter.clone());
        }
        let exporter = TEST_EXPORTER.get().unwrap().clone();
        let otel = OtelMetrics::install_global(global::meter("aikv"));
        (exporter, otel)
    }

    fn flush() {
        if let Some(provider) = TEST_PROVIDER.get() {
            provider.force_flush().unwrap();
        }
    }

    pub fn counter_sum(exporter: &InMemoryMetricExporter, name: &str) -> u64 {
        flush();
        let metrics = exporter.get_finished_metrics().unwrap();
        let mut total = 0u64;
        for rm in &metrics {
            for sm in rm.scope_metrics() {
                for m in sm.metrics() {
                    if m.name() != name {
                        continue;
                    }
                    if let AggregatedMetrics::U64(MetricData::Sum(sum)) = m.data() {
                        total += sum.data_points().map(|dp| dp.value()).sum::<u64>();
                    }
                }
            }
        }
        total
    }

    pub fn gauge_value(exporter: &InMemoryMetricExporter, name: &str) -> f64 {
        flush();
        let metrics = exporter.get_finished_metrics().unwrap();
        for rm in &metrics {
            for sm in rm.scope_metrics() {
                for m in sm.metrics() {
                    if m.name() != name {
                        continue;
                    }
                    if let AggregatedMetrics::F64(MetricData::Gauge(g)) = m.data() {
                        if let Some(dp) = g.data_points().last() {
                            return dp.value();
                        }
                    }
                    if let AggregatedMetrics::I64(MetricData::Gauge(g)) = m.data() {
                        if let Some(dp) = g.data_points().last() {
                            return dp.value() as f64;
                        }
                    }
                }
            }
        }
        0.0
    }

    pub fn metric_exists(exporter: &InMemoryMetricExporter, name: &str) -> bool {
        flush();
        let metrics = exporter.get_finished_metrics().unwrap();
        metrics.iter().any(|rm| {
            rm.scope_metrics()
                .any(|sm| sm.metrics().any(|m| m.name() == name))
        })
    }
}
