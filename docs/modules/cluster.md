---
name: aikv-cluster
depends_on:
  - aikv-storage
  - aidb-cluster
description: AiKv Redis Cluster protocol — MOVED/ASK/CROSSSLOT routing, CLUSTER subcommands, connection ASKING/READONLY, init_cluster wiring to aidb MetaRaft/MultiRaft, AnnounceResolver, slot migration hooks. Use when changing src/cluster/*, command/router cluster_route, main init_cluster, debugging redirects, CLUSTER MEET/SETSLOT/FAILOVER, or cluster feature startup.
---

# AiKv Cluster (Redis Cluster 协议层)

## 何时读本文

- 改 `src/cluster/*`、`main.rs` 的 `init_cluster`, 或 `command/router.rs` 的 `cluster_route`
- 排查 MOVED/ASK/CROSSSLOT/CLUSTERDOWN、slot 迁移 (SETSLOT/MIGRATE)、failover、节点 MEET/FORGET
- 理解 aikv 与 aidb 在集群上的分工
- **不覆盖**: MetaRaft/MultiRaft/Router 实现、slot 迁移状态机、gRPC → [aidb cluster.md](../../../aidb/docs/modules/cluster.md)
- **不覆盖**: 数据面 `propose_group` / `ClusterDataAdapter` → [storage.md](storage.md)
- **不覆盖**: 命令分发骨架 / CROSSSLOT 前置插入点 → [commands-core.md](commands-core.md)
- **不覆盖**: MIGRATE TCP/RESTORE → [commands-extended.md](commands-extended.md)
- **不覆盖**: `aikv_cluster_redirects_total` / INFO cluster 段 → [observability.md](observability.md)
- **构建**: `--features cluster` (启用 `aidb/cluster`); aidb 侧需 `protoc`

## 与 aidb 的分工

| 层 | 仓库 | 职责 |
|----|------|------|
| 共识与拓扑 | aidb | MetaRaft (group 0)、MultiRaft 数据 group、Router/slot 表、MembershipCoordinator、SlotMigrationManager |
| RESP 适配 | aikv (本章) | MOVED/ASK 决策、CLUSTER 子命令 RESP、连接态、client 地址通告、进程装配 |
| KV 读写 | aikv storage | slot 已分配时写经 Raft (`ClusterDataAdapter`) |

权威拓扑 **始终** 来自 MetaRaft; aikv **不** 独立维护 slot/成员状态.

## 架构一览

```mermaid
flowchart TB
  CLI["Redis client"]
  Conn["Connection\nClusterConnectionState"]
  CR["CommandRouter::cluster_route"]
  DEC["ClusterRouter::decide"]
  FWD["forward_command"]
  DIS["dispatch_cluster"]
  CSM["CLUSTER_STATE_MGR"]
  MR["aidb MetaRaftNode"]
  MRN["aidb MultiRaftNode"]
  CDA["ClusterDataAdapter"]

  CLI --> Conn --> CR
  CR --> DEC
  DEC -->|Execute| CDA
  DEC -->|Moved/Ask| FWD
  CR -->|CLUSTER| DIS
  DIS --> MR
  DIS --> MRN
  CSM --> MR
  CSM --> MRN
  CDA --> MRN
```

## 代码地图

| 路径 | 职责 | 入口 |
|------|------|------|
| `cluster/mod.rs` | 模块根; re-export `key_to_slot`/`extract_hash_tag` | `pub mod cluster` |
| `cluster/state.rs` | 全局单例; leader 缓存; coordinator 注入 | `CLUSTER_STATE_MGR`, `ClusterStateManager` |
| `cluster/router.rs` | 同步路由决策 | `ClusterRouter::decide`, `check_cross_slot` |
| `cluster/forward.rs` | 单端点透明 TCP 转发; EVAL 路由 key | `forward_command`, `cluster_routing_key` |
| `cluster/connection.rs` | 每连接 asking/readonly | `ClusterConnectionState` |
| `cluster/announce.rs` | MOVED/SLOTS 客户端地址; 内部转发 TCP 地址 | `AnnounceResolver`, `AnnounceMode` |
| `cluster/commands.rs` | CLUSTER 子命令 + `dispatch_cluster` | `cluster_meet`, `cluster_set_slot`, … |
| `cluster/gossip.rs` | 拓扑 tick: leader 缓存 + gossip metrics | `start_background_refresh` |
| `cluster/config_auto_save.rs` | MetaRaft version → `nodes.conf` | `ConfigAutoSave::run` |
| `cluster/replication.rs` | HELLO/INFO role 标签 | `node_replication_role` |
| `command/router.rs` | 普通命令 cluster 前置 + `CLUSTER` dispatch | `cluster_route`, `execute_with_client` |
| `server/connection.rs` | ASKING/READONLY/READWRITE; 传/重置 asking | `Connection::run` |
| `main.rs` | CLI + `init_cluster` + `build_storage` 包 adapter | `init_cluster` (~L209–501) |

> 实现文件为 `cluster/commands.rs`, **非** `command/cluster_commands.rs`.

## 关键 invariant (勿破坏)

- **`CLUSTER_STATE_MGR`**: `init_cluster` 成功后才 `set`; 未 set 时 `cluster_route` 跳过 (非 cluster 单机行为).
- **Assigned slot 写**: 必须经数据面 Raft (`ClusterDataAdapter`); **禁止** 写 local fallback (见 storage.md).
- **`ClusterRouter::decide` 同步**: 只读缓存 + MetaRaft 快照; 不 `.await` OpenRaft.
- **IMPORTING 写窗口**: 目标节点 `importing_slots` 或连接 `ASKING` + Migrating/IMPORTING 状态 → `Execute`; 与 MIGRATE RESTORE 配合.
- **ASKING 一次性**: `Connection` 每命令执行后 `reset_asking`.
- **readonly replica 读**: `READONLY` + `CommandType::Read` + 本地 group → 本地读; 写仍 MOVED 到 leader.
- **admin 白名单**: `cluster_route` 内命令 (PING/MIGRATE/SCAN/INFO/…) **不** 按 key 路由.
- **CROSSSLOT**: 多 key 命令 (MGET/MSET/DEL/BLPOP/…) 须同 slot; MSET key 在偶数下标.
- **Announce unknown 模式**: 客户端见 `:port`; 进程内 `forward_command` 用 `tcp_connect_addr` (rpc host + client port).

## 数据流

### 进程启动 (`init_cluster`)

1. 打开/复用 aidb `DB` (`use_wal: true`)
2. `MetaRaftNode` + 共享 `RaftServiceDispatcher` + Meta gRPC (`--cluster-rpc-addr`)
3. bootstrap (`peers` 空) 或 join peers; 后台 `AIKV_CLIENT_ADDR` → MetaRaft
4. `Router` + `LifecycleManager` + `MultiRaftNode::start_lifecycle_with_data`
5. 数据面 gRPC: `rpc_port + cluster_data_port_offset` (默认 +10000)
6. `MembershipCoordinator` + `SlotMigrationManager` 注入 `ClusterStateManager` → `CLUSTER_STATE_MGR.set`
7. 后台: 拓扑 tick (leader 缓存 + metrics)、LeaderChangeWatcher → `apply_observed_group_leader`、`ConfigAutoSave` → `nodes.conf`

`build_storage` (aidb 路径): `StorageAdapter` → `ClusterDataAdapter` → `KvStorageAdapter`.

### 普通命令 (非 CLUSTER)

```mermaid
sequenceDiagram
  participant C as Connection
  participant R as CommandRouter
  participant D as ClusterRouter
  participant F as forward_command
  participant K as KvStorage

  C->>R: execute_with_client(..., cluster_state)
  R->>R: cluster_route (若 mgr 已 set)
  alt admin 白名单
    R->>K: execute_inner
  else CROSSSLOT
    R-->>C: -CROSSSLOT
  else decide Execute
    R->>K: execute_inner
  else Moved/Ask
    R->>F: TCP 转发 (可选 ASKING)
    F-->>C: 目标节点响应
  end
  C->>C: reset_asking
```

### Slot 迁移 (Kv 侧协调)

1. 源: `CLUSTER SETSLOT <slot> MIGRATING <target-id>` → `SlotMigrationManager::start_migration`
2. 目标: `CLUSTER SETSLOT <slot> IMPORTING <source-id>` → 本地 `importing_slots`
3. `MIGRATE` + `ASKING` + RESTORE (commands-extended)
4. `CLUSTER SETSLOT <slot> STABLE` → 清 `importing_slots` + `commit_migration`

源节点 Migrating 写 → **ASK**; 目标 IMPORTING 写 → **Execute** (含 router `importing_slots` 短路).

## 关键类型

| 类型 | 说明 |
|------|------|
| `RouteDecision` | `Execute` / `Moved` / `Ask` / `ClusterDown` |
| `CommandType` | `Read` / `Write` / `Admin` |
| `ClusterConnectionState` | `asking`, `readonly` |
| `AnnounceMode` | `Fixed` (完整 host:port) / `UnknownEndpoint` (默认 `:port`) |
| `ReplicationRole` | `Primary` / `Replica { primary_id }` (由 `CLUSTER REPLICATE` 设置) |

## CLUSTER 子命令

### Redis 标准 (已实现)

| 子命令 | 入口 | 说明 |
|--------|------|------|
| KEYSLOT, MYID | `cluster_keyslot`, `cluster_myid` | |
| INFO, NODES, SLOTS, SHARDS, MYSHARDID | `cluster_*` | INFO: `cluster_state` 按 slot 覆盖 + group leader 动态 ok/fail |
| COUNTKEYSINSLOT, GETKEYSINSLOT | scan 本地 group SM | |
| MEET, FORGET [FORCE] | `MembershipCoordinator` | MEET NotLeader 退避重试 |
| ADDSLOTS [NODE id], DELSLOTS | MetaRaft `AssignSlots`/`UnassignSlots` | 无 group 时 ADDSLOTS 可自动 CreateGroup |
| SETSLOT MIGRATING/IMPORTING/NODE/STABLE | migration mgr + 本地 map | |
| REPLICATE, REPLICAS | 元数据 / 列表 | REPLICATE 仅本地 role |
| FAILOVER [FORCE\|TAKEOVER] | `change_group_membership` 升主 | 需 replica role |
| SAVECONFIG, BUMPEPOCH | `nodes.conf` / MetaRaft propose | |

### AiKv 扩展

| 子命令 | 说明 |
|--------|------|
| CREATEGROUP | 空 data group, 不分配 slot |
| ADD_REPLICA / DEL_REPLICA | group 成员变更 (跨 group 可仅 MetaRaft 元数据) |
| REBALANCE | `SlotMigrationManager` 再均衡 |

### 未实现 / stub

| 子命令 | 行为 |
|--------|------|
| CLUSTER RESET | **不支持** — 返回 ERR; 见下方 [重置集群](#重置集群无-cluster-reset) |
| CLUSTER METARAFT * | 不在 aikv; 见 [aidb cluster.md](../../../aidb/docs/modules/cluster.md) |
| SET-CONFIG-EPOCH | 恒 OK |
| COUNT-FAILURE-REPORTS | 恒 0 |

NotLeader (MetaRaft propose): `map_propose_error` → `MOVED 0 <addr>` 或 CLUSTERDOWN.

## 配置与 feature flags

| 项 | 位置 | 说明 |
|----|------|------|
| `feature cluster` | `Cargo.toml` | `cluster = ["aidb/cluster"]` |
| `--cluster-node-id` | `main.rs` CLI | 节点 ID |
| `--cluster-rpc-addr` | `main.rs` CLI | MetaRaft gRPC |
| `--cluster-peers` | `main.rs` CLI | 加入集群 peer 列表 |
| `--cluster-data-port-offset` | `main.rs` CLI | 默认 10000 |
| `AIKV_CLIENT_ADDR` | env | 写入 MetaRaft `client_addr` |
| `AIKV_CLUSTER_ANNOUNCE_MODE` | env | `unknown` (默认) / `fixed` |
| gossip / lifecycle / autosave 间隔 | `main.rs` CLI | 拓扑 tick / lifecycle / autosave 间隔 |

## 常见任务

### 排查 MOVED 循环

1. 目标节点 `CLUSTER NODES` 中 `client_addr` 是否可达 (`AIKV_CLIENT_ADDR`)
2. `AIKV_CLUSTER_ANNOUNCE_MODE=fixed` 是否更适合 LAN
3. `LeaderChangeWatcher` / 拓扑 tick 是否刷新 router (`apply_observed_group_leader`)
4. 对比 aidb Router slot 表: `CLUSTER SLOTS` vs MetaRaft

### 手动 failover

1. 副本节点: `CLUSTER REPLICATE <master-id>` (元数据)
2. `CLUSTER FAILOVER FORCE` → 本节点 `change_group_membership` 为 sole voter
3. 验证 `CLUSTER NODES` 中 `myself,master`; 数据可读性依赖 prior Raft 复制 (storage adapter)

### 在线迁槽

1. 源 `SETSLOT MIGRATING`; 目标 `SETSLOT IMPORTING`
2. `MIGRATE` (源) → 目标 RESTORE (自动 ASKING)
3. 双方 `SETSLOT STABLE`

### 重置集群 (无 CLUSTER RESET)

AiKv 集群元数据由 **MetaRaft 共识** 维护, 非 Redis 式每节点本地 `nodes.conf`. 单节点 `CLUSTER RESET` **不支持** (ISSUE-016, doc-only): 命令返回明确 ERR, 不会清 MetaRaft / MultiRaft 状态.

**为何不支持**: 忘记节点、清空 slot、改 node ID 等需 MetaRaft propose 或删持久化目录; 运行中假 OK 比 explicit ERR 更易造成运维误判. oldmain 虽有 `cluster_reset()` 但未接入 dispatch, SOFT 为空操作.

| 场景 | 替代步骤 |
|------|----------|
| 单节点重初始化 | 停进程 → `rm -rf <data_dir>/*` → 重启 (bootstrap: `--cluster-peers` 空; join: 带 peers) |
| 全集群重搭 | 所有节点停服 → 各节点清 `data_dir` → 按 e2e 流程 MEET + ADDSLOTS (参考 `e2e/test_cluster_formation.sh`) |
| slot 冲突 / 重复分配 | MetaRaft leader 上 `CLUSTER DELSLOTS` / `FORGET`; 严重则全量清 data_dir |
| 仅清连接 ephemeral | 重启进程 (清 `importing_slots`、ASKING 等) |

```bash
# 单节点示例 (停服后)
rm -rf /var/lib/aikv/node1/*
aikv --features cluster --engine aidb --data-dir /var/lib/aikv/node1 \
  --cluster-node-id 1 --cluster-rpc-addr 127.0.0.1:5001 --bind 127.0.0.1:7001
```

### 新增 CLUSTER 子命令

1. 在 `cluster/commands.rs` 实现 handler
2. 注册 `dispatch_cluster` match arm
3. 若影响路由: 同步 `cluster_route` admin 白名单 / `is_multi_key_cmd`
4. 若写 MetaRaft: 用 `map_propose_error` 统一 NotLeader

## 测试

```bash
cargo test --features cluster cluster_ -- --test-threads=1
cargo test --features cluster -p aikv --test cluster_integration
cargo test --features cluster -p aikv --test cluster_routing
cargo test --features cluster -p aikv --test cluster_commands
cargo test --features cluster -p aikv --test cluster_creategroup

# e2e
./e2e/test_cluster_formation.sh
./e2e/test_cluster_routing.sh
./e2e/test_cluster_failover.sh
```

## 已知限制

- **非 Redis Gossip**: 无 cluster bus PING/PONG; 拓扑靠 MetaRaft + MEET (见 Gossip 节).
- **REPLICATE/FAILOVER**: 手动模型; replica 不自动服务写; 非 Redis 全自动 failover.
- **无 CLUSTER RESET**: MetaRaft 共识; 停服清 `data_dir` 重搭 — 见上方排障节.
- **无 CLUSTER METARAFT RESP 子命令**.
- **无 ADDSLOTSRANGE** (redis-cli 常用 ADDSLOTS 仍可用).
- **透明转发**: 仅 server 侧; smart client (`redis-cli -c`) 仍靠 MOVED/ASK 字符串.
- **MSETNX**: 未实现; 非 cluster 特有限制 — 见 [commands-core.md](commands-core.md).

## Gossip (轻量)

**无 Redis cluster bus PING/PONG**; 成员与 slot 权威拓扑来自 MetaRaft. `cluster/gossip.rs` 的 `start_background_refresh` 仅周期性:

- 调用 `ClusterStateManager::refresh()` 更新本地 group leader 路由缓存
- 递增 `CLUSTER INFO` 的 `cluster_stats_messages_*` (metrics 语义, 非真实 gossip 报文)

`CLUSTER NODES` **直接读 MetaRaft** (`cluster_nodes()`); ping-sent/pong-recv 恒 `0 0` (与 oldmain 一致). link-state 来自 `NodeInfo.status` (`Online`/`Draining` → `connected`, `Offline` → `disconnected`). 故障检测与成员变更以 MetaRaft/Raft 为准.
