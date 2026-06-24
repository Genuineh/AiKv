---
name: aikv-observability-reference
depends_on:
  - aikv-observability
description: AiKv OTel aikv_* metric catalog and INFO field cross-reference. Use when looking up metric names, labels, or INFO↔PromQL mapping for aikv observability.
---

# AiKv Observability Reference

> 主文档: [observability.md](observability.md). `aidb_*` → [aidb observability.md](../../../aidb/docs/modules/observability.md).

## OTel Resource 与 Trace

Resource / span 语义见 [observability.md](observability.md) §配置. 全部 instrument 声明 UCUM `unit` (`s`, `By`, `ms`, `1`, `{request}/s`).

PromQL label 来自 OTLP 属性: 点号 `.` 通常映射为下划线 `_` (如 `aikv.command.status` → `aikv_command_status`).

## aikv_* 指标 (`OtelMetrics`, monitoring + 可选 cluster)

生产经 **OTLP** 导出; PromQL 中 `otel_scope_name="aikv"`. 热路径写入点见 `ServerMetrics` → `with_otel`.

| 指标 | 类型 | labels (OTel → Prom) | ServerMetrics 来源 |
|------|------|----------------------|-------------------|
| `aikv_commands_total` | Counter | `aikv.command.name`, `aikv.command.status` | `on_command` |
| `aikv_command_duration_seconds` | Histogram | 同上 | `on_command_duration` |
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
| `aikv_db_keys` | Gauge | `aikv.db.index` | `set_db_key_count` |
| `aikv_net_input_bytes_total` | Counter | — | `on_net_input_bytes` |
| `aikv_net_output_bytes_total` | Counter | — | `on_net_output_bytes` |
| `aikv_slow_queries_total` | Counter | `aikv.command.name` | 超阈值 `on_slow_query` |
| `aikv_uptime_seconds` | Gauge (Observable) | — | refresh |
| `aikv_process_resident_memory_bytes` | Gauge (Observable) | — | `/proc` RSS (legacy) |
| `aikv_process_cpu_milliseconds_total` | Counter | — | `/proc` CPU delta (legacy) |
| `aikv_process_read_bytes_total` | Counter | — | `/proc/io` (legacy) |
| `aikv_process_write_bytes_total` | Counter | — | `/proc/io` (legacy) |
| `process.cpu.time` | Counter | — | 与 legacy CPU 双写 (OTel semconv) |
| `process.memory.usage` | Gauge | `process.memory.state=used` | 与 RSS 双写 |
| `process.disk.io` | Counter | `disk.io.direction` | 与 read/write 双写 |
| `aikv_lua_scripts_total` | Counter | — | `on_lua_command` |
| `aikv_lua_execution_duration_seconds` | Histogram | — | `on_lua_execution` |
| `aikv_json_commands_total` | Counter | `aikv.command.name` | `on_json_command` |
| `aikv_cluster_redirects_total` | Counter | `aikv.cluster.redirect.type` | `on_cluster_redirect` [cluster] |
| `aikv_gossip_messages_total` | Counter | — | `on_gossip_refresh` [cluster] |
| `aikv_failover_total` | Counter | — | `on_failover` [cluster] |

## aidb_* 与 OTel semconv (同一 OTLP 管道)

| 指标 | labels (OTel → Prom) |
|------|----------------------|
| `aidb_operations_total` | `aidb.operation.name` |
| `aidb_operation_duration_seconds` | `aidb.operation.name` |
| `db.client.operations` | `db.system`, `db.operation.name` |
| `db.client.operation.duration` | `db.system`, `db.operation.name` |
| `aidb_compaction_total` | `aidb.compaction.phase` |
| `aidb_compaction_duration_seconds` | `aidb.compaction.phase` |
| `aidb_memtable_size_bytes` | `aidb.memtable.state` |
| `aidb_sstable_*` | `aidb.sstable.level` |
| `aidb_backup_total` | `aidb.backup.operation` |
| `aidb_raft_rpc_total` | `aidb.raft.rpc.type`, `aidb.raft.rpc.direction` |

`aidb_*` (及 `[cluster]` 时 `aidb_raft_*`) 由 aidb OTel Meter 直写, 同一 OTLP 管道出口; HTTP scrape 已移除 (C2.6).
