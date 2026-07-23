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
| `aikv_process_cpu_milliseconds_total` | Counter | — | `/proc` CPU delta 合计 (legacy) |
| `aikv_process_read_bytes_total` | Counter | — | `/proc/io` (legacy) |
| `aikv_process_write_bytes_total` | Counter | — | `/proc/io` (legacy) |
| `process.cpu.time` | Counter | **`cpu.mode` Required**: `user`, `system` | Prom: `process_cpu_time_seconds_total`, label `cpu_mode` |
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

## INFO ↔ `aikv_*` ↔ redis_exporter (Redis 8.8 基线)

对照基准: Redis Open Source **8.8**; stub/真源策略见 [observability.md](observability.md).

**同步模型 (P3):** 热路径仅写 `ServerMetrics`; `refresh_runtime_metrics` 周期调用 `info_catalog::sync_otel_from_server_metrics` 读真源、算 delta、写 OTel (OTLP 相对 INFO 最多滞后 ~15s). 实现: [`info_catalog.rs`](../../src/server/info_catalog.rs).

### Stats 段

| INFO 字段 | `aikv_*` (OTLP → PromQL) | redis_exporter (参考) | 备注 |
|-----------|--------------------------|----------------------|------|
| `total_commands_processed` | `sum(aikv_commands_total)` | `redis_commands_processed_total` | 仅客户端命令; 不含 MOVED |
| `total_error_replies` | `sum(aikv_commands_total{aikv_command_status="error"})` | 解析 INFO / 部分 exporter 版本 | |
| `keyspace_hits` | `aikv_keyspace_hits_total` | `redis_keyspace_hits_total` | |
| `keyspace_misses` | `aikv_keyspace_misses_total` | `redis_keyspace_misses_total` | |
| `expired_keys` | `aikv_expired_keys_total` | `redis_expired_keys_total` | 需 refresh drain |
| `evicted_keys` | `aikv_evicted_keys_total` | `redis_evicted_keys_total` | AiKv 恒 0 |
| `instantaneous_ops_per_sec` | `aikv_instantaneous_ops_per_sec` | `redis_instantaneous_ops_per_sec` | 15s 采样 |
| `total_net_input_bytes` | `aikv_net_input_bytes_total` | `redis_net_input_bytes_total` | |
| `total_net_output_bytes` | `aikv_net_output_bytes_total` | `redis_net_output_bytes_total` | |
| `rejected_connections` | `aikv_rejected_connections_total` | `redis_rejected_connections_total` | |
| `slowlog_commands_count` | `sum(aikv_slow_queries_total)` (近似) | INFO 解析 (8.8+) | 全局慢命令计数 |
| `slowlog_commands_time_ms_sum/max` | — | INFO 解析 (8.8+) | INFO 真源; 暂无独立 OTel gauge |
| `instantaneous_input_kbps` / `instantaneous_output_kbps` | — | INFO 解析 | 15s 滑动窗口; INFO-only |

### Memory / Clients 段

| INFO 字段 | `aikv_*` | redis_exporter | 备注 |
|-----------|----------|----------------|------|
| `used_memory` | `aikv_used_memory_bytes` | `redis_memory_used_bytes` | refresh 后对齐 |
| `used_memory_peak` | `aikv_used_memory_peak_bytes` | `redis_memory_used_peak_bytes` | |
| `used_memory_rss` | `process.memory.usage` / `aikv_process_resident_memory_bytes` | `redis_memory_used_rss_bytes` | `/proc` RSS |
| `maxmemory` / `maxmemory_policy` | — | INFO 解析 | CONFIG 镜像, 无 OTel gauge |
| `mem_fragmentation_ratio` | — | INFO 解析 | rss/used 计算 |
| `connected_clients` | `aikv_connected_clients` | `redis_connected_clients` | UpDownCounter |
| `blocked_clients` | `aikv_blocked_clients` | `redis_blocked_clients` | BLPOP 等 |
| `maxclients` | — | — | 仅 INFO CONFIG, 无 OTel gauge |

### Errorstats 段 (Redis 8.8)

| INFO 行 | 语义 | 备注 |
|---------|------|------|
| `errorstat_ERR:count=N` | 错误前缀聚合 | AiKv 解析命令错误首 token (非命令名) |
| `errorstat_WRONGTYPE:count=N` | WRONGTYPE 类错误 | 与 Redis 8.8 一致 |

### AiKV-only INFO 字段 (扩展, 段末)

| 字段 | 段 | 说明 |
|------|-----|------|
| `storage_engine` | Server | `memory` / `aidb` |
| `persistent` | Server | 0/1 |
| `cluster_stats_messages_sent/received` | — (仅 `CLUSTER INFO`) | gossip 计数; OTel `aikv_gossip_messages_total` 近似 | **不在** `INFO cluster` (Redis 8.8 INFO 仅 `cluster_enabled`) |

