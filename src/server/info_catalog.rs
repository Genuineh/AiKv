//! INFO P0 字段 ↔ OTel 同步 catalog (Redis 8.8 基线).
//!
//! `ServerMetrics` 为 INFO 与 OTLP 的共同真源; refresh 周期调用本模块,
//! 读真源、算 delta、写入 OTel 镜像 (P3: 热路径仅写 ServerMetrics).

use crate::server::metrics::ServerMetrics;

/// 从 `ServerMetrics` 同步 OTLP 镜像 (monitoring feature 下生效).
pub fn sync_otel_from_server_metrics(metrics: &ServerMetrics) {
    #[cfg(feature = "monitoring")]
    if let Some(otel) = metrics.otel_handle() {
        otel.sync_stats_gauges(metrics);
        otel.sync_counters(metrics);
        otel.sync_commandstats(metrics);
    }
    #[cfg(not(feature = "monitoring"))]
    let _ = metrics;
}
