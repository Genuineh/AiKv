# AiKv — 待核实与问题跟踪

> 位于 aikv 仓库根目录. module 内 **一行引用** 本文件条目 (见 `AiKv-Workflow/backup/design.md` 模板).

**图例**: 状态 = `open` | `confirmed-bug` | `doc-only` | `closed`

---

## 如何使用

1. 文档整理 **步 2–3** 发现设计偏离、实现疑点、oldmain 行为差异时, 在此新增条目.
2. 在对应 module 的 **「待核实」** 小节写: `见 ISSUES.md#ISSUE-NNN — 一句话`
3. 文档整理 **不阻塞** 于修复; 确认要修的 bug 另开开发任务.
4. 关闭条目时更新状态, 必要时回写 module 删除或改写引用.

整理流程中新增 ISSUES 条目, 须在 **步 2–3 确认门控** 内讨论后再写入.

---

## 条目模板 (复制后填写)

```markdown
### ISSUE-NNN: 标题

- **状态**: open
- **发现于**: PROGRESS 步 N / 章节 `docs/modules/xxx.md`
- **相关 src**: `src/...`
- **旧文档**: `aikv-oldmain/docs/...` (可选)
- **oldmain 代码**: `aikv-oldmain/src/...` (可选)
- **现象**: 当前实现 vs 旧设计/旧代码 的差异
- **影响**: 文档应如何描述 / 是否可能是 bug
- **下一步**: 待核实 | 需写测试 | 需开 issue 修代码
```

---

## 条目列表

<!-- 按 ISSUE-NNN 倒序追加 -->

### ISSUE-023: Slowlog 默认阈值 100ms vs Redis/oldmain 10ms

- **状态**: closed (doc-only)
- **发现于**: PROGRESS 步 12 / 章节 `docs/modules/observability.md` (步 3)
- **相关 src**: `src/server/slowlog.rs` (`DEFAULT_SLOWLOG_THRESHOLD_US = 100_000`)
- **oldmain 代码**: `aikv-oldmain/src/observability/logging.rs` — 默认 10_000 µs (10ms, 近 Redis)
- **现象**: 现默认 100ms; Redis `slowlog-log-slower-than` 默认 10000 µs; CONFIG GET 返回 100000
- **修复**: doc-only — module 写实际默认 100ms; 改默认另开开发任务
- **影响**: 新部署记录的慢查询更少

### ISSUE-022: metrics refresh 周期 15s vs 设计 spec 1s

- **状态**: closed (doc-only)
- **发现于**: PROGRESS 步 12 / 章节 `docs/modules/observability.md` (步 2)
- **相关 src**: `src/main.rs` (15s interval), `src/server/config.rs` (`refresh_runtime_metrics`)
- **旧文档**: `backup/aikv/docs/superpowers/specs/2026-06-10-redis-observability-alignment-design.md` §R4 写 1s
- **现象**: 设计 spec 要求 refresh 与 ops/sec 窗口 1s; 现 `tokio::time::interval(Duration::from_secs(15))`
- **修复**: doc-only — module 以代码 15s 为准
- **影响**: `instantaneous_ops_per_sec` 采样粒度粗于 spec

### ISSUE-021: `refresh_runtime_metrics` 仅 monitoring 后台 tick

- **状态**: closed (doc-only)
- **发现于**: PROGRESS 步 12 / 章节 `docs/modules/observability.md` (步 2)
- **相关 src**: `src/main.rs` (`#[cfg(feature = "monitoring")]` 15s loop), `src/server/config.rs` (`refresh_runtime_metrics`)
- **现象**: 无 `monitoring` feature 时不启后台 refresh; `StorageObservation` drain、ops/sec sample、Prometheus gauge 同步不自动运行
- **修复**: doc-only — module 写 refresh 条件 (`monitoring` feature + 15s tick)
- **影响**: 无 monitoring 构建下 INFO `stats.expired_keys` / `instantaneous_ops_per_sec` 可能滞后; memory/keyspace 段仍可直接查 `KvStorage`

### ISSUE-020: `blocked_clients` 无写入点

