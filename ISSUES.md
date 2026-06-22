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

- **状态**: doc-only
- **发现于**: PROGRESS 步 12 / 章节 `docs/modules/observability.md` (步 3)
- **相关 src**: `src/server/slowlog.rs` (`DEFAULT_SLOWLOG_THRESHOLD_US = 100_000`)
- **oldmain 代码**: `aikv-oldmain/src/observability/logging.rs` — 默认 10_000 µs (10ms, 近 Redis)
- **现象**: 现默认 100ms; Redis `slowlog-log-slower-than` 默认 10000 µs; CONFIG GET 返回 100000
- **影响**: 新部署记录的慢查询更少; module 写实际默认
- **下一步**: doc-only; 是否改默认另开开发任务

### ISSUE-022: metrics refresh 周期 15s vs 设计 spec 1s

- **状态**: doc-only
- **发现于**: PROGRESS 步 12 / 章节 `docs/modules/observability.md` (步 2)
- **相关 src**: `src/main.rs` (15s interval), `src/server/config.rs` (`refresh_runtime_metrics`)
- **旧文档**: `backup/aikv/docs/superpowers/specs/2026-06-10-redis-observability-alignment-design.md` §R4 写 1s
- **现象**: 设计 spec 要求 refresh 与 ops/sec 窗口 1s; 现 `tokio::time::interval(Duration::from_secs(15))`
- **影响**: `instantaneous_ops_per_sec` 采样粒度粗于 spec; module 以代码 15s 为准
- **下一步**: doc-only

### ISSUE-021: `refresh_runtime_metrics` 仅 monitoring 后台 tick

- **状态**: doc-only
- **发现于**: PROGRESS 步 12 / 章节 `docs/modules/observability.md` (步 2)
- **相关 src**: `src/main.rs` (`#[cfg(feature = "monitoring")]` 15s loop), `src/server/config.rs` (`refresh_runtime_metrics`)
- **现象**: 无 `monitoring` feature 时不启后台 refresh; `StorageObservation` drain、ops/sec sample、Prometheus gauge 同步不自动运行
- **影响**: 无 monitoring 构建下 INFO `stats.expired_keys` / `instantaneous_ops_per_sec` 可能滞后; memory/keyspace 段仍可直接查 `KvStorage`
- **下一步**: doc-only — module 写 refresh 条件

### ISSUE-020: `blocked_clients` 无写入点

- **状态**: open
- **发现于**: PROGRESS 步 12 / 章节 `docs/modules/observability.md` (步 2)
- **相关 src**: `src/server/metrics.rs` (`blocked_clients`), `src/command/blocking.rs` (BlockingRegistry)
- **旧文档**: `WiQunTools/docs/wiqun-kv-inventory/08-observability.md` — 「无 BLPOP 时为 0」
- **现象**: `AtomicUsize blocked_clients` 仅 default 0; BlockingRegistry 等待/唤醒未同步计数
- **影响**: INFO clients / `aikv_blocked_clients` 恒 0; 实现阻塞命令后仍可能不准
- **下一步**: doc-only 先描述现状; 接 blocking 时修代码

### ISSUE-019: SET-CONFIG-EPOCH / COUNT-FAILURE-REPORTS 为 stub

- **状态**: doc-only
- **发现于**: PROGRESS 步 11 / 章节 `docs/modules/cluster.md` (步 2–3)
- **相关 src**: `src/cluster/commands.rs` (`dispatch_cluster`)
- **oldmain 代码**: `aikv-oldmain/src/cluster/commands.rs` — `SET-CONFIG-EPOCH` 有实现; `COUNT-FAILURE-REPORTS` 部分逻辑
- **现象**: 现码 `SET-CONFIG-EPOCH` 恒 OK; `COUNT-FAILURE-REPORTS` 恒 0
- **影响**: module「已知限制」; redis-cli 部分检查路径可能跳过
- **下一步**: doc-only — 文档注明 stub; 按需补实现

### ISSUE-018: CLUSTER FAILOVER 仅 FORCE/TAKEOVER 手动升主