### Stub 字段策略

Redis 8.8 **键名齐全**; 无子系统真源的字段在 INFO 中以 `0` / `-1` / `ok` 输出. **不因客户端负载变化** — 与「当前无 traffic 所以为 0」不同.

| 类别 | 示例 | 说明 |
|------|------|------|
| Pub/Sub / Tracking | `pubsub_*`, `tracking_*`, `watching_clients` | 无 SUBSCRIBE / CLIENT TRACKING 聚合 |
| AOF / 复制网络 | `aof_*`, `total_net_repl_*`, `sync_*` | 无 AOF; 复制 offset stub |
| 内存分配器 | `allocator_*`, `mem_clients_*`, `lazyfree_*` | 非 jemalloc 细账 |
| 事件循环 / ACL | `eventloop_*`, `acl_access_denied_*` | 无等价 eventloop 统计 |
| Persistence 未发生 | `rdb_last_bgsave_time_sec:-1` | Redis 语义: 从未 BGSAVE |

真源接入后 **只改值, 不改键名**; golden `redis88_info_full_fields.txt` 键名集合不变.

### 运行时真源 (负载可验证)

| INFO 段/字段 | 真源 | 备注 |
|--------------|------|------|
| `stats.*` counters | `ServerMetrics` | hits/misses/net/commands |
| `stats.slowlog_commands_*` | 全局慢命令 | 阈值默认 100ms |
| `stats.instantaneous_*_kbps` | 网络滑动窗口 | ~15s refresh |
| `commandstats` / `latencystats` | `ServerMetrics` / `LatencyStats` | MOVED/ASK 不计 |
| `keyspace` | `KvStorage` | AiDb 下比 `used_memory` 更反映 key 数 |
| `memory.used_memory_rss` | `/proc` VmRSS | refresh 缓存 |
| `memory.mem_fragmentation_ratio` | rss/used | ISSUE-024 OTel gauge 仍缺 |

### Commandstats 段 (动态行, Redis 8.8 八字段)

| INFO 子字段 | `aikv_*` | redis_exporter | 备注 |
|-------------|----------|----------------|------|
| `cmdstat_<cmd>:calls` | `aikv_commands_total{aikv_command_name=<CMD>}` (ok+error) | `redis_commands_total{cmd=...}` | MOVED/ASK **不计** |
| `cmdstat_<cmd>:usec` | `aikv_command_duration_seconds` 积分 × 1e6 | INFO 解析 | |
| `cmdstat_<cmd>:rejected_calls` | (P3 catalog) | 6.2+ | AiKv 字段已输出, 常为 0 |
| `cmdstat_<cmd>:failed_calls` | `aikv_commands_total{...,status=error}` | INFO 解析 | |
| `cmdstat_<cmd>:slowlog_count` | `aikv_slow_queries_total{aikv_command_name=<CMD>}` | **8.8+ 可能未解析** | AiKv INFO 先对齐 |
| `slowlog_time_ms_sum/max` | (P3 catalog) | **8.8+ 可能未解析** | |

### Cluster 段 [cluster]

| INFO / 行为 | `aikv_*` | redis_exporter | 备注 |
|-------------|----------|----------------|------|
| MOVED/ASK 响应 | `aikv_cluster_redirects_total{aikv_cluster_redirect_type=moved\|ask}` | 无直接 commandstats | **不** 增加 `cmdstat_*:calls`; 内部键 `CLUSTER.redirect.*` 经 `sync_commandstats` 同步 (大小写不敏感) |
| `cluster_stats_messages_sent/received` (`CLUSTER INFO`) | `aikv_gossip_messages_total` (近似) | cluster INFO | gossip tick; **不在** `INFO` (INFO `# Cluster` 仅 `cluster_enabled`) |
| `total_cluster_links_buffer_limit_exceeded` | — | cluster INFO | Redis 8.8 键名; AiKv 暂 stub `0` |
| `cluster_slot_migration_*` (ASM) | — | cluster INFO | Redis 8.8 常驻; AiKv 暂 stub `0` |
| `cluster_slots_migrating` | — | — | **AiKv-only**: `SlotStatus::Migrating` 计数 |
| Failover | `aikv_failover_total` | — | 无 failover 事件时 Prom 无 series (Grafana 该线可 No data) |

### Server 元数据

| INFO 字段 | OTel | redis_exporter |
|-----------|------|----------------|
| `redis_compatible_version:8.8` | 无 (测试/golden 断言) | N/A |
| `redis_version` | 无 (AiKv 真实版本) | N/A |

**语义注意:** MOVED/ASK 响应节点 **不** 增加 `cmdstat_*:calls` (Redis 8.8). 集群客户端须 `-c` / cluster-aware SDK — 见 [cluster.md](cluster.md).