- **状态**: closed
- **发现于**: PROGRESS 步 12 / 章节 `docs/modules/observability.md` (步 2)
- **相关 src**: `src/server/metrics.rs` (`blocked_clients`), `src/command/blocking.rs` (`BlockedClientGuard`), `src/command/list.rs`, `src/command/zset.rs`
- **现象**: `AtomicUsize blocked_clients` 仅 default 0; BlockingRegistry 等待/唤醒未同步计数
- **修复**: `BlockedClientGuard` 在阻塞命令 handler 进入等待时 +1, Drop 时 -1; `ListCommands`/`ZSetCommands` 经 router 接入 `ServerMetrics`
- **影响**: INFO clients / `aikv_blocked_clients` 反映当前阻塞客户端数

### ISSUE-019: SET-CONFIG-EPOCH / COUNT-FAILURE-REPORTS 为 stub

- **状态**: closed (doc-only)
- **发现于**: PROGRESS 步 11 / 章节 `docs/modules/cluster.md` (步 2–3)
- **相关 src**: `src/cluster/commands.rs` (`dispatch_cluster`)
- **oldmain 代码**: `aikv-oldmain/src/cluster/commands.rs` — `SET-CONFIG-EPOCH` 有实现; `COUNT-FAILURE-REPORTS` 部分逻辑
- **现象**: 现码 `SET-CONFIG-EPOCH` 恒 OK; `COUNT-FAILURE-REPORTS` 恒 0
- **修复**: doc-only — stub 表与「未实现 / stub」节见 [cluster.md](docs/modules/cluster.md)
- **影响**: redis-cli 部分检查路径可能跳过

### ISSUE-018: CLUSTER FAILOVER 仅 FORCE/TAKEOVER 手动升主