- **状态**: doc-only
- **发现于**: PROGRESS 步 11 / 章节 `docs/modules/cluster.md` (步 2–3)
- **相关 src**: `src/cluster/commands.rs` (`cluster_failover`)
- **旧文档**: `aikv-oldmain/docs/development/api/02-cluster-api.md` — 称 openraft 自动故障切换
- **oldmain 代码**: `FailoverMode::Default|Force|Takeover`; 部分路径经 MetaRaft leader 转发
- **现象**: 现码仅 `FORCE|TAKEOVER`; replica 上 `change_group_membership` 升主; 无 Redis 标准选举等待
- **影响**: module 说明手动 failover 模型; 与 Redis 全兼容 FAILOVER 语义不同
- **下一步**: doc-only

### ISSUE-017: CLUSTER REPLICATE 仅本地 ReplicationRole 元数据

- **状态**: doc-only
- **发现于**: PROGRESS 步 11 / 章节 `docs/modules/cluster.md` (步 2–3)
- **相关 src**: `src/cluster/commands.rs` (`cluster_replicate`, `cluster_failover`)
- **oldmain 代码**: oldmain `cluster_replicate` 调 membership/MultiRaft 路径更完整
- **现象**: 现码只写 `ReplicationRole::Replica { primary_id }`; 注释声明 replicas 不服务数据读取
- **影响**: FAILOVER 前需 REPLICATE; replica 读依赖 `READONLY` + 本地 group; module 已知限制
- **下一步**: doc-only — 与 storage/cluster_adapter 读路径一并描述

### ISSUE-016: CLUSTER RESET 未实现

