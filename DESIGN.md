# AiKv 设计决策

本文回答 **为什么** 这样设计: 选型理由、放弃的替代方案、已知限制. **是什么、怎么分层、数据怎么走** 见 [ARCHITECTURE.md](ARCHITECTURE.md); LSM/Raft/WAL/Checkpoint 等存储内核 why 见 [aidb/DESIGN.md](../aidb/DESIGN.md); 实现细节与入口见 [docs/modules/](docs/modules/).

## 阅读导航

| 域 | 深入阅读 |
|----|----------|
| RESP2/3、Pipeline、解析 limits | [protocol.md](docs/modules/protocol.md) |
| TCP 循环、HELLO、内联命令 | [server.md](docs/modules/server.md) |
| `KvStorage`、memory/aidb、`StoredValue` | [storage.md](docs/modules/storage.md) |
| 核心命令、Router、KeyLock | [commands-core.md](docs/modules/commands-core.md) |
| JSON/Lua/SAVE/INFO/MIGRATE | [commands-extended.md](docs/modules/commands-extended.md) |
| MOVED/ASK、CLUSTER 子命令 | [cluster.md](docs/modules/cluster.md) |
| slowlog、INFO、`/metrics` | [observability.md](docs/modules/observability.md) |
| LSM、WAL、MetaRaft/MultiRaft | [aidb engine](../aidb/docs/modules/engine.md)、[aidb cluster](../aidb/docs/modules/cluster.md) |

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
| `monitoring` | off | 避免默认 Prom/OTel 依赖与 scrape 开销 |

### 与 Redis: 兼容什么, 放弃什么?

**兼容 (目标)**:

- RESP2/RESP3 (HELLO 协商)、标准 **数组形** 命令、BulkString 二进制安全 key.
- 16384 slot、`{hash tag}`、MOVED/ASK、ASKING/READONLY、多数数据结构与 admin 命令子集.
- 集群客户端协作 (`redis-cli -c` 等 smart client).

**刻意不实现或延后 (YAGNI)**:

| 项 | 说明 |
|----|------|
| telnet 内联 (`PING\r\n`) | 仅数组命令; 见 [server.md](docs/modules/server.md) |
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
- **trade-off**: scan/sort 在大 keyspace 上 O(n); 见 [storage.md](docs/modules/storage.md).

### 为什么 `spawn_blocking` 包装 AiDb?

AiDb `DB` 同步; Tokio 服务不能直接阻塞 runtime. 所有 aidb I/O 经 `AiDbEngine::blocking()`.

### 为什么类型编码在 aikv (`StoredValue`), 不在 aidb?

AiDb 字节 KV + tombstone; **Redis 类型语义** 属协议/命令层. bincode(`StoredValue`) 与内部 DUMP 一致; **非** Redis DUMP.

### 为什么 cluster 写走 `ClusterDataAdapter`?

Assigned slot 写 **必须** `propose_group` → Raft apply; **禁止** local fallback (防 SET 成功 GET 空). 读本地 group 或 `CLUSTERDOWN`. 数据面细节链 [aidb cluster](../aidb/docs/modules/cluster.md).

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

`redis.call` 多写原子提交; 读见己写; 失败 drop buffer. aidb 路径单 `WriteBatch` WAL 原子; memory 路径语义见 [commands-extended.md](docs/modules/commands-extended.md).

### 为什么 PING/ECHO/HELLO 在 connection?

减 Router 路径延迟; 行为与走 command 层等价.

### 已知限制 (摘要)

- GETRANGE/SETRANGE、MSETNX (cluster dead branch) — ISSUE-003/004.
- Lua 无 SCRIPT KILL; MIGRATE 无 AUTH2 — ISSUE-007/010.
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
| aikv **轻量 Gossip** | 从 MetaRaft 刷新 `GossipState` + metrics; **无** PING/PONG bus |

WiQunTools inventory 07 中的完整 gossip 故障检测 **未实现**; 故障与成员变更以 Raft 为准. 勿将 Gossip 写作 NODES 权威源 — 见 [ISSUES.md#ISSUE-014](ISSUES.md#issue-014-gossipstate-后台刷新但未接入-cluster-nodes).

### 为什么 MOVED/ASK 由客户端处理?

Redis Cluster 协议: 服务器返回 `MOVED`/`ASK` 字符串, smart client 重定向. **放弃** 服务端透明代理 (除 `forward_command` 单端点辅助). 客户端可缓存 slot 表.

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

### 为什么 tracing 始终编译, Prometheus 在 `monitoring` feature?

- **tracing**: 命令/连接 span; 未订阅零开销; 与 aidb 一致.
- **Prometheus + OTel + HTTP `/metrics`**: 可选 feature; 默认无 Prom 依赖.

### 为什么 HTTP `/metrics` 在 aikv 进程?

与 [aidb/DESIGN.md](../aidb/DESIGN.md) 对齐: aidb `register_into`; **RESP 端口不能兼 HTTP**. 进程决定 scrape 端点.

### 为什么 `ServerMetrics` 为 INFO 唯一数据源?

`InfoRenderer` / `CLUSTER INFO` stats 只读 `ServerMetrics` (及 refresh 后 gauge); Prometheus 为 **镜像**. **放弃** INFO 独立计数公式 — invariant 见 [observability.md](docs/modules/observability.md).

### 为什么 Slowlog 独立于 tracing?

`SLOWLOG GET` 需环形缓冲最近 N 条; tracing span 流式, 不适合「保留 N 条」查询 — inventory 08 与 Redis 行为一致.

### 默认与 refresh (与 Redis/spec 差异)

| 项 | 现默认 | 备注 |
|----|--------|------|
| slowlog 阈值 | **100ms** (`100_000` µs) | Redis/oldmain 10ms — ISSUE-023 |
| metrics refresh | **15s** (`main` 后台 tick) | design spec 1s — ISSUE-022 |
| 无 `monitoring` | 无自动 refresh | stats 可能滞后 — ISSUE-021 |
| `blocked_clients` / `evicted_keys` | 恒 0 | 无 maxmemory eviction — ISSUE-020 |

指标前缀: **`aikv_*`** (非历史 `wiqun_kv_*`). `aidb_*` 不进 Redis INFO, 仅 `/metrics`.

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
| 重定向 | MOVED/ASK 字符串 | smart client 兼容 | 默认透明代理 |
| 持久化主路径 | aidb Checkpoint | LSM 对齐 | 标准 RDB/AOF |
| DUMP | 内部 bincode | 与 StoredValue 一致 | Redis DUMP |
| 指标 | tracing + 可选 Prom | 库/进程分离 | 默认 HTTP scrape |
| INFO 数据源 | ServerMetrics | INFO↔Prom 一致 | 双计数 |

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
