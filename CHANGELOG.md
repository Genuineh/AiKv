# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.3] - 2026-04-29

### Added

- **`JSON.NUMINCRBY`**：对 JSON 文档中指定路径的数值进行原子递增/递减，支持整数与浮点数，兼容 `$[*]` 通配符与 JSONPath 过滤器 `[?(@...)]`。
- **`JSON.ARRAPPEND`**：向 JSON 数组中追加一个或多个元素，支持深层嵌套路径与 `[*]` 通配符。
- **`JSON.UPDATE`**：更新 JSON 文档中指定字段的值，支持条件更新（`wherePath` 过滤器）、多字段批量更新及 `NN`（key 不存在时忽略）标志位。
- **`JSON.MSET`**：原子性批量设置多个 key 的 JSON 值，所有 key 需在同一 slot 内。
- **`ATOM.EXEC` JSON 支持**：事务中完整支持 `JSON.SET`（含子路径）、`JSON.DEL`（含路径级删除、过滤器匹配删除）、`JSON.NUMINCRBY` 等 JSON 命令的原子执行。
- **JSONPath 过滤器 `[?(@...)]`**：实现 JSONPath 过滤表达式求值，支持字段比较（`@.field == value`）、数值比较（`>`, `<`, `>=`, `<=`）、不等（`!=`）、布尔真值判断、正则匹配（`=~`）、取反（`!`）、`||` / `&&` 逻辑组合，以及嵌套 `Any()` 子过滤器。
- **JSONPath 嵌套数组支持**：过滤前自动扁平化嵌套数组结构，确保 `[?(@.field == v)]` 作用于内层数组元素而非外层包装。
- **路径解析增强**：`split_path_parts` 支持嵌套括号 `[?(@.List[?(@ == '2')])]` 的深度感知拆分，替代原有简单 `.split('.')` 方式；统一应用于 JSON_GET、JSON_SET、JSON_DEL、JSON_ARRAPPEND 等全部 JSON 命令及 ATOM.EXEC。
- **点分字段路径支持数组索引**：`eval_single_condition`、`filter_jsonpath` 等过滤器的字段路径（如 `InnerDocument.StrList`）现支持 `[N]` 数组下标索引，可精确匹配数组内元素。

### Changed

- **`flush_db` / `flush_all` Raft 元数据安全**：原实现末尾调用 `clear_all_data()` 会连带清除 Raft 内部键（`raft:vote`、`raft:log:N`、`raft:membership`），导致集群在 FLUSH 操作后 Raft 状态丢失、选主失败。现改为对每个数据组执行 `db().flush()`，仅将 MemTable 持久化为 SSTable，不破坏 Raft 日志与投票状态。

### Fixed

- **TL.KvDoc 兼容：字段选择器缺失字段处理**：`JSON.GET` 使用字段选择器（如 `$[*]['Str','IntValue','NotExistField']`）查询 JSON 文档时，对于存储文档中不存在的字段，返回值由 `null` 改为 `""`（空字符串），与 TL.KvDoc 的 `FixWhenSpecialFieldSingleGet` 解析器对齐。修复了读取含额外字段的 DTO 时 `NotExistField` 导致 `null → Int32` 反序列化失败，以及事务中 `JSON.DEL` 清除字段后回读实体时 `Str` 被误解析为数字的问题。
- **ATOM.EXEC JSON.SET 子路径双重编码修复**：在事务中执行 `JSON.SET` 子路径写入时，值不再被重复 JSON 序列化；采用读-改-写策略正确合并到现有文档。
- **ATOM.EXEC JSON.DEL 路径级删除**：事务中的 `JSON.DEL` 现支持完整的 JSONPath 路径（含 `[*]` 通配符、`[?()]` 过滤器），使用与独立 `JSON.DEL` 命令一致的括号感知路径拆分逻辑。
- **ATOM.EXEC JSON.SET 错误传播与 TTL 处理**：事务中 JSON 操作的正确错误传播，以及 TTL 参数解析。
- **SCAN 延迟直方图溢出**：`histogram_quantile` 在 usec 超过最大边界时无法正确计算延迟百分位数的 bug 已修复。
- **依赖 AiDb v0.7.3**：合并迭代器重写、幽灵 key 消除、Raft 元数据安全等改进随依赖同步。