- **状态**: closed (doc-only)
- **发现于**: PROGRESS 步 11 / 章节 `docs/modules/cluster.md` (步 2)
- **相关 src**: `src/cluster/commands.rs` (`dispatch_cluster` RESET arm)
- **旧文档**: `aikv-oldmain/docs/development/architecture/03-cluster.md` §排障 — `CLUSTER RESET SOFT`
- **现象**: MetaRaft 架构下集群元数据为共识状态, 单节点 RESET 无法等价 Redis 语义; oldmain `cluster_reset()` 亦未接入 dispatch (SOFT 空操作, HARD 仅部分清 slot)
- **修复**: doc-only — `CLUSTER RESET` 返回明确 ERR + 替代运维步骤见 [cluster.md](docs/modules/cluster.md#重置集群无-cluster-reset)
- **影响**: redis-cli / 旧脚本需改用停服 + 清 `data_dir` 重搭 (e2e 同模式)

### ISSUE-015: CLUSTER METARAFT * 子命令已移除

- **状态**: doc-only
- **发现于**: PROGRESS 步 11 / 章节 `docs/modules/cluster.md` (步 2–3)
- **相关 src**: (无 aikv RESP 层)
- **旧文档**: `aikv-oldmain/docs/development/api/02-cluster-api.md`, `03-cluster.md` §METARAFT
- **oldmain 代码**: `aikv-oldmain/src/server/connection.rs` — ADDLEARNER/PROMOTE/MEMBERS/STATUS/SETSTATUS
- **现象**: 重构后 MetaRaft 运维不在 aikv CLUSTER 子命令暴露; 由 aidb gRPC/内部 API 承担
- **影响**: module 一行指向 [aidb cluster.md](../aidb/docs/modules/cluster.md); 勿写 aikv 支持 METARAFT
- **下一步**: doc-only

### ISSUE-014: GossipState 后台刷新但未接入 CLUSTER NODES

- **状态**: open
- **发现于**: PROGRESS 步 11 / 章节 `docs/modules/cluster.md` (步 2–3)
- **相关 src**: `src/cluster/gossip.rs`, `src/main.rs`, `src/cluster/commands.rs` (`cluster_nodes`)
- **旧文档**: WiQunTools `07-cluster-protocol.md` — Gossip 供 NODES timestamps; `AGENTS.md` 称刷新 NODES 展示
- **现象**: `start_background_refresh` 更新 `GossipState` + metrics; `cluster_nodes()` 只读 MetaRaft, 不读 gossip; 无 PING/PONG 网络
- **影响**: 文档勿称「Gossip 驱动 NODES」; 可能是 dead code 或未完成 wiring
- **下一步**: 待核实 — 接入 NODES ping 字段或删 unused GossipState 读路径/简化模块

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

- **状态**: open
- **发现于**: PROGRESS 步 6 / 章节 `docs/modules/storage.md`
- **相关 src**: `src/storage/memory.rs` (`mget`), `src/storage/adapter.rs` (`mget` → `get`)
- **oldmain 代码**: `memory_adapter.rs` `get_from_db` → WRONGTYPE; `aidb_adapter.rs` `get_from_db` → 非 String 返回 `None`
- **现象**: `MemoryEngine::mget` 遇 Hash/List 等返回 `None` (等同 key 不存在); `KvStorageAdapter::mget` 调 `get` → 整命令 **WRONGTYPE**. oldmain memory MGET 失败 WRONGTYPE, aidb MGET 静默 nil — **重构后与 oldmain 两侧均不完全一致**.
- **影响**: `--engine memory` vs `aidb` 下 MGET wrong-type 语义不同; `compat.rs` 未覆盖.
- **下一步**: 对照 Redis 7 语义; 统一 `mget` 实现; 补 compat 测试.

### ISSUE-012: EVAL 声明 key 的 KeyLock 无超时 (oldmain 30s)

- **状态**: open
- **发现于**: PROGRESS 步 8 / 章节 `docs/modules/commands-extended.md` (步 3)
- **相关 src**: `src/command/script.rs` (`lock_keys_sorted`); `src/command/router.rs` (`KeyLock`)
- **oldmain 代码**: `aikv-oldmain/src/command/script.rs` — `KeyLockManager` 默认 30s, 超时 `script_lock_timeouts` + ERR
- **现象**: 现码 `KeyLock` 无 script 专用超时; 同 key 长时间脚本或死锁可能永久占桶
- **影响**: 与 oldmain 行为不同; 高并发同 key EVAL 风险
- **下一步**: 待核实 — 恢复 script 锁超时或文档声明 intentional

### ISSUE-011: SHUTDOWN NOW/FORCE/ABORT 未实现

- **状态**: doc-only
- **发现于**: PROGRESS 步 8 / 章节 `docs/modules/commands-extended.md` (步 2)
- **相关 src**: `src/command/persistence.rs` (`parse_shutdown_mode` — 仅 Default/SAVE/NOSAVE)
- **旧文档**: `aikv-oldmain/docs/development/api/01-commands.md` §SHUTDOWN
- **现象**: oldmain API 文档列 NOW/FORCE/ABORT; 现码仅 SAVE/NOSAVE; 未知 mode → ERR
- **影响**: module「已知限制」; Redis 全兼容客户端可能发未支持 flag
- **下一步**: doc-only — 文档注明; 或按需补实现

### ISSUE-010: MIGRATE 无 AUTH2 (username+password)

- **状态**: open
- **发现于**: PROGRESS 步 8 / 章节 `docs/modules/commands-extended.md` (步 2)
- **相关 src**: `src/command/key.rs` (`migrate`), `src/command/migrate.rs` (`send_restore` — 仅 AUTH password)
- **oldmain 代码**: `aikv-oldmain/src/command/key.rs` — AUTH2 user/pass 分支
- **现象**: 目标需 ACL username+password 时现码无法 MIGRATE
- **影响**: 集群/多租户迁移场景; module 一行引用
- **下一步**: 待核实 — 补 AUTH2 或文档声明不支持

### ISSUE-009: Lua redis.call JSON.MGET 未实现

- **状态**: open
- **发现于**: PROGRESS 步 8 / 章节 `docs/modules/commands-extended.md` (步 1–2)
- **相关 src**: `src/command/script/execute.rs` (`command_key_indices` 含 `JSON.MGET`; dispatch match 无)
- **现象**: 脚本内 `redis.call('JSON.MGET', …)` → unknown command; 顶层也无 `JSON.MGET` Redis 命令
- **影响**: Lua JSON 子集不完整; key_indices 表误导
- **下一步**: 待核实 — 实现 exec_json_mget 或从 key_indices 移除

### ISSUE-008: SAVE 日志 target 为 bgsave.complete

- **状态**: doc-only
- **发现于**: PROGRESS 步 8 / 章节 `docs/modules/commands-extended.md` (步 1)
- **相关 src**: `src/command/persistence.rs` (`save` — `tracing::info!(target: "persist", …, "bgsave.complete")`)
- **现象**: 同步 SAVE 成功日志复用 BGSAVE 事件名
- **影响**: 日志检索/告警可能混淆; 行为无影响
- **下一步**: doc-only 或改 log message

### ISSUE-007: SCRIPT KILL 恒 NOTBUSY

- **状态**: doc-only
- **发现于**: PROGRESS 步 8 / 章节 `docs/modules/commands-extended.md` (步 2)
- **相关 src**: `src/command/script.rs` (`script_kill`)
- **旧文档**: `aikv-oldmain/docs/development/architecture/05-lua-scripting.md` Limitations — 已承认不可用
- **现象**: 无运行中脚本跟踪; 始终 `NOTBUSY No scripts in execution right now.`
- **影响**: module「已知限制」; `backup/aikv/README.md` 标 SCRIPT KILL ✅ 与事实不符
- **下一步**: doc-only

### ISSUE-006: MIGRATE KEYS 忽略 COPY, 批量路径始终 delete

- **状态**: closed
- **发现于**: PROGRESS 步 8 / 章节 `docs/modules/commands-extended.md` (步 1–2)
- **相关 src**: `src/command/key.rs` (`migrate` KEYS 分支)
- **oldmain 代码**: `aikv-oldmain/src/command/key.rs` L990 `if !copy { delete }`
- **现象**: 单 key 路径尊重 COPY; `MIGRATE … KEYS k1 k2` 批量路径曾无条件 delete; KEYS 列表曾吞掉 trailing COPY
- **修复**: KEYS 批量路径 `if !copy { delete }`; KEYS 列表解析遇 COPY/REPLACE/AUTH 停止并继续解析选项; 回归测 `test_migrate_keys_copy`
- **下一步**: —

### ISSUE-005: BlockingRegistry evict_expired 无后台调用

- **状态**: open
- **发现于**: PROGRESS 步 8 / 章节 `docs/modules/commands-extended.md` (步 1)
- **相关 src**: `src/command/blocking.rs` (`evict_expired`); 全仓库无 caller
- **oldmain 代码**: (无 BlockingRegistry — 重构新增)
- **现象**: 过期 waiter 仅靠 handler poll 超时; `waiters` DashMap 可能滞留至 notify 或进程结束
- **影响**: 长时间阻塞 + 大量 key 时内存; `deadline` 字段未主动清理
- **下一步**: 待核实 — 接 server 定时 task 或删除 dead API

### ISSUE-004: cluster_route 预留 MSETNX 但命令未注册/未实现

- **状态**: open
- **发现于**: PROGRESS 步 7 / 章节 `docs/modules/commands-core.md`
- **相关 src**: `src/command/router.rs` (`cluster_route` `is_mset` 含 `msetnx`); `src/command/registry.rs` 无 `MSETNX`
- **现象**: CROSSSLOT 分支引用不存在的命令; 客户端发 MSETNX → `ERR unknown command`
- **影响**: cluster 多 key 语义文档需注明 dead branch; 或补实现/删注释
- **下一步**: 待核实 — 实现 MSETNX 或从 cluster 注释移除 `msetnx`

### ISSUE-003: GETRANGE/SETRANGE oldmain 有、现码未实现

- **状态**: open
- **发现于**: PROGRESS 步 7 / 章节 `docs/modules/commands-core.md`
- **相关 src**: (无) — `registry`/`router`/`string.rs` 均无
- **旧文档**: `aikv-oldmain/docs/development/api/01-commands.md` §GETRANGE
- **oldmain 代码**: `aikv-oldmain/src/command/string.rs`, `mod.rs` match
- **现象**: 重构线 (aikv/wiqun-kv) 移除; oldmain 测试 `data_types_test.rs` 仍覆盖
- **影响**: module「已知限制」; Redis 客户端 substring 命令不可用
- **下一步**: 待核实是否刻意裁剪或遗漏
