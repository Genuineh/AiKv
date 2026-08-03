# AiKv 设计决策

本文回答 **为什么** 这样设计: 选型理由、放弃的替代方案、已知限制. **是什么、怎么分层、数据怎么走** 见 [ARCHITECTURE.md](ARCHITECTURE.md); LSM/Raft/WAL/Checkpoint 等存储内核 why 见 [aidb/DESIGN.md](../aidb/DESIGN.md); 实现细节与入口见 [docs/modules/](docs/modules/).

## 阅读导航

| 域 | 深入阅读 |
|----|----------|
| RESP2/3、Pipeline、解析 limits | [protocol.md](docs/modules/01-protocol.md) |
| TCP 循环、HELLO、内联命令 | [server.md](docs/modules/02-server.md) |
| `KvStorage`、memory/aidb、`StoredValue` | [storage.md](docs/modules/03-storage.md) |
| 核心命令、Router、KeyLock | [commands-core.md](docs/modules/04-commands-core.md) |
| JSON/Lua/SAVE/INFO/MIGRATE | [commands-extended.md](docs/modules/05-commands-extended.md) |
| MOVED/ASK、CLUSTER 子命令 | [cluster.md](docs/modules/06-cluster.md) |
| slowlog、INFO、OTel | [observability.md](docs/modules/07-observability.md) |
| LSM、WAL、MetaRaft/MultiRaft | [aidb engine](../aidb/docs/modules/01-engine.md)、[aidb cluster](../aidb/docs/modules/03-cluster.md) |

## 产品形态与横切取舍

### 为什么是网络服务, 而不是嵌入式 lib?

AiKv 是 **bin + lib** 的 Redis RESP 服务 (Tokio async). [AiDb](../aidb/DESIGN.md) 提供同步 LSM + 可选 Raft; AiKv 在其上实现 TCP、命令语义与 Redis Cluster **客户端协议**. 协议层可独立演进, 存储与共识复用 sibling 库.

### 为什么 Tokio async + `spawn_blocking`?

- **Async**: 高并发 TCP、pipeline 读写在单连接上 multiplex.
- **AiDb 同步 API**: `DB::put/get` 等阻塞; 在 `AiDbEngine` 内用 `spawn_blocking` 桥接, **避免** 在协议层重写 async LSM.
- **trade-off**: blocking pool 线程切换开销; 换来不 fork LSM 实现.

### 为什么 `cluster` / `monitoring` feature gate?

| Feature | 默认 | 理由 |
|---------|------|------|
| (none) | — | 单机二进制无 tonic/OpenRaft/Prometheus 传递依赖 |
| `cluster` | off | 与 `aidb/cluster` 对齐; CI 主测 `--features cluster` |
| `monitoring` | off | 避免默认 OTel/health HTTP 依赖与 export 开销 |

### 与 Redis: 兼容什么, 放弃什么?

**兼容 (目标)**:

- RESP2/RESP3 (HELLO 协商)、标准 **数组形** 命令、BulkString 二进制安全 key.
- 16384 slot、`{hash tag}`、MOVED/ASK、ASKING/READONLY、多数数据结构与 admin 命令子集.
- 集群客户端协作 (`redis-cli -c` 等 smart client).

**刻意不实现或延后 (YAGNI)**:

| 项 | 说明 |
|----|------|
| telnet 内联 (`PING\r\n`) | 仅数组命令; 见 [server.md](docs/modules/02-server.md) |
| 标准 `dump.rdb` / memory AOF | 持久化走 aidb `Checkpoint`; memory 无生产持久化 |
| Redis DUMP 互操作 | 内部 `version + bincode(StoredValue)` |
| `CONFIG REWRITE` | 未实现 |
| Serf / Redis 16379 gossip **共识** | 拓扑与成员变更走 MetaRaft; 见 [集群](#集群) |
| 100% 命令兼容 | stub/未实现命令见各 module「已知限制」 |

### 双引擎: MemoryEngine vs AiDbEngine

- **MemoryEngine**: 测试、无盘开发; **无** checkpoint 生产路径.
- **AiDbEngine**: WAL + LSM; **集群生产推荐** `--engine aidb`.
- 命令层只依赖 `KvStorage`; 切换引擎不改 handler.

---

## 协议

### 为什么 RESP2 + RESP3?

- RESP2: redis-cli 与主流客户端默认.
- RESP3: Map/Set/Push 等; HELLO 在连接级协商.
- **渐进**: 编解码支持全类型; 命令层 RESP3 富类型逐步采用; null 线格式 (`$-1` vs `_`) 在 server `adapt_for_protocol`.

### 为什么 Pipeline + `Ok(None)`?

Redis 性能关键路径. `RespParser::feed` + 循环 `parse()`: 一次 read 多命令; 数据不足 **不消费** buffer. Tokio connection 循环自然 pipeline.

### 为什么 parser / encoder 分离?

`parser.rs` 流式解析与 recoverable 策略; `encoder.rs` 纯 `serialize()`. 职责分离, 与 WiQunTools inventory 01 一致.

### 默认 limits (512MiB bulk / 64MiB buffer / depth 128)

防 OOM 与恶意帧; **无** 运行时 CONFIG 调 limits. 不可恢复错误断连; 可恢复错误 skip 1 byte (server 判定 fatal).

---

## 存储

### 为什么 `KvStorage` trait?

命令层只调 trait; memory / aidb 可切换; 测试注入 `MemoryEngine`. **放弃** oldmain `StorageEngine` enum + 字符串 `get_from_db` — 现 typed 分轨 + `&[u8]` key.

### 为什么 16 逻辑 db (memory)?

Redis `SELECT 0–15`. MemoryEngine 每 db 独立容器 + 过期队列; `FLUSHDB`/`SWAPDB` 隔离.

### 为什么 AiDb 单 `DB` + `{db_index}:{user_key}`?

**演进**: oldmain / wiqun-kv 曾为每逻辑库独立 `DB` 目录; 现 **单 `aidb::DB` + ASCII 前缀** 模拟多库.

- **理由**: 减实例数; cluster 每 **数据 group** 一 `DB` 目录更清晰.
- **trade-off**: scan/sort 在大 keyspace 上 O(n); 见 [storage.md](docs/modules/03-storage.md).

### 为什么 `spawn_blocking` 包装 AiDb?

AiDb `DB` 同步; Tokio 服务不能直接阻塞 runtime. 所有 aidb I/O 经 `AiDbEngine::blocking()`.

### 为什么类型编码在 aikv (`StoredValue`), 不在 aidb?

AiDb 字节 KV + tombstone; **Redis 类型语义** 属协议/命令层. bincode(`StoredValue`) 与内部 DUMP 一致; **非** Redis DUMP.

### 为什么 cluster 写走 `ClusterDataAdapter`?

Assigned slot 写 **必须** `propose_group` → Raft apply; **禁止** local fallback (防 SET 成功 GET 空). 读本地 group 或 `CLUSTERDOWN`. 数据面细节链 [aidb cluster](../aidb/docs/modules/03-cluster.md).

### 已知限制 (摘要)

- MGET 对 non-String key: memory 静默 nil vs aidb WRONGTYPE — 见 [ISSUES.md#ISSUE-001](ISSUES.md#issue-001-memoryengine-mget-对非-string-key-静默返回-none).
- aidb 路径 `scan`/`keys` 全量 sort; memory `write_batch` 非原子.

---

## 命令

### 为什么 `CommandRouter` 为中枢?

单点: `[cluster]` `cluster_route`、metrics、`execute_inner` 分发、KeyLock 注入. **放弃** oldmain `CommandExecutor` 单类 — 域 handler 分文件 + registry 元数据.

### 为什么 KeyLock 字典序?

单 key `lock`; 双 key `lock_two` (a<b); Lua `lock_keys_sorted_with_timeout` 去重排序 (30s 等待超时). **避免** 交叉死锁. 1024 桶 `tokio::Mutex` — 与 oldmain per-key `KeyLockManager` + Condvar 不同; EVAL 路径已恢复 30s 锁等待超时.

### 为什么 JSON 存 `ValueType::String`?

全文档 RMW + `serde_json`; **放弃** RedisJSON 专用存储与 partial update 优化; AiDb 无 JSON 抽象.

### 为什么 Lua: mlua vendored lua54?

Redis 官方脚本语言; 零系统 lua 依赖; 沙箱裁 StdLib. **放弃** rhai/自研 DSL.

### 为什么 `ScriptTransaction` + WriteBatch?

`redis.call` 多写原子提交; 读见己写; 失败 drop buffer. aidb 路径单 `WriteBatch` WAL 原子; memory 路径语义见 [commands-extended.md](docs/modules/05-commands-extended.md).

### 为什么 PING/ECHO/HELLO 在 connection?

减 Router 路径延迟; 行为与走 command 层等价.

### 已知限制 (摘要)

- GETRANGE/SETRANGE — 已实现 (ISSUE-003 closed); MSETNX 未实现 (oldmain 亦无, doc-only ISSUE-004).
- Lua 无 SCRIPT KILL — ISSUE-007.
- SHUTDOWN 仅 SAVE/NOSAVE — ISSUE-011.

---

## 集群

### 为什么 `#[cfg(feature = "cluster")]`?

单机零 cluster 依赖; 与 `aidb/cluster` 一条 feature 对齐.

### 为什么 MetaRaft 共识, 而非 Redis gossip?

| 方案 | 结论 |
|------|------|
| Redis 16379 gossip 共识 | **不采用** — 成员/slot 变更无强一致保证 |
| **MetaRaft + MultiRaft (aidb)** | **选用** — 权威拓扑与数据复制 |
| aikv **轻量拓扑 tick** | 周期性 `ClusterStateManager::refresh()` + gossip metrics; **无** PING/PONG bus; NODES 读 MetaRaft |

WiQunTools inventory 07 中的完整 gossip 故障检测 **未实现**; 故障与成员变更以 Raft 为准. `CLUSTER NODES` 权威源为 MetaRaft, 非 gossip tick.

### 为什么 MOVED/ASK 由客户端处理?

与 **Redis 8.x 官方 Cluster 模型** 一致 ([INFO/cluster 语义](https://redis.io/docs/latest/commands/info/)):

- 服务器对非本地 slot 返回 `-MOVED`/`-ASK` 字符串; **不** 代客户端 TCP 转发.
- smart client (`redis-cli -c` 等) 更新 slot 表并重试.
- **命令统计** 仅在实际执行节点计入 `commandstats`; MOVED 响应节点 **不** 给该命令加 calls.

**放弃:** 服务端透明代理 (`forward_command`). 历史实现偏离 DESIGN, 已移除; 现行行为见 [docs/modules/cluster.md](docs/modules/06-cluster.md).

### 为什么 `ClusterRouter::decide` 同步?

只读 MetaRaft 快照 + 本地 leader 缓存; **不** `.await` OpenRaft. `CLUSTER_STATE_MGR` 全局 `OnceLock` — 热路径零额外 `Arc` 间接 (inventory 07 rationale).

### 为什么 READONLY + replica read?

`READONLY` 后副本上读命令本地执行; 写仍 MOVED leader — Redis 官方行为.

### aidb 与 aikv 分工 (集群)

| 能力 | AiDb | AiKv |
|------|------|------|
| MetaRaft / MultiRaft / Router / slot 迁移执行 | ✅ | `init_cluster` wiring |
| `ClusterDataAdapter` / `propose_group` | Raft 实现 | ✅ 包装写路径 |
| MOVED/ASK / CLUSTER RESP / ASKING | — | ✅ |
| METARAFT * RESP 子命令 | gRPC/内部 API | **已移除** — ISSUE-015 |

### 已知限制 (摘要)

- 不支持 CLUSTER RESET (MetaRaft 共识; 停服清 data_dir 替代) — ISSUE-016 closed doc-only; FAILOVER 仅 FORCE/TAKEOVER — ISSUE-018.
- SET-CONFIG-EPOCH / COUNT-FAILURE-REPORTS stub — ISSUE-019.

---

## 可观测性

**Redis 参考:** Open Source **8.8** (`redis_compatible_version:8.8`; INFO sections + commandstats 行格式). 字段与 OTLP 对照见 [docs/modules/observability.md](docs/modules/07-observability.md) · [observability-reference.md](docs/modules/08-observability-reference.md).

### 为什么 tracing 始终编译, OTel 在 `monitoring` feature?

- **tracing**: 命令/连接 span; 未订阅零开销; 与 aidb 一致.
- **`monitoring`**: OTel traces + metrics (OTLP push) + HTTP **`/health`** (及 `/`); **无** 进程内 Prometheus `/metrics` scrape.

### 为什么 RESP 端口不兼 HTTP metrics?

**RESP 端口不能兼 HTTP**. 健康检查走 `--metrics-addr`/`--metrics-port` (默认 9191); 生产业务指标经 **OTLP → Collector → Prom remote write**, 与 [observability.md](docs/modules/07-observability.md) 一致.

### 为什么 `ServerMetrics` 为 INFO 唯一数据源?

`InfoRenderer` / `CLUSTER INFO` stats 只读 `ServerMetrics` (及 refresh 后 gauge). **OTel `aikv_*` 为 INFO 镜像** — 热路径仅写 `ServerMetrics`; `refresh_runtime_metrics` 经 `info_catalog::sync_otel_from_server_metrics` delta 同步 OTLP (最多滞后 ~15s). **放弃** INFO 与监控双计数 — invariant 见 [observability.md](docs/modules/07-observability.md).

### 与 redis_exporter 的关系

| redis_exporter | AiKV |
|----------------|------|
| pull `INFO` → `redis_*` | push OTLP → `aikv_*` |
| stats 零值字段仍导出 | INFO catalog 固定字段 sync |
| commandstats 仅有 calls 的命令 | 同 |
| 指标名 `redis_*` | 指标名 **`aikv_*`**; reference 文档提供对照表 |

### 为什么 Slowlog 独立于 tracing?

`SLOWLOG GET` 需环形缓冲最近 N 条; tracing span 流式, 不适合「保留 N 条」查询 — inventory 08 与 Redis 行为一致.

### 默认与 refresh (与 Redis/spec 差异)

| 项 | 现默认 | 备注 |
|----|--------|------|
| slowlog 阈值 | **100ms** (`100_000` µs) | Redis/oldmain 10ms — ISSUE-023 |
| metrics refresh | **15s** (`main` 后台 tick) | design spec 1s — ISSUE-022 |
| 无 `monitoring` | 无自动 refresh | stats 可能滞后 — ISSUE-021 |
| `evicted_keys` | 恒 0 | 无 maxmemory eviction |

指标前缀: **`aikv_*`** (非历史 `wiqun_kv_*`). `aidb_*` 不进 Redis INFO, 经同一 OTLP 管道导出.

---

## 性能

### 为什么全局分配器用 mimalloc?

生产环境在 `main.rs` 经 `#[global_allocator]` 使用 **mimalloc** 替代 glibc malloc. 基线来自 eBPF 火焰图 (Grafana Profiles / Pyroscope, 15min 累计), 分配器自身是当时最大 CPU 热点:

| 排名 | Symbol | Self Time | CPU 占比 |
|:----:|--------|:---------:|:--------:|
| 1 | `__libc_malloc` | 8.42 s | 8.1% |
| 2 | `cfree` | 7.37 s | 7.1% |
| 3 | `seccomp_export_bpf` | 3.95 s | 3.8% |
| 4 | `__libc_realloc` | 3.53 s | 3.4% |
| **合计** | malloc + free + realloc | **~19.3 s** | **18.6%** |

**预期收益**: 分配器自身开销 (锁争用、碎片整理) 降低 **30-50%**, 总 CPU 收益 **5-9%**.

### 如何验证 / 对比基准

```bash
redis-benchmark -h 127.0.0.1 -p 6379 -t SET,GET -n 50000 -c 50 -d 64 --cluster
```

对照压测脚本与结果见 [aifactory/benchmark/README.md](../aifactory/benchmark/README.md); 火焰图看板见 [aifactory/monitor/README.md](../aifactory/monitor/README.md).

### 后续优化 (Phase 2)

在验证 Phase 1 收益后, 通过新火焰图定位逻辑层分配热点, 进行 buffer 复用优化 — 待核实, 见 [ISSUES.md](ISSUES.md).

### 已知限制 (摘要)

- 无 SAVE/RDB 之外的 heap 峰值控制; 大 value 突发分配可能抬升 RSS.

---

## 决策总表

| 决策 | 选择 | 理由 | 放弃 / 限制 |
|------|------|------|-------------|
| 产品形态 | RESP 网络服务 | Redis 兼容入口 | 非嵌入式 DB |
| 运行时 | Tokio + spawn_blocking | 高并发 TCP + sync LSM | async 内嵌 LSM |
| 存储抽象 | `KvStorage` trait | 双引擎可切换 | StorageEngine enum |
| AiDb 多库 | 单 DB + key 前缀 | 减实例; cluster 每 group 一库 | 16 独立 DB 目录 |
| 协议 | RESP2 + RESP3 | 客户端覆盖 | telnet 内联 |
| Pipeline | feed + 循环 parse | RTT | — |
| 命令中枢 | CommandRouter | cluster/metrics/KeyLock | CommandExecutor |
| KeyLock | 桶 + 字典序 | 防死锁 | script 30s 超时 (oldmain) |
| JSON | String + serde_json | 简单 | RedisJSON 路径 |
| Lua | mlua vendored | Redis 脚本生态 | rhai |
| 集群 feature | `cfg(cluster)` | 单机精简二进制 | 始终链 cluster |
| 共识 | aidb MetaRaft/MultiRaft | 强一致 | gossip 共识 |
| Gossip | 轻量 MetaRaft 刷新 | metrics/telemetry | PING/PONG 决策 |
| 重定向 | MOVED/ASK 字符串 | Redis 8.8 官方; smart client | 服务端透明代理 |
| 持久化主路径 | aidb Checkpoint | LSM 对齐 | 标准 RDB/AOF |
| DUMP | 内部 bincode | 与 StoredValue 一致 | Redis DUMP |
| 指标 | tracing + 可选 OTel OTLP | INFO 真源镜像 | Prom `/metrics` scrape |
| INFO 兼容 | Redis **8.8** sections + commandstats + 键名 parity | `redis_compatible_version:8.8`; stub 字段见 observability-reference | 7.2 旧基准 / 仅 P0 |
| INFO 数据源 | ServerMetrics | INFO↔OTel 一致 | 双计数 |

---

## 进一步阅读

- [ARCHITECTURE.md](ARCHITECTURE.md) — 分层、数据流、AiDb 边界
- [AGENTS.md](AGENTS.md) — AI 入口与 CI
- [docs/modules/](docs/modules/) — 域级实现与常见任务
- [aidb/DESIGN.md](../aidb/DESIGN.md) — LSM/Raft/Checkpoint why
- [aidb/ARCHITECTURE.md](../aidb/ARCHITECTURE.md) — AiDb 嵌入关系
- [DEPLOYMENT.md](DEPLOYMENT.md) — 构建、feature、运行 (步 21)
- [ISSUES.md](ISSUES.md) — 待核实与跟踪

## 已知限制 (根文档摘要)

- 双引擎 MGET wrong-type 语义不一致 — [ISSUES.md#ISSUE-001](ISSUES.md#issue-001-memoryengine-mget-对非-string-key-静默返回-none).
- 集群 Gossip 与 oldmain 行为差异 — [ISSUES.md](ISSUES.md) (ISSUE-014 等).

## 待核实

- 集群 failover / stub 子命令 — 见 [ISSUES.md](ISSUES.md) (ISSUE-016, ISSUE-019; modules 一行引用).
- 可观测性默认与 metrics 刷新 — 见 [ISSUES.md](ISSUES.md) (ISSUE-020~023).
- mimalloc Phase 2 (逻辑层分配热点 buffer 复用) — 收益未量化, 见 [性能](#性能).