## [0.2.2] - 2026-04-14

### Added

- **集群（slot 键枚举）**：`CLUSTER GETKEYSINSLOT` / `CLUSTER COUNTKEYSINSLOT` 基于 AiDb **`MultiRaftNode::scan_local_group_slot_keys_sync`** 扫描本地数据组 `sm:` 键空间（与 Multi-Raft 持久化路径一致）；枚举结果对非 0 号 DB 的物理键使用 **`user_key_from_physical_raft_key`** 还原为客户端可见 key（与 `physical_raft_storage_key` 对称）。
- **测试**：`tests/common`（`exec_cmd` 在启用 `cluster` 时为 `execute` 传入 `allow_importing_slot_once`；**`migrate_target_port`** 提供本地 RESP mock 以稳定覆盖 `MIGRATE`）；`cluster::commands::physical_key_tests` 覆盖物理键编解码往返。
- **集群（Multi-Raft）**：`initialize_cluster` 后启动后台任务，周期性 `sync_data_groups_from_meta` 并刷新路由元数据，降低数据组与路由不同步风险。
- **可配置同步间隔**：环境变量 **`AIKV_DATA_GROUPS_SYNC_INTERVAL_MS`**（默认 **2000**，最小 **100**）控制上述后台任务两次同步之间的间隔；启动后首次同步前仍有固定 **1s** 延迟。
- 添加 `scan_keys_in_db` 方法，实现真正的游标式 SCAN 操作
- AiDb storage adapter: 使用 AiDb 的 `dbsize()` 返回精确 key 计数
- **集群命令扩展**：`CLUSTER FAILOVER`（含 FORCE/TAKEOVER 与目标节点）、`CLUSTER METARAFT SETSTATUS`（运维调整节点状态）；`cluster` feature 可选依赖 **`redis`**，用于集群内部命令转发等路径。
- **集群（扩容 / 在线迁槽）**：`ensure_shard_group_for_node` —— 节点已在 `CLUSTER MEET` 元数据里但尚无数据分片组（空 master、未 `ADDSLOTS`）时，为 `SETSLOT` **`MIGRATING` / `IMPORTING` / `NODE`** 按需 **`create_group`（`group_id = node_id`）** 与 **`update_group_leader`**，并 **`schedule_post_meta_sync`**；与「先 ADDSLOTS 再迁槽」路径对齐。
- **集群（MetaRaft leader 转发）**：`forward_meet_to_leader` —— 在非 MetaRaft leader 上 `CLUSTER MEET` 遇 **`ForwardToLeader`** 时，向解析出的 leader **Redis** 地址转发同名命令以提交 `add_node`。
- **`SAVE`/`BGSAVE` 文件级备份**：AiDb 引擎下 `SAVE`/`BGSAVE` 改用底层 `BackupManager::create_backup`（flush MemTable + 拷贝 SSTable/WAL 文件），替代原有低效的 RDB 全量序列化；`AiDbStorageAdapter` 与 `ClusterRaftEngine` 均新增 `create_backup()` 分发。集群模式下对本节点所有 Raft 分片执行同样备份。
- **`backup-dir` 配置项**：`CONFIG GET/SET backup-dir` 可动态修改备份目录（默认 `{data_dir}/backups/`），启动时由 `data_dir` 自动推导。
- **`BGSAVE` 真后台执行**：`BGSAVE` 改由独立线程 `std::thread::spawn` 执行，`AtomicBool` 防并发重复调用，立即返回 `"Background saving started"`；`LASTSAVE` 正常更新。

### Changed

