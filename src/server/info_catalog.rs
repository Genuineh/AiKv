//! INFO P0 字段 ↔ OTel 同步 catalog (Redis 8.8 基线).
//!
//! `ServerMetrics` 为 INFO 与 OTLP 的共同真源; refresh 周期调用本模块,
//! 将 gauge 快照与 commandstats 不变式与 OTel 镜像对齐 (P2: 热路径仍双写, P3 可收敛).

use crate::server::metrics::ServerMetrics;

#[cfg(feature = "monitoring")]
use crate::server::otel_metrics::OtelMetrics;

/// 从 `ServerMetrics` 同步 OTLP 镜像 (monitoring feature 下生效).
pub fn sync_otel_from_server_metrics(metrics: &ServerMetrics) {
    #[cfg(feature = "monitoring")]
    if let Some(otel) = metrics.otel_handle() {
        otel.sync_stats_gauges(metrics);
        otel.sync_commandstats(metrics);
    }
    #[cfg(not(feature = "monitoring"))]
    let _ = metrics;
}

#[cfg(feature = "monitoring")]
impl OtelMetrics {
    /// 将 observable gauge 快照与 `ServerMetrics` atomics 对齐.
    pub fn sync_stats_gauges(&self, metrics: &ServerMetrics) {
        self.sync_blocked_clients(metrics.blocked_clients());
        self.set_instantaneous_ops(metrics.instantaneous_ops_per_sec());
        self.set_memory_bytes(
            metrics.used_memory_bytes(),
            metrics.used_memory_peak_bytes(),
        );
    }

    /// 校验 INFO commandstats 与 OTel 计数不变式 (P2: debug 断言; P3 可改为 delta sync).
    pub fn sync_commandstats(&self, metrics: &ServerMetrics) {
        let info_calls: u64 = metrics
            .client_command_totals()
            .iter()
            .map(|(_, totals)| totals.ok + totals.err)
            .sum();
        let processed = metrics.total_commands_processed();
        debug_assert_eq!(
            processed, info_calls,
            "total_commands_processed must match sum of client commandstats calls"
        );
        let _ = (info_calls, processed);
    }
}
