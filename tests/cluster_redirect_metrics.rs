//! @component aikv-cluster
//! MOVED/ASK 不应计入 commandstats (对齐 Redis 8.8 / redis_exporter).
#![cfg(feature = "cluster")]
#![recursion_limit = "512"]
#[path = "modules/cluster/redirect_metrics.rs"]
mod tests;