- SCAN 命令改用 `scan_keys_in_db` 实现真游标，O(1) 每次调用
- KEYS 命令添加 60 秒超时保护
- 依赖 **AiDb v0.7.3**（路径依赖 `../AiDb`）：含 `backup_all_groups` 与 `OpenRaftStorage::db` API，支持 `SAVE`/`BGSAVE` 文件级备份
- **Raft 快照策略**：集群节点（`ClusterNode::initialize`）与服务端（`Server::initialize_cluster`）的 Raft 配置统一设置 **`SnapshotPolicy::Never`**，禁用自动快照触发。当前快照 payload 为空（尚未实现全量状态导出），自动触发仅增加无效 I/O 开销；显式禁用后可避免空快照覆盖日志截断点
- **进程入口（集群 + debug 日志）**：使用 `tokio::runtime::Builder` 为多线程运行时设置较大工作线程栈（8 MiB），并收敛 tracing-subscriber 初始化（默认文本、避免过重的 JSON/span 序列化），减轻 `RUST_LOG=debug` 下 Tokio 工作线程栈溢出风险，便于 Docker 集群部署。
- **`extract_data_address`（gRPC → Redis 数据端口）**：Raft 监听端口识别范围由 **50051–50056** 扩展为 **50051–50057**，与第三分片默认 **50057 ↔ 6385**（`6379 + (port - 50051)`）等 compose 布局一致。
- **`MIGRATE` 目标解析**：由 **`SocketAddr` 字面量解析** 改为 **`ToSocketAddrs`**，支持 **主机名**（如 Docker DNS `aikv-master-3`）；集成测试 mock 仍可使用 **`127.0.0.1`**。
- **`CLUSTER INFO` 一致性检查**：`cluster_state:ok` 新增 `slot_maps_to_known_group` 校验——每个非零 slot 映射的 `group_id` 必须在 `meta.groups` 中真实存在；避免「slot 数量看似满、但映射指向孤儿 group」时仍报 ok。

### Fixed

- **集群 `INFO memory` 的 AiDb 指标不再恒为 0**：`ClusterRaftEngine::get_aidb_stats` 改为调用 `MultiRaftNode::aggregate_aidb_storage_stats`，返回本节点所有已加载 Raft 数据组的真实 MemTable/WAL/BlockCache 统计，修复 `aidb-exporter` 与 Grafana 面板（AiDb MemTable/WAL/Block Cache）读数长期为 0 的问题。
- **集成测试**：在 `--features cluster` 下 `CommandExecutor::execute` 需额外布尔参数导致的编译失败；各 `tests/*.rs` 统一经 `common::exec_cmd` 调用。`MIGRATE` 用例改为连接 **`127.0.0.1` 动态端口** mock，并修正断言——**不**再要求迁移后数据出现在同一进程内的其它逻辑 DB（数据仅经 TCP 发往对端）。
- FLUSHDB 现在调用 `clear_all_data()` 清除所有 SSTable 和 WAL 文件
- 修复 `key_exists` 跳过 `__exp__:` 前缀的内部 key
- 修复 SCAN 命令延迟直方图溢出 bug：当 usec > max_bound 时未增加任何 bucket，导致 `histogram_quantile` 无法正确计算延迟百分位数
- **集群读与 MOVED**：`cluster_raft` 读路径在「本地无该数据 Raft 组存储」时返回 Redis `MOVED`，按 slot 查 `ClusterMeta` 中 group leader（或副本）在 `nodes` 里登记的外部地址。
- **集群写与故障转移后路由**：写路径遇 AiDb `ForwardToLeader` 时，按 **slot** 解析 **`ClusterMeta` 当前组 leader**（及副本回退）地址，经 **`AIKV_ADVERTISE_HOST`** 与 **50051↔6379 宿主机端口** 规范化后返回 `MOVED`，避免仅依赖错误信息内 `leader_id` + 旧端口推导导致指向错误节点。若解析目标为**本机**但数据 Raft 仍无可用 leader（例如多数派未恢复），返回明确 **`TRYAGAIN Data group leader is converging after failover`**，避免客户端 **MOVED 自环**。`Raft write batch timeout` 映射为存储错误时增加可观测日志。
- **集群服务**：`initialize_cluster` 与连接层对集群命令/超时与可观测性的小幅调整，与上述存储路由行为一致。
- **`CLUSTER NODES` 角色**：**`slave`** 仅当节点出现在某组 **`replicas`** 且 **不是该组 `leader`**；尚未持槽、且非他人副本的新 master（扩容空节点）显示为 **`master`**，避免误标为 `slave` 及错误的 `master` 指向。
- **`CLUSTER METARAFT SETSTATUS` 转发回环**：转发链路增加单跳标记 `__FORWARDED__` 与“已转发不再二次转发”保护；解析到 leader Redis 地址为 `127.0.0.1` / `localhost` / `::1` 时拒绝转发（容器内 loopback 防护），并对超长嵌套错误文本做截断日志，避免错误链递归膨胀导致副本内存异常增长。
- **迁移路径路由错误**：`IMPORTING`/`MIGRATING` 分支在无法解析 MOVED/ASK 目标地址时，不再静默落入后续逻辑，而是返回含因的明确 `CLUSTERDOWN` 错误，便于与「slot 未分配」区分。

