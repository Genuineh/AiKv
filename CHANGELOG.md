# Changelog

本项目的所有重要变更都会记录在此文件中.

格式基于 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/),
版本号遵循 [语义化版本](https://semver.org/lang/zh-CN/).

## [Unreleased]

### Changed

- **ISSUE-004**: doc-only 关闭 — MSETNX 刻意不实现 (oldmain 亦无); 移除 `cluster_route` 不可达 `msetnx` 引用; module 已知限制注明.

### Fixed

- **ISSUE-014**: 移除 unused `GossipState` 缓存; 拓扑 tick 保留 leader 路由刷新 + gossip metrics; `CLUSTER NODES` 继续读 MetaRaft, link-state 恢复 `NodeStatus` → connected/disconnected.
- **ISSUE-001**: `MGET` 对齐 Redis 7 — non-String / missing key per-key 返回 `nil`, memory 与 aidb 双引擎一致; `KvStorageAdapter::mget` 不再经 `get()` 抛 WRONGTYPE.
- **ISSUE-012**: EVAL/EVALSHA 声明 key 的 `KeyLock` 恢复 30s 锁等待超时 (`lock_keys_sorted_with_timeout`); 超时 `ERR Lock acquisition timeout after 30s`.
- **ISSUE-016**: `CLUSTER RESET` 明确返回不支持 ERR (非 unknown subcommand); doc-only 关闭 — 替代步骤见 `docs/modules/cluster.md`.
- **ISSUE-006**: `MIGRATE … KEYS` 批量路径尊重 `COPY` (源 key 保留); KEYS 列表解析不再把 trailing `COPY`/`REPLACE`/`AUTH` 当作 key 名.
- **ISSUE-013**: `CLUSTER INFO` 恢复动态 `cluster_state:ok`/`fail` (16384 slot 全覆盖 + group leader + 映射一致).
- **ISSUE-002**: CLI `--engine aidb` 与 cluster lifecycle 使用生产 `Options::default()` preset (`server_db_options`); 单测改 `open_for_testing()`; 新增 `--sync-wal`.

### Added

- **B1.3**: `tests/modules/storage/prod_options.rs` — 生产 Options 非 `for_testing()` 回归测.

## [0.10.5] - 2026-06-10

### Added

- **Redis INFO 对齐 (Phase 1–4)**: 新增 `InfoRenderer` (`src/server/info.rs`), INFO 与 `ServerMetrics` 单一数据源; default/all section 顺序对齐 Redis 7.
- **INFO section**: `commandstats`, `errorstats`, `modules`, `clients.maxclients`, `stats.total_error_replies`, persistence/replication/cpu 扩展字段, keyspace `avg_ttl`.
- **Prometheus**: `aikv_instantaneous_ops_per_sec`, `aikv_evicted_keys_total`, `aikv_blocked_clients` gauge; 周期 `sync_redis_aligned_gauges`.
- **集群可观测性**: gossip tick → `cluster_stats_messages_sent/received`; `CLUSTER INFO` 接 metrics; `cluster/replication.rs` 按 MetaRaft 计算 role/connected_slaves.
- **进程指标**: `process_metrics.rs` 独立模块; INFO cpu 与 monitoring 共用 `/proc`; 后台 task 调用 `refresh_process_metrics`.
- **测试**: `info_alignment`, `info_golden` (Redis 7 P0 字段清单), `info_metrics_consistency`, 集群 observability 单测.

### Changed

- **INFO 命令**: 委托 `InfoRenderer`; 未知 section 返回空 bulk; `redis_mode`/`cluster_enabled` 按集群初始化状态; memory 使用真实 `storage.memory_usage_bytes`.
- **命令计数**: router 层统一 `on_command`; connection 层记录 `on_command_duration` usec (供 commandstats).
- **`KeyspaceStats`**: 增加 `avg_ttl` (memory/adapter 引擎统计).

### Fixed

- **INFO memory 占位公式**: 移除 `1MB + 16KB×连接数` 假值.

## [0.10.4] - 2026-06-09

### Added

- **CLUSTER CREATEGROUP**: `CLUSTER CREATEGROUP <primary-id> [group-id]` 为已 MEET 节点创建空 data group (不分配槽位); 默认 `group_id = primary_id`. 用于扩容时先组主从、日后迁槽.

### Fixed

- **ADD_REPLICA 报错文案**: 主节点未入 data group 时返回 `ERR Primary is not a data group member` (原 `ERR Primary has no assigned slots` 易误解为槽位问题).

## [0.10.3] - 2026-06-08

### Fixed

- **SlotMigrationManager 未初始化**: `init_cluster` 现创建并注入 `SlotMigrationManager`, 修复 `CLUSTER SETSLOT MIGRATING` / `CLUSTER REBALANCE` 返回 `ERR SlotMigrationManager not initialized` 的问题.
- **槽迁移期间 MIGRATE 失败**: `MIGRATE` 不再经集群 MOVED/ASK 重定向 (源节点本地导出 key); `send_restore` 在集群模式下先发送 `ASKING`; 目标节点 `importing_slots` 本地追踪时允许 RESTORE 写入 IMPORTING 槽位.
- **IMPORTING 窗口 RESTORE 被 MOVED**: `ClusterRouter::decide()` 在 `Assigned` 缓存窗口期对 `importing_slots` 中的写命令短路为 `Execute`, 修复 `SETSLOT IMPORTING` 后 router 未刷新时 `RESTORE` 返回 `MOVED` 的问题.
- **IMPORTING 窗口 RESTORE 写入错误分片**: `ClusterDataAdapter` 写路径在 `importing_slots` 期间将数据提交到本节点目标 data group, 避免 RESTORE 仍按 router 源分片写入导致 NODE 后目标 GET 为 `nil`.

## [0.10.2] - 2026-06-08

### Added

- **CLUSTER DEL_REPLICA**: `CLUSTER DEL_REPLICA <primary_id> <replica_id>` 从 data group 移除副本; 语义与 `ADD_REPLICA` 对称, MultiRaft 不可用时回退 MetaRaft 元数据.
- **CLUSTER FORGET FORCE**: `CLUSTER FORGET <node-id> [FORCE]` 支持强制摘除仍关联 MetaRaft/data group 的节点.
- **集群 node id 解析**: `CLUSTER ADD_REPLICA` / `DEL_REPLICA` 等命令同时接受十进制 id (factory 引导) 与 40 位 hex id (redis-cli 风格).

### Fixed

- **集群模式 SAVE/BGSAVE**: `ClusterDataAdapter::create_checkpoint` 对本地 data group `ShardedStorage` 逐组 checkpoint (`dest/group_{id}/`), `flush` 同步刷盘; 修复集群 primary 上 SAVE/BGSAVE 无本地 group 或 checkpoint 失败的问题.

## [0.10.1] - 2026-06-08

### Fixed

- **集群数据面经 Raft 复制 (primary failover 后旧数据可读)**: 新增 `ClusterDataAdapter`, cluster 模式下用户数据读写改经数据面 Raft group —— 写经 `propose_group` 提交并复制到各副本状态机, 读直读本地 group 状态机 (`get_local`); 集群未就绪时回退本地引擎. 修复此前数据仅写本地 DB、从不经 Raft 复制, 导致 primary 故障转移后新 leader 读到 `nil` 的问题. 所有 string/hash/list 等命令逻辑无需改动 (注入在 `StorageAdapter` 层).

## [0.10.0] - 2026-06-05

### Added

- **SETBIT / GETBIT** (Phase 3 Task 3.2): Redis 位图命令 (MSB-first offset), 解锁 `TL.Redis` `创建bits`.
- **集群透明转发** (Phase 3 Task 3.1): 非本地 slot 命令经 TCP 转发至 leader, 单端点客户端无需 MOVED; `EVAL` 路由 key 修正.

### Fixed

- **ATOM.EXEC JSON batch 集群回滚**: snapshot/rollback 改经路由层 `DUMP`/`DEL`/`RESTORE`, 修复跨分片写入后回滚仅操作本节点导致 key 残留 (`TL.KvDoc` 事务回滚测试).
- **ATOM.EXEC JSON batch 并发回滚**: 仅回滚本 batch 已成功写入的 key; 修复失败事务用旧 snapshot `RESTORE` 覆盖其他并发事务提交 (`TL.KvDoc` `多个并发_只成功一个`).
- **ZRANGEBYSCORE / ZRANGEBYLEX LIMIT 负数 count**: `LIMIT offset -1` 视为无限制 (对齐 StackExchange.Redis), 修复 `SocketClosed` 断连.
- **JSON Path filter 裸 `@`**: `[?(@ > '1')]`、`[?(@ =~ /.../i)]`、嵌套 `Any()`/真值判断 — `filter_subject_field()` 将裸 `@` 解析为元素本身而非字段名 `@`.
- **JSON Path 字符串数字比较**: `json_compare` 支持 `"10" > "2"` 等字典序比较.

## [0.9.9] - 2026-06-05

### Added

- **集群客户端地址通告 (`AnnounceResolver`)**: 新增 `AIKV_CLUSTER_ANNOUNCE_MODE` 环境变量 (`unknown` | `fixed`, 默认 `unknown`). `unknown` 模式下 `CLUSTER SLOTS` / `CLUSTER SHARDS` / MOVED/ASK 输出空 host (`:port`), 客户端沿用种子连接地址 (对标 Redis 7 `cluster-preferred-endpoint-type`). `CLUSTER NODES` 仍显示完整 `client_addr` 供运维查看.
- **bootstrap `client_addr` 同步**: 所有节点 (含 bootstrap) 启动后自动将 `AIKV_CLIENT_ADDR` 同步至 MetaRaft.
- **E2E**: `e2e/test_cluster_announce.sh` 验证 unknown 模式 SLOTS 与跨 shard 路由.

### Changed

- **默认集群通告行为**: 新部署默认 `unknown` 模式; 依赖 LAN IP 做服务发现的环境请设置 `AIKV_CLUSTER_ANNOUNCE_MODE=fixed`.
- **WSL2 默认通告**: WSL2 环境下默认 `AIKV_EXTERNAL_HOST=127.0.0.1`.

## [0.9.8] - 2026-06-04

### Fixed

- **Data Group failover 失败**: 修复 ADD_REPLICA 时成员变更操作在 replica 节点 group 创建前执行的问题. MetaRaft 元数据更新先于 `add_learner_to_group`, 配合 500ms 重试 (最多 10s), 确保 LifecycleManager 有时间发现并创建 group.
- **复制屏障下沉**: `OpenRaftNode::change_membership` 内置 `wait_members_catch_up` + `confirm_replication` 屏障, MetaRaft 和 Data Group 的成员变更统一受益.

### E2E

- **`test_cluster_failover.sh`**: 补充 `CLUSTER ADD_REPLICA` 步骤, 增强 failover 验证 (检查 `myself,master` 切换).

## [0.9.7] - 2026-06-03

### Fixed

- **CLUSTER NODES 角色显示错误**: `cluster_nodes()` 现在从 Raft 元数据 (group membership) 推导本节点角色 (`myself,master` / `myself,slave`) 和 `primary_id`, 而非依赖始终为 `Primary` 的本地缓存. 同时为所有节点 (含远端) 正确计算 `primary_id`.
- **CLUSTER ADD_REPLICA 跨 group 失败**: `change_group_membership()` 对非本地 group 跳过 MultiRaft 操作, 仅更新 MetaRaft 元数据. `cluster_add_replica()` 添加元数据回退路径, 在 MultiRaft group 未初始化或不在本地时仍能完成元数据更新.
- **集群地址通告 (announce address)**: `AIKV_CLIENT_ADDR` 现在对所有节点 (含 `--cluster-peers` 加入节点) 生效. 加入节点启动后通过后台 task 将外部可达地址写入 MetaRaft 元数据, 确保 MOVED 重定向返回客户端可解析的地址而非容器内部 hostname.

### Changed

- **集群部署**: 添加 CLUSTER MEET 步骤 (Docker 网络使用容器 hostname 进行 RPC 通信, 使用 `EXTERNAL_HOST` 作为客户端通告地址); group 稳定等待时间延长至 10s; ADD_REPLICA 支持重试 (最多 15 次) 和错误报告.

## [0.9.6] - 2026-06-03

### Changed

- **数据面端口偏移可配置化**: 新增 `--cluster-data-port-offset` CLI 参数, 替代硬编码 `+10000`. 默认值 `10000` 完全向后兼容. 端口溢出校验改为动态 `rpc_port ≤ 65535 - offset`.

## [0.9.5] - 2026-06-02

### Changed

- 依赖 AiDb `0.14.6`, 包含集群副本自动对账修复.

## [0.9.4] - 2026-06-01

### Fixed

- **MetaRaft `client_write` 挂起**: DB::write 中 `memtable` 读锁死锁 (AiDb `0.14.5`), 集群命令 `CLUSTER ADDSLOTS` / `CLUSTER BUMPEPOCH` 不再永久挂起.
- **LeaderChangeWatcher / ConfigAutoSave 启动即退出**: `watch::Sender` 在 `init_cluster()` 返回时被 drop 导致 receiver 收到 `Closed`, 现已持久存储在 `ClusterStateManager` 中.
- **add_learner 阻塞 CLUSTER MEET**: 切换到 `add_learner_nonblocking`, 新增节点不可达时不再永久挂起.

### Changed

- **CLAUDE.md**: 移除过时的 BUMPEPOCH `fetch_add` known_limitation (已共识化).

## [0.9.3] - 2026-06-01

### Added (Iteration 1 — Auto Failover + Slot Migration CLI)

- **可配置 Raft election timeout**: CLI `--raft-election-timeout-min <ms>` / `--raft-election-timeout-max <ms>`, 默认 500/1000ms; `heartbeat_interval` 自动推导为 `min/5`.
- **LeaderChangeWatcher 集成**: `init_cluster()` 启动 AiDb LeaderChangeWatcher 后台任务, tick 间隔为 `election_timeout_min/2`.
- **槽迁移 CLI 可见性**:
  - `CLUSTER SLOTS`: 迁移中的 slot 增加目标节点信息 (第四个元素, `redis-cli --cluster check` 兼容).
  - `CLUSTER INFO`: 新增 `cluster_slots_migrating` 字段, 统计 `SlotStatus::Migrating` 槽数.
  - `CLUSTER NODES`: 迁移参与节点追加 `,migrating` / `,importing` flags.

### Added (Iteration 2 — Rebalance + BUMPEPOCH + Auto SAVECONFIG)

- **CLUSTER REBALANCE**: 新增子命令, 比较当前槽分布与理想分布, 依次执行 `start_migration → run_pending → commit_migration` 完成再均衡.
- **BUMPEPOCH 共识化**: 从本地 `AtomicU64::fetch_add` 改为通过 `MetaRaft::propose(MetaRequest::BumpEpoch)` 共识, 返回共识后的 epoch 值.
- **SAVECONFIG 自动持久化**: `config_auto_save.rs` 后台任务 (2s 轮询 `ClusterMeta.version`), 变更时自动写入 `nodes.conf` (原子 write+fsync+rename); 手动 `CLUSTER SAVECONFIG` 复用 `save_nodes_conf()` 公共函数.

### Added

- **阻塞命令**: BLPOP/BRPOP/BLMOVE/BZPOPMIN/BZPOPMAX — DashMap + oneshot 通知机制, 超时控制, 写命令自动 notify 阻塞等待者.
- **MIGRATE 跨节点**: 移除 localhost 限制; 新增 AUTH password 参数; 新增 KEYS 子命令批量迁移.
- **文档同步**: 清理 CLAUDE.md Known Limitations 中已修复项, 更新版本号.

## [0.9.2] - 2026-05-30

### Fixed

- **CLUSTER MEET 后 CLUSTER NODES 地址错误**: `MetaRaftNode::initialize` 未在 MetaStateMachine 中注册引导节点自身, 导致第一个 `CLUSTER MEET` 用错误的 node_id 覆盖引导节点地址. 上游 aidb 0.14.3 修复.
- **CLUSTER MEET 后新节点不可达**: `MembershipCoordinator::add_node` 未将新节点地址注册到 gRPC network factory, 导致 Raft 心跳无法送达; 同时未调用 `add_learner`, 新节点无法加入 Raft 集群.
- **MOVED 重定向返回 RPC 端口而非客户端端口**: `client_addr` 未存储, MOVED 指向 gRPC 端口导致 `redis-cli -c` 收到二进制数据. 现通过 LifecycleManager 回退逻辑优先使用 `client_addr`.
- **`is_local_group_leader` 缓存永不过期**: `ClusterStateManager::refresh()` 现集成到 GossipState 后台循环 (每 1s), 确保路由决策使用最新 leader 信息.

### Added

- **CLUSTER ADDSLOTS NODE \<id\>**: 支持从 leader 节点为任意节点分配 slot, 绕过非 leader 节点 Raft 转发未实现的限制.
- **集群 E2E 测试套件 (6 个脚本)**: `test_cluster_formation.sh` (组建), `test_cluster_slots.sh` (槽分配), `test_cluster_routing.sh` (路由 + 全命令族), `test_cluster_data_consistency.sh` (200 keys + DBSIZE), `test_cluster_failover.sh` (故障恢复), `test_cluster_forget.sh` (节点摘除). 全部通过.
- **`utils.sh` 集群扩展**: `build_release_cluster`, `start_cluster_node`, `wait_node`, `rc_node`, `stop_cluster_node`, `register_cluster_cleanup` — 可复用的集群测试基础设施.

### Known Limitations

- **非 leader 节点 ADDSLOTS 不可用**: Multi-Raft `client_write` 转发未实现, 变通方案 `ADDSLOTS NODE <id>`.
- **数据面端口溢出**: `rpc_port + 10000 > 65535` 时端口溢出, 部署时确保 RPC 端口 ≤ 55535.

## [0.9.1] - 2026-05-29

### Fixed

- **CLUSTER ADDSLOTS `ERR Node not in any group`**: `cluster_add_slots` 在节点未属于任何 Raft group 时自动创建 (通过 `MetaRequest::CreateGroup`, node_id 作为 group_id, 本节点作为 leader). 此前 `CLUSTER MEET` 只注册节点元数据不创建 group, 导致首次 ADDSLOTS 必然失败.
- **数据面 gRPC 服务器未启动**: `MultiRaftNode::start()` 现已在 `init_cluster` 中启动 (tokio::spawn 后台任务, `rpc_port + 10000` 端口). 上游 aidb 0.14.2 将 `start()`/`shutdown()` 等 5 个方法从 `&mut self` 改为 `&self`, 消除了与 `Arc` 的借用冲突.

### Changed

- `init_cluster` 初始化顺序调整: `Arc::new(multi_raft)` → `start_lifecycle_with_data()` → `tokio::spawn(start())`

## [0.9.0] - 2026-05-29

### Added

- **集群测试覆盖 (C3)**: 17 个新增测试, 总计 40 个集群测试 — L1 命令格式/错误路径 (22 个) + L2 路由全分支/集成 (9 个) + 路由格式验证 (8 个)
- **E2E 脚本扩展 (E)**: 6 个新增 shell 脚本 — `test_hash.sh` (Hash 全命令族), `test_string_extra.sh` (APPEND/INCR/MGET/MSET), `test_key_mgmt.sh` (RENAME/EXPIRE/COPY), `test_lua_json_ttl.sh` (从  移植改自启动), `test_persistence.sh` (从  移植), `test_restart_recovery.sh` (从  移植, kill-9 崩溃恢复)
- **`jsonpath_util.rs` (D.4)**: 从 `jsonpath.rs` 拆分, 提取 `split_top_level`/`json_equal`/`json_compare` 工具函数及测试

### Changed

- **jsonpath 重构 (D.1)**: 提取 `traverse_path` 消除 4 处重复路径遍历逻辑 (~250 行→~30 行); 移除全部 9 个文件级 `#[allow(clippy::*)]` 抑制; 修复 30 个 clippy 错误 (collapsible_if/match/cloned_ref_to_slice_refs/len_zero/manual_strip/map_or 等)
- **AiDb 引擎指标集成**: `Metrics::new()` 调用 `aidb::metrics::register_into()`, 13 个 `aidb_*` 指标通过 `GET /metrics` 暴露
- **`#[allow(dead_code)]` → `#[expect(dead_code)]` (D.5)**

## [0.8.0] - 2026-05-28

### Added

- **日志初始化 (P0)**: `init_logging()` 支持 `AIKV_JSON_LOG` 环境变量切换 JSON/人类可读格式; `EnvFilter` 配置; 使用 `try_init()` 避免重复初始化 panic
- **统一 Metrics 结构体 (P0)**: `Metrics` 结构体整合 18 个 Prometheus 指标 (15 基础 + 3 集群), 自定义 `Registry`; `ServerMetrics` 所有方法 (`on_command`/`on_connect`/`on_keyspace_hit` 等) 同步更新 Prometheus 指标
- **Metrics HTTP 服务器 (P0)**: `metrics_server.rs` — 独立端口 (默认 9191) 暴露 `/metrics` (Prometheus text/plain)、`/health`、`/` 端点; 仅在 `monitoring` feature 下编译
- **CLI 参数 (P0)**: `--metrics-port` (默认 9191)、`--metrics-addr` (默认 127.0.0.1)
- **OTel 导出器 (P1)**: `init_otel_tracer()` 支持 `AIKV_OTLP_ENDPOINT` 环境变量; 通过 `opentelemetry-otlp` 批式导出 tracing span; 失败降级不阻塞启动
- **可观测性测试**: Prometheus 集成测试验证 `connects_total`/`commands_total` 数据流; 依赖 `monitoring` feature
- **Raft snapshot span**: `raft_snapshot`/`raft_install_snapshot` 添加 `#[instrument]`

### Changed

- **tracing 订阅者**: 从 `tracing_subscriber::fmt().json().init()` 升级为 `init_logging()` — `AIKV_JSON_LOG=false` 输出人类可读 `compact` 格式
- **ServerSharedState**: 新增 `metrics_port`/`metrics_addr`/`prometheus_metrics` 字段; `new_with_backup_dir` 新增 `metrics_port`/`metrics_addr` 参数
- **ServerMetrics**: 新增 `prom: Option<Arc<Metrics>>` 字段和 `with_prometheus()` 方法; 默认行为不变 (无 monitoring feature 时无 prometheus 同步)

## [0.7.1] - 2026-05-28

### Added

- **SETSLOT 子命令补齐 (P0)**: MIGRATING (SlotMigrationManager.start_migration), IMPORTING (本地 `importing_slots: HashMap<u16, u64>` 轻量追踪), STABLE (commit_migration + 清除本地状态)
- **集群路由测试 6 分支覆盖 (P0)**: LocalLeader Execute, Readonly replica Execute, MOVED (not local), ASK (Migrating + asking), CLUSTERDOWN (unallocated), ASKING flag set; 单一大测试函数绕过 OnceLock 限制
- **Lua JSON via redis.call (P1)**: 9 个 JSON 命令 (SET/GET/DEL/TYPE/STRLEN/ARRLEN/OBJLEN/NUMINCRBY/ARRAPPEND) 在 `redis.call()`/`redis.pcall()` 内可用; 新建 `script/json_exec.rs`; 18 个新增测试
- **CLAUDE.md**: 新增 `Known Limitations` 章节 (集群/持久化/功能三节)
- **CI ignored 测试**: 新增 `ignored` CI job 定期运行 3 个 `#[ignore]` 慢测试

### Changed

- `ClusterStateManager`: 新增 `importing_slots: RwLock<HashMap<u16, u64>>` 字段
- `commands.rs` `cluster_set_slot`: MIGRATING/IMPORTING/STABLE 子命令改为实际实现

## [0.7.0] - 2026-05-28

### Added

- **Phase 16 缺口闭合**:
  - `CLUSTER DELSLOTS` 实现: 通过 `MetaRequest::UnassignSlots` propose 释放槽
  - `CLUSTER SETSLOT NODE` 实现: 通过 `MetaRequest::AssignSlots` 分配 slot 到目标 Group
  - `CLUSTER COUNTKEYSINSLOT` / `GETKEYSINSLOT` 实现: `block_on(scan_keys)` 扫描本地 Group key 计数
  - 18 个 Cluster 函数添加 `#[instrument]` tracing 标注 (含 `kv_cluster_route`, `gossip_tick`, `kv_cluster_cross_slot`)
  - Metrics counters: `aikv_gossip_messages_total` (gossip tick) + `aikv_failover_total` (failover)
  - `ClusterStateManager` 新增 `metrics: Option<Arc<ServerMetrics>>` 字段
  - `start_background_refresh` loop 内增加 gossip metrics 计数
  - 路由测试覆盖: 新增 MOVED + CLUSTERDOWN 分支测试 (cluster_routing.rs)
  - `parse_int` 测试: 3 个新测试 (valid/invalid/overflow)
  - `CLUSTER NODES` 锁优化: `role.read()` 提前释放, 一次读取存入局部变量
- **集群协议 (Phase 16)**: Redis Cluster 协议 Kv 侧适配层 (`src/cluster/`, cluster feature)
  - ClusterStateManager: 16384 槽表 + 节点拓扑缓存 + 本地 Group Leader 缓存, 全局 `OnceLock` 单例
  - ClusterRouter: `decide()` 同步路由决策 (Execute / MOVED / ASK / CLUSTERDOWN), CROSSSLOT 检查
  - ClusterConnectionState: per-connection asking/readonly 状态, ASKING 自动重置
  - GossipState: 轻量节点拓扑缓存 (故障检测委托 AiDb MetaRaft)
  - CLUSTER 命令 (20+): INFO, NODES, SLOTS, KEYSLOT, MYID, MEET, FORGET, ADDSLOTS, DELSLOTS, SETSLOT, FAILOVER, REPLICATE, REPLICAS, SHARDS, MYSHARDID, COUNTKEYSINSLOT, GETKEYSINSLOT, SAVECONFIG, BUMPEPOCH, SET-CONFIG-EPOCH, COUNT-FAILURE-REPORTS
  - MOVED/ASK 集群路由: `CommandRouter::execute_with_client` 扩展 `conn_state` 参数, `cluster_route()` 方法实现 admin 命令过滤 + MSET 感知 CROSSSLOT + MOVED/ASK/CLUSTERDOWN 响应
  - READONLY / READWRITE / ASKING: Connection 层内联处理, 不经过 CommandRouter
  - HELLO 响应集群模式: mode/role 读取 ClusterStateManager
  - CommandRegistry +4 (CLUSTER, READONLY, READWRITE, ASKING)
- 测试: 3 个集成测试文件 (cluster_skeleton, cluster_routing, cluster_commands)

### Changed

- `Connection`: 新增 `cluster_state` 字段; ASKING 重置钩子; HELLO mode/role 改为动态
- `CommandRouter::execute_with_client`: 新增 `#[cfg(feature = "cluster")] conn_state` 参数
- `src/lib.rs`, `src/main.rs`, `src/error.rs`: 增加 recursion_limit 256, cluster 模块声明, error 变体
- Cargo.lock: aidb cluster feature 依赖自举

## [0.6.0] - 2026-05-26

### Added

- **持久化运维兼容 (Phase 11.6′)**: SAVE / BGSAVE / LASTSAVE / SHUTDOWN
  - aidb: `Checkpoint::create` → `{data_dir}/backup/`; `INFO persistence`; `CONFIG GET appendonly` = no
  - memory: 启动 WARN; SAVE/BGSAVE → ERR; LASTSAVE → 0
  - Server graceful shutdown (`CancellationToken`); `KvStorage` 桥接 (`flush_engine` / `create_checkpoint` / `close_engine`)
- **AiDb 0.7.5**: `engine/checkpoint/` MVP + `checkpoint_in_progress` 并发协议
- CommandRegistry +4 (COMMAND COUNT **126**)

## [0.5.0] - 2026-05-26

### Added

- **Lua 脚本 (Phase 11.2–11.4)**: EVAL / EVALSHA / SCRIPT (LOAD/EXISTS/FLUSH/KILL)
  - mlua Lua 5.4 + 沙箱 (StdLib 裁剪 + 危险全局封印)
  - `redis.call` / `redis.pcall` (`{err=...}` 表); `ScriptTransaction` + TTL 保留
  - KEYS 校验; `KeyLock::lock_keys_sorted`; LRU 256 脚本缓存 (**仅 SCRIPT LOAD 写入**)
  - 超时 5s + 内存 128MB; AiKv 命令子集 async 移植
- **可观测性**: `cmd_eval` / `cmd_evalsha` / `cmd_script_load` span; `cmd_lua_exec` (duration_us) + `cmd_lua_redis_call` (cmd.name); `on_lua_command` + `on_lua_execution` metrics
- **测试**: 22 项 Lua L1 + `test_tcp_eval_basic` + `test_aidb_script_roundtrip` + `e2e/test_lua.sh`
- CommandRegistry +3 (COMMAND COUNT 122)
- `StoredValue::as_hash()` (脚本 Hash 路径)

## [0.4.0] - 2026-05-26

### Added

- **JSON 命令 (Phase 11)**: JSON.SET/GET/DEL/TYPE/STRLEN/ARRLEN/OBJLEN, JSON.NUMINCRBY/ARRAPPEND/UPDATE/MSET
  - JSONPath **P11-core**: `$`, `.`, `$.field`, `$[N]` (负索引 → ERR)
  - JSONPath **P11-ext**: `[*]`, `['f1','f2']`, `[?(@...)]`, `=~` (contains 近似); SET 扩展 NX/XX/XE/NN + filter 预检
  - 整文档存为 `ValueType::String`; RMW 经 `write_back_json` 保留 TTL
  - `KeyLock::lock_keys_sorted` 供 JSON.MSET 多 key 字典序加锁
- **可观测性**: 每 JSON handler `#[instrument(name = "cmd_json_*")]` (含 key/path 等字段); `on_json_command` metrics; DEBUG `target = "cmd.json"`
- **测试**: 32 项 JSON L1 + `test_tcp_json_set_get` + `test_aidb_json_roundtrip` + `e2e/test_json.sh`
- CommandRegistry +11 (COMMAND COUNT 119)

## [0.3.3] - 2026-05-26

### Changed

- **AiDb 适配器**: 实现 `StorageAdapter::delete_range`; `FLUSHDB`/`clear` 改走 `delete_range` 前缀删除 (依赖 AiDb 0.7.4)

## [0.3.2] - 2026-05-26

### Added

- **List 扩展**: LINSERT, LMOVE, LPOS
- **Set 扩展**: SMOVE, SSCAN (`scan_util` 与 ZSCAN 共享分页 helper)
- **Key 扩展**: EXPIRETIME, PEXPIRETIME, DUMP, RESTORE, MIGRATE
  - DUMP/RESTORE 使用 AiKv 内部格式 `[version: u8=0][bincode(StoredValue)]`, **与 Redis DUMP 不兼容**
  - MIGRATE 仅支持 localhost 目标 (`127.0.0.1` / `::1`); 非 localhost 返回 `ERR not supported`
- **Server 扩展**: SLOWLOG GET/LEN/RESET, COMMAND COUNT/INFO/GETKEYS/DOCS/HELP, LATENCY HISTOGRAM/LATEST/HISTORY/RESET/HELP
  - LATENCY 为内存直方图 (无 Prometheus); RESP2 返回 Array, RESP3 返回 Map
  - COMMAND DOCS 返回空 Array (Redis 7.0+ 占位, 无文档正文)
- **基础设施**: SlowQueryLog, LatencyStats, CommandRegistry (~96 条命令元数据)
- **CONFIG**: `slowlog-log-slower-than`, `slowlog-max-len` 白名单键; GET 读 SlowQueryLog 实值
- **E2E**: `e2e/test_ext.sh` (LINSERT, SMOVE, DUMP/RESTORE, SLOWLOG, EXPIRETIME)
- 测试: P10-ext L1 (`test_linsert_*`, `test_smove_*`, `test_dump_*`, `test_migrate_*`, `test_slowlog_*`, `test_command_*`, `test_latency_*`); `test_aidb_dump_restore_roundtrip`

### Changed

- `Connection::execute_with_client` 记录命令耗时, 写入 SlowQueryLog 与 LatencyStats (排除 SLOWLOG/MONITOR/PING/ECHO/HELLO/QUIT)
- `CommandRouter` 传递 `protocol_version`, LATENCY 按 RESP2/RESP3 选择响应结构

## [0.3.1] - 2026-05-25

### Added

- **INFO keyspace**: `expires` 统计带 TTL 的 key 数量 (替代固定 `expires=0`)
- **keyspace 指标**: GET/MGET/EXISTS/HGET 记录 hits/misses; INFO stats 输出 `keyspace_hits` / `keyspace_misses`
- **KvStorage**: `KeyspaceStats` + `keyspace_stats(db)` (keys / expires)
- 测试: `test_info_keyspace`, `test_info_stats_keyspace_hits_misses`, `test_command_metrics_recorded`, `test_memory_engine_keyspace_stats`

### Changed

- `CommandRouter::new_with_shared` 注入 metrics, 读命令路径记录 keyspace 命中
- E2E 默认端口改为随机高位端口, 避免与本地 6399 冲突
- README 按实际能力描述 (JSON/Lua/集群尚未实现)

## [0.3.0] - 2026-05-25

### Added

- **List 命令**: LPUSH, RPUSH, LPOP, RPOP, LLEN, LRANGE, LINDEX, LSET, LREM, LTRIM
- **Set 命令**: SADD, SREM, SISMEMBER, SMEMBERS, SCARD, SPOP, SRANDMEMBER, SUNION, SINTER, SDIFF, SUNIONSTORE, SINTERSTORE, SDIFFSTORE
- **Sorted Set 命令**: ZADD, ZREM, ZSCORE, ZRANK, ZREVRANK, ZRANGE, ZREVRANGE, ZRANGEBYSCORE, ZREVRANGEBYSCORE, ZCARD, ZCOUNT, ZINCRBY, ZSCAN, ZPOPMIN, ZPOPMAX, ZRANGEBYLEX, ZREVRANGEBYLEX, ZLEXCOUNT (ZSCAN 为全量游标)
- **Key 管理**: KEYS, SCAN, RANDOMKEY, RENAME, RENAMENX, TYPE, COPY
- **Server**: INFO (server/clients/memory/stats/keyspace), TIME, CONFIG GET/SET, CLIENT LIST/SETNAME/GETNAME
- **MONITOR**: Connection 内联广播; Monitor 模式 `select!`, 仅处理 QUIT
- **AiDb 持久化**: `StorageAdapter`, `KvStorageAdapter`, `AiDbEngine`; CLI `--engine memory|aidb --data-dir`
- **存储/并发**: `StoredValue::as_list/as_set/as_zset`, `KeyLock::lock_two`, `KvStorage` rename/copy/random
- **可观测性**: tracing spans `cmd_list`, `cmd_set`, `cmd_zset`, `cmd_keys`, `cmd_server`
- **E2E**: `e2e/test_basic.sh`, `e2e/test_datatypes.sh` (shell, 需 `redis-cli`)
- 测试: `--test commands` 56, `--test storage` 17, `--test server` 28 + 2 `#[ignore]` stress

### Changed

- `ServerSharedState::new(config, storage, tcp_port)`: 注入 storage; `CommandRouter` 经 OnceLock 延迟初始化; 扩展 clients / monitor / config_map
- `src/storage/` 引用 `aidb` (持久化引擎路径)

## [0.2.0] - 2026-05-22

### Added

- **存储层**: `KvStorage` trait, `StoredValue`/`ValueType`, `MemoryEngine` (16 db, 惰性过期, glob `keys`, 内部 `scan`)
- **命令层**: `CommandRouter` + `KeyLock`; String / Hash / Database / Key 过期命令
- **服务集成**: `Connection.current_db`, 非内联命令走 router; `aikv_commands_total` 计数
- **可观测性**: spans `mem_engine_*`, `kv_command`, `cmd_string`, `cmd_hash`, `cmd_keys`
- 测试: `--test storage` 13, `--test commands` 35, `--test server` 21 + 2 `#[ignore]` stress

### Changed

- 运行时仍无 `aidb` 代码引用 (`Cargo.toml` path 依赖保留供后续持久化)

## [0.1.0] - 2026-05-22

### Added

- **RESP 协议**: `src/protocol/` — `RespValue`, `RespParser`, RESP2/RESP3 编码
- **TCP 服务**: `src/server/` — `Server::run`, 单连接 `Connection` 读写循环
- **内联命令**: PING, ECHO, HELLO (2/3/无参), QUIT; 未知命令 `-ERR unknown command`
- **连接指标**: `connections_total`, `connected_clients` (AtomicU64)
- **可观测性**: spans `kv_accept`, `kv_connection`, `kv_read`, `kv_write`, `kv_parse`, `kv_encode`
- 测试: `--test resp` 66, `--test server` 13 + 2 `#[ignore]` stress
- CLI: `cargo run -- --bind 127.0.0.1:6379`

### Changed

- 无 `aidb` 代码引用 (`Cargo.toml` path 依赖保留)

## [0.0.1] - 2026-05-18

### Added

- 项目骨架: Cargo.toml, `error` 类型, tracing JSON 入口占位