- **状态**: closed (doc-only)
- **发现于**: PROGRESS 步 11 / 章节 `docs/modules/cluster.md` (步 2–3)
- **相关 src**: `src/cluster/commands.rs` (`cluster_failover`)
- **旧文档**: `aikv-oldmain/docs/development/api/02-cluster-api.md` — 称 openraft 自动故障切换
- **oldmain 代码**: `FailoverMode::Default|Force|Takeover`; 部分路径经 MetaRaft leader 转发
- **现象**: 现码仅 `FORCE|TAKEOVER`; replica 上 `change_group_membership` 升主; 无 Redis 标准选举等待
- **修复**: doc-only — 手动 failover 模型见 [cluster.md](docs/modules/cluster.md#手动-failover)
- **影响**: 与 Redis 全兼容 FAILOVER 语义不同

### ISSUE-017: CLUSTER REPLICATE 仅本地 ReplicationRole 元数据

- **状态**: closed (doc-only)
- **发现于**: PROGRESS 步 11 / 章节 `docs/modules/cluster.md` (步 2–3)
- **相关 src**: `src/cluster/commands.rs` (`cluster_replicate`, `cluster_failover`)
- **oldmain 代码**: oldmain `cluster_replicate` 调 membership/MultiRaft 路径更完整
- **现象**: 现码只写 `ReplicationRole::Replica { primary_id }`; 注释声明 replicas 不服务数据读取
- **修复**: doc-only — REPLICATE 仅本地元数据; replica 读依赖 `READONLY` + 本地 group, 见 [cluster.md](docs/modules/cluster.md)
- **影响**: FAILOVER 前需 REPLICATE

### ISSUE-016: CLUSTER RESET 未实现

- **状态**: closed (doc-only)
- **发现于**: PROGRESS 步 11 / 章节 `docs/modules/cluster.md` (步 2)
- **相关 src**: `src/cluster/commands.rs` (`dispatch_cluster` RESET arm)
- **旧文档**: `aikv-oldmain/docs/development/architecture/03-cluster.md` §排障 — `CLUSTER RESET SOFT`
- **现象**: MetaRaft 架构下集群元数据为共识状态, 单节点 RESET 无法等价 Redis 语义; oldmain `cluster_reset()` 亦未接入 dispatch (SOFT 空操作, HARD 仅部分清 slot)
- **修复**: doc-only — `CLUSTER RESET` 返回明确 ERR + 替代运维步骤见 [cluster.md](docs/modules/cluster.md#重置集群无-cluster-reset)
- **影响**: redis-cli / 旧脚本需改用停服 + 清 `data_dir` 重搭 (e2e 同模式)

### ISSUE-015: CLUSTER METARAFT * 子命令已移除

- **状态**: closed (doc-only)
- **发现于**: PROGRESS 步 11 / 章节 `docs/modules/cluster.md` (步 2–3)
- **相关 src**: (无 aikv RESP 层)
- **旧文档**: `aikv-oldmain/docs/development/api/02-cluster-api.md`, `03-cluster.md` §METARAFT
- **oldmain 代码**: `aikv-oldmain/src/server/connection.rs` — ADDLEARNER/PROMOTE/MEMBERS/STATUS/SETSTATUS
- **现象**: 重构后 MetaRaft 运维不在 aikv CLUSTER 子命令暴露; 由 aidb gRPC/内部 API 承担
- **修复**: doc-only — METARAFT 运维见 [aidb cluster.md](../aidb/docs/modules/cluster.md); aikv 无 RESP 子命令
- **影响**: 勿写 aikv 支持 METARAFT

### ISSUE-014: GossipState 后台刷新但未接入 CLUSTER NODES

- **状态**: closed
- **发现于**: PROGRESS 步 11 / 章节 `docs/modules/cluster.md` (步 2–3)
- **相关 src**: `src/cluster/gossip.rs`, `src/main.rs`, `src/cluster/commands.rs` (`cluster_nodes`)
- **旧文档**: WiQunTools `07-cluster-protocol.md` — Gossip 供 NODES timestamps; `AGENTS.md` 曾称刷新 NODES 展示
- **现象**: `start_background_refresh` 曾写 unused `GossipState`; `cluster_nodes()` 只读 MetaRaft; 无 PING/PONG 网络
- **修复**: 移除 dead `GossipState` 缓存; tick 保留 leader 路由缓存 + gossip metrics; `CLUSTER NODES` 继续读 MetaRaft, link-state 恢复 `NodeStatus` → connected/disconnected; ping/pong 仍为 `0 0` (与 oldmain 一致)
- **测试**: `cluster_nodes_link_state_from_meta_status`
- **下一步**: —

### ISSUE-013: CLUSTER INFO 恒输出 cluster_state:ok

- **状态**: closed
- **发现于**: PROGRESS 步 11 / 章节 `docs/modules/cluster.md` (步 3)
- **相关 src**: `src/cluster/commands.rs` (`cluster_info`, `compute_cluster_state`)
- **oldmain 代码**: `aikv-oldmain/src/cluster/commands.rs` — slot 未满 16384 或无 leader 时 `cluster_state:fail`
- **现象**: 曾硬编码 `cluster_state:ok\n`; wiqun-kv 同形
- **修复**: 恢复动态 `ok`/`fail` — 16384 slot 全覆盖 (含 Migrating) + 持有 slot 的 group 有 leader (MetaRaft `is_leader` + 本地 MultiRaft metrics) + slot 映射指向已知 group
- **测试**: `cluster_info_state_tests` (partial slots / no leader / healthy / orphan / migrating)
- **下一步**: —

### ISSUE-002: AiDbEngine::open 固定 Options::for_testing()

- **状态**: closed
- **发现于**: PROGRESS 步 6 / 章节 `docs/modules/storage.md`
- **相关 src**: `src/storage/aidb.rs`, `src/storage/aidb_options.rs`, `src/main.rs`
- **旧文档**: WiQunTools `06-persistence.md` — 「生产 Options 待收敛」
- **oldmain 代码**: `aikv-oldmain/src/storage/aidb_adapter.rs` — `Options::default()`, `sync_wal(false)`
- **现象**: 曾固定 `Options::for_testing()` + `sync_wal: true`; CLI 与 lifecycle 不一致.
- **修复**: `open()` / CLI 走 `server_db_options()` (`Options::default` preset); 单测 `open_for_testing()`; 可选 `--sync-wal`; `init_cluster` lifecycle 共用同一 builder.
- **下一步**: —

### ISSUE-001: MemoryEngine mget 对非 String key 静默返回 None

- **状态**: closed
- **发现于**: PROGRESS 步 6 / 章节 `docs/modules/storage.md`
- **相关 src**: `src/storage/memory.rs` (`mget`), `src/storage/adapter.rs` (`mget`)
- **oldmain 代码**: `memory_adapter.rs` `get_from_db` → WRONGTYPE; `aidb_adapter.rs` `get_from_db` → 非 String 返回 `None`
- **现象**: 曾 `MemoryEngine::mget` 遇 Hash/List 返回 `None`; `KvStorageAdapter::mget` 调 `get` → 整命令 WRONGTYPE; 双引擎不一致.
- **修复**: 对齐 Redis 7 — MGET 对 non-String / missing key 均 per-key 返回 `nil`, 命令不失败; `KvStorageAdapter::mget` 改 `load_typed` + String 提取 (与 memory 一致).
- **测试**: `compat::test_compat_mget_wrongtype`, `memory`/`aidb`/`command/string` 回归测.
- **下一步**: —

### ISSUE-012: EVAL 声明 key 的 KeyLock 无超时 (oldmain 30s)

- **状态**: closed
- **发现于**: PROGRESS 步 8 / 章节 `docs/modules/commands-extended.md` (步 3)
- **相关 src**: `src/command/script.rs` (`lock_keys_sorted_with_timeout`); `src/command/router.rs` (`KeyLock`)
- **oldmain 代码**: `aikv-oldmain/src/command/script.rs` — `KeyLockManager` 默认 30s, 超时 `script_lock_timeouts` + ERR
- **现象**: 曾 `KeyLock::lock_keys_sorted` 无 script 专用超时; 同 key 并发 EVAL 可能永久等待
- **修复**: EVAL/EVALSHA 改 `lock_keys_sorted_with_timeout` (默认 30s); 超时返回 `ERR Lock acquisition timeout after 30s`; JSON.MSET 等仍用无超时 `lock_keys_sorted`
- **测试**: `script.rs` — 同 key 并发 EVAL、锁超时 ERR、多 key 部分持锁回滚
- **下一步**: —

### ISSUE-011: SHUTDOWN NOW/FORCE/ABORT 未实现

- **状态**: closed (doc-only)
- **发现于**: PROGRESS 步 8 / 章节 `docs/modules/commands-extended.md` (步 2)
- **相关 src**: `src/command/persistence.rs` (`parse_shutdown_mode` — 仅 Default/SAVE/NOSAVE)
- **旧文档**: `aikv-oldmain/docs/development/api/01-commands.md` §SHUTDOWN
- **现象**: oldmain API 文档列 NOW/FORCE/ABORT; 现码仅 SAVE/NOSAVE; 未知 mode → ERR
- **修复**: doc-only — 仅 Default/SAVE/NOSAVE; 未知 mode → ERR, 见 [commands-extended.md](docs/modules/commands-extended.md)
- **影响**: Redis 全兼容客户端可能发未支持 flag

### ISSUE-010: MIGRATE 无 AUTH2 (username+password)

- **状态**: closed
- **发现于**: PROGRESS 步 8 / 章节 `docs/modules/commands-extended.md` (步 2)
- **相关 src**: `src/command/key.rs` (`migrate`), `src/command/migrate.rs` (`send_restore`, `RestoreAuth`)
- **oldmain 代码**: `aikv-oldmain/src/command/key.rs` — AUTH2 user/pass 分支
- **现象**: 目标需 ACL username+password 时现码无法 MIGRATE
- **修复**: 解析 `AUTH2 username password`; TCP 发 `AUTH user pass` (对齐 Redis 7 / oldmain); `KEYS` 列表遇 `AUTH2` 停止; `AUTH2` 优先于 `AUTH`
- **测试**: `tests/modules/command/key.rs` — `test_migrate_auth2`, `test_migrate_keys_stops_at_auth2`, `test_migrate_auth2_precedence_over_auth`, `test_migrate_auth2_syntax_error`

### ISSUE-009: Lua redis.call JSON.MGET 未实现

- **状态**: closed
- **发现于**: PROGRESS 步 8 / 章节 `docs/modules/commands-extended.md` (步 1–2)
- **相关 src**: `src/command/json.rs`, `src/command/script/json_exec.rs`, `src/command/script/execute.rs`, `registry.rs`, `router.rs`
- **现象**: `command_key_indices` 含 `JSON.MGET` 但 dispatch 无; 顶层亦无 `JSON.MGET`
- **修复**: 实现 `JSON.MGET key [key ...] path` — 顶层 + Lua `redis.call`; 修正 Lua KEYS 校验 (全部 key 除末尾 path); missing key/path → null; wrong-type → 整命令 WRONGTYPE (与 `JSON.GET` 一致)
- **测试**: `tests/modules/command/json.rs`, `script.rs` — `json_mget` / `script_json_mget` 回归测

### ISSUE-008: SAVE 日志 target 为 bgsave.complete

- **状态**: closed (doc-only)
- **发现于**: PROGRESS 步 8 / 章节 `docs/modules/commands-extended.md` (步 1)
- **相关 src**: `src/command/persistence.rs` (`save` — `tracing::info!(target: "persist", …, "bgsave.complete")`)
- **现象**: 同步 SAVE 成功日志复用 BGSAVE 事件名
- **修复**: doc-only — 同步 SAVE 成功日志事件名 `bgsave.complete` (target `persist`); 行为无影响
- **影响**: 日志检索/告警可能混淆

### ISSUE-007: SCRIPT KILL 恒 NOTBUSY

- **状态**: closed (doc-only)
- **发现于**: PROGRESS 步 8 / 章节 `docs/modules/commands-extended.md` (步 2)
- **相关 src**: `src/command/script.rs` (`script_kill`)
- **旧文档**: `aikv-oldmain/docs/development/architecture/05-lua-scripting.md` Limitations — 已承认不可用
- **现象**: 无运行中脚本跟踪; 始终 `NOTBUSY No scripts in execution right now.`
- **修复**: doc-only — stub, 恒 NOTBUSY; 见 [commands-extended.md](docs/modules/commands-extended.md) Lua 节
- **影响**: `backup/aikv/README.md` 标 SCRIPT KILL ✅ 与事实不符 (不在本仓维护范围)

### ISSUE-006: MIGRATE KEYS 忽略 COPY, 批量路径始终 delete

- **状态**: closed
- **发现于**: PROGRESS 步 8 / 章节 `docs/modules/commands-extended.md` (步 1–2)
- **相关 src**: `src/command/key.rs` (`migrate` KEYS 分支)
- **oldmain 代码**: `aikv-oldmain/src/command/key.rs` L990 `if !copy { delete }`
- **现象**: 单 key 路径尊重 COPY; `MIGRATE … KEYS k1 k2` 批量路径曾无条件 delete; KEYS 列表曾吞掉 trailing COPY
- **修复**: KEYS 批量路径 `if !copy { delete }`; KEYS 列表解析遇 COPY/REPLACE/AUTH 停止并继续解析选项; 回归测 `test_migrate_keys_copy`
- **下一步**: —

### ISSUE-005: BlockingRegistry evict_expired 无后台调用

- **状态**: closed
- **发现于**: PROGRESS 步 8 / 章节 `docs/modules/commands-extended.md` (步 1)
- **相关 src**: `src/command/blocking.rs` (`evict_expired`, `start_background_eviction`); `src/main.rs`
- **oldmain 代码**: (无 BlockingRegistry — 重构新增)
- **现象**: 过期 waiter 仅靠 handler poll 超时; `waiters` DashMap 滞留 dead entry 与 notify 后空槽
- **修复**: `main.rs` 启动 1s tick 后台 task 调 `BlockingRegistry::global().evict_expired()`; 不依赖 `monitoring` feature
- **测试**: `blocking.rs` unit tests — 过期清理、保留活跃 waiter、notify 后空槽移除

### ISSUE-004: cluster_route 预留 MSETNX 但命令未注册/未实现

- **状态**: closed (doc-only)
- **发现于**: PROGRESS 步 7 / 章节 `docs/modules/commands-core.md`
- **相关 src**: `src/command/router.rs` (`cluster_route` 曾含不可达 `msetnx` 分支); `src/command/registry.rs` 无 `MSETNX`
- **oldmain 代码**: 无 MSETNX (全仓无实现)
- **现象**: `cluster_route` 注释引用未注册命令; 客户端发 MSETNX → `ERR unknown command`
- **定案**: 刻意不实现 — oldmain 亦无; 单 key 用 `SETNX`; 多 key 原子 NX 按需再开 issue
- **收尾**: 移除 `router.rs` dead `msetnx` 引用; module 已知限制注明

### ISSUE-003: GETRANGE/SETRANGE oldmain 有、现码未实现

- **状态**: closed
- **发现于**: PROGRESS 步 7 / 章节 `docs/modules/commands-core.md`
- **相关 src**: `src/command/string.rs`, `registry.rs`, `router.rs`
- **oldmain 代码**: `aikv-oldmain/src/command/string.rs`, `mod.rs` match
- **现象**: 重构线移除; 客户端发 GETRANGE/SETRANGE → `ERR unknown command`
- **定案**: 恢复实现 — 对齐 Redis 7 / oldmain (负索引、越界、空 key、SETRANGE `\0` 填充)
- **收尾**: `string.rs` handler + registry/router + `tests/modules/command/string.rs`
