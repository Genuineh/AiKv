---
name: aikv-observability-reference
depends_on:
  - aikv-observability
description: AiKv OTel aikv_* metric catalog and INFO field cross-reference. Use when looking up metric names, labels, or INFO↔PromQL mapping for aikv observability.
---

# AiKv Observability Reference

> 主文档: [observability.md](observability.md). `aidb_*` → [aidb observability.md](../../../aidb/docs/modules/observability.md).

## aikv_* 指标 (`OtelMetrics`, monitoring + 可选 cluster)

生产经 **OTLP** 导出; PromQL 中 `otel_scope_name="aikv"`. 热路径写入点见 `ServerMetrics` → `with_otel`.

| 指标 | 类型 | labels | ServerMetrics 来源 |
|------|------|--------|-------------------|
| `aikv_commands_total` | Counter | command, status | `on_command` |
| `aikv_command_duration_seconds` | Histogram | command, status | `on_command_duration` |
| `aikv_connections_total` | Counter | — | `on_connect` |
| `aikv_connected_clients` | UpDownCounter | — | connect/disconnect |
| `aikv_rejected_connections_total` | Counter | — | `on_rejected_connection` |
| `aikv_used_memory_bytes` | Gauge (Observable) | — | `set_memory_bytes` |
| `aikv_used_memory_peak_bytes` | Gauge (Observable) | — | 同上 |
| `aikv_keyspace_hits_total` | Counter | — | `on_keyspace_hit` |
| `aikv_keyspace_misses_total` | Counter | — | `on_keyspace_miss` |
| `aikv_expired_keys_total` | Counter | — | `record_expired_keys` |
| `aikv_evicted_keys_total` | Counter | — | (恒 0) |
| `aikv_instantaneous_ops_per_sec` | Gauge (Observable) | — | `sample_instantaneous_ops` |
| `aikv_blocked_clients` | Gauge (Observable) | — | `on_client_blocked` / guard Drop |
| `aikv_db_keys` | Gauge | db | `set_db_key_count` |
| `aikv_net_input_bytes_total` | Counter | — | `on_net_input_bytes` |
| `aikv_net_output_bytes_total` | Counter | — | `on_net_output_bytes` |
| `aikv_slow_queries_total` | Counter | command | 超阈值 `on_slow_query` |
| `aikv_uptime_seconds` | Gauge (Observable) | — | refresh |
| `aikv_process_resident_memory_bytes` | Gauge (Observable) | — | `/proc` RSS |
| `aikv_process_cpu_milliseconds_total` | Counter | — | `/proc` CPU delta |
| `aikv_process_read_bytes_total` | Counter | — | `/proc/io` |
| `aikv_process_write_bytes_total` | Counter | — | `/proc/io` |
| `aikv_lua_scripts_total` | Counter | — | `on_lua_command` |
| `aikv_lua_execution_duration_seconds` | Histogram | — | `on_lua_execution` |
| `aikv_json_commands_total` | Counter | command | `on_json_command` |
| `aikv_cluster_redirects_total` | Counter | type | `on_cluster_redirect` [cluster] |
| `aikv_gossip_messages_total` | Counter | — | `on_gossip_refresh` [cluster] |
| `aikv_failover_total` | Counter | — | `on_failover` [cluster] |

`aidb_*` (及 `[cluster]` 时 `aidb_raft_*`) 由 aidb OTel Meter 直写, 同一 OTLP 管道出口; HTTP scrape 已移除 (C2.6).