## [0.1.1] - 2026-03-26

### Fixed

- 修复 keyspace scan 导致 CPU 100% 问题，改为使用 O(1) 的 estimated_dbsize 替代 O(N) 全表扫描
- 修复后台 flush 后 WAL 未轮转问题，导致 WAL 文件无限增长 (2.8GB+)
- 依赖 AiDb v0.7.1，修复 MemTable key_map 计数 bug（put-after-delete 和重复 delete 场景）
- 默认存储引擎从 memory 改为 aidb

### Performance

- Keyspace scan: O(N) → O(1)，使用序列号估算

## [0.1.0] - 2026-01-09

### Changed
- **AiDb v0.6.3 Upgrade (2026-01-09)**
  - Upgraded AiDb dependency from v0.6.2 to v0.6.3
  - Fix: MemTable tombstone visibility and `DB::get()` behavior — tombstones in MemTable now block older SSTable values, resolving the issue where `DEL` returned success but `EXISTS` still returned true.
  - Verification: Rebuilt cluster and ran TL.Redis test suite locally — all 63 tests passed (previously 5 failures related to this issue).

- **AiDb v0.5.1 Upgrade (2025-12-11)**
  - Upgraded AiDb dependency from v0.5.0 to v0.5.1
  - Refactored cluster implementation to use AiDb v0.5.1's official Multi-Raft API
  - Adopted legacy compatibility layer during migration
  - All 211 tests pass (118 library + 93 cluster)
  - Exported AiDb v0.5.1 new APIs: ClusterMeta, MigrationManager, MembershipCoordinator, etc.
  - Created minimalist implementation prototypes for future optimization (~84% code reduction potential)
  - Zero-downtime upgrade with backward compatibility

### Added
- **P2: Server 命令补全 (2025-12-01)**
  - `COMMAND` - 获取所有命令的详细信息（名称、参数数量、标志、键位置等）
  - `COMMAND COUNT` - 获取支持的命令总数
  - `COMMAND INFO` - 获取指定命令的详细信息
  - `COMMAND DOCS` - 获取命令文档
  - `COMMAND GETKEYS` - 从完整命令中提取键名
  - `COMMAND HELP` - 显示帮助信息
  - `CONFIG REWRITE` - 重写配置文件（存根实现）
  - `SAVE` - 同步保存数据到磁盘
  - `BGSAVE` - 异步保存数据到磁盘
  - `LASTSAVE` - 获取上次成功保存的 Unix 时间戳
  - `SHUTDOWN` - 请求关闭服务器（支持 NOSAVE/SAVE/NOW/FORCE/ABORT 选项）
  - Server 命令从 9 个增加到 16 个
  - 新增 4 个单元测试验证新命令功能

---

[Unreleased]: https://github.com/Genuineh/AiKv/compare/v0.2.3...HEAD
[0.2.3]: https://github.com/Genuineh/AiKv/compare/v0.2.2...v0.2.3
[0.2.2]: https://github.com/Genuineh/AiKv/compare/v0.1.1...v0.2.2
[0.1.1]: https://github.com/Genuineh/AiKv/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/Genuineh/AiKv/releases/tag/v0.1.0
