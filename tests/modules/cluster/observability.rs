//! 集群可观测性 (Phase 3).

#![cfg(feature = "cluster")]

use std::sync::Arc;

use aikv::server::metrics::ServerMetrics;

#[test]
fn gossip_refresh_increments_cluster_message_stats() {
  let metrics = ServerMetrics::default();
  metrics.on_gossip_refresh(3);
  assert_eq!(metrics.cluster_messages_sent(), 1);
  assert_eq!(metrics.cluster_messages_received(), 3);

  metrics.on_gossip_refresh(5);
  assert_eq!(metrics.cluster_messages_sent(), 2);
  assert_eq!(metrics.cluster_messages_received(), 8);
}

#[test]
fn gossip_refresh_with_zero_nodes_only_counts_sent() {
  let metrics = ServerMetrics::default();
  metrics.on_gossip_refresh(0);
  assert_eq!(metrics.cluster_messages_sent(), 1);
  assert_eq!(metrics.cluster_messages_received(), 0);
}

#[test]
fn blocked_clients_defaults_to_zero() {
  let metrics = ServerMetrics::default();
  assert_eq!(metrics.blocked_clients(), 0);
}

#[cfg(feature = "monitoring")]
#[test]
fn sync_redis_aligned_gauges_exports_blocked_clients() {
  let prom = Arc::new(aikv::server::metrics::Metrics::new().expect("metrics registration"));
  let metrics = ServerMetrics::default().with_prometheus(prom.clone());
  metrics.sync_redis_aligned_gauges();
  assert_eq!(prom.kv_blocked_clients.get(), 0);
}
