# AiKv 项目指南

## 概述

Redis RESP 协议兼容的 KV 服务 (Rust bin crate, **0.9.3**). Phase 8 RESP/TCP; Phase 9 MemoryEngine + 基础命令; Phase 10 扩展类型 + AiDb; Phase 11 JSON+Lua; Phase 16 集群协议.

## 架构

- **协议** ✅: RESP2 + RESP3 解析器/编码器, tokio TCP (`src/protocol/`, `src/server/`)
- **命令** ✅: `CommandRouter` — String/Hash/List/Set/ZSet/Key/Server + **JSON** + **Lua** + **阻塞命令** (BLPOP/BRPOP/BLMOVE/BZPOPMIN/BZPOPMAX); 内联 PING/ECHO/HELLO/QUIT 仍在 Connection
- **存储** ✅: `MemoryEngine` + `AiDbEngine` (`src/storage/`)
- **集群** ✅: Phase 16 集群协议 (`src/cluster/`) — CLUSTER INFO/NODES/SLOTS/… 命令, MOVED/ASK 路由, ClusterConnectionState per-connection 状态, GossipState 轻量拓扑缓存. 底层委托 aidb MetaRaft/Multi-Raft.
- **持久化**: SAVE/BGSAVE/LASTSAVE/SHUTDOWN ✅; AOF + 标准 RDB ❌ (AiDb 引擎 WAL+Checkpoint 已覆盖持久化需求)

## 命令

```bash
cargo build
cargo test -- --test-threads=1
cargo test --test storage --test commands -- --test-threads=1
cargo test --test resp --test server -- --test-threads=1
cargo clippy -- -D warnings
cargo fmt --check
cargo run -- --bind 127.0.0.1:6379   # redis-cli SET/GET/HSET/SELECT
```

## 约束

1. 生产代码中禁止 `unwrap()`/`expect()` (测试代码除外)
2. 单文件上限 800 行, 单函数上限 50 行, 嵌套上限 4 层
3. TDD: 先写测试 (RED → GREEN → IMPROVE)
4. 二进制安全: 所有 BulkString 编码跳过 UTF-8 校验
5. E2E 测试在 `e2e/` 目录, 通过 shell 脚本 + redis-cli 运行 (Phase 10+)

## 测试

```bash
cargo test --features cluster   # 集群测试 (40 个: skeleton/routing/commands/integration)
cargo test --features cluster -- --test-threads=1  # 避免 tracing 并行竞争

# E2E 测试
cd e2e && for t in test_cluster_*.sh; do bash "$t"; done   # 集群 E2E (6 个脚本)
bash e2e/test_cluster_formation.sh   # 单个集群测试
```

## Known Limitations

以下功能在 M1-M4 阶段已明确决定延后或不做:

### 集群 (known_limitation)
- **cluster_forget**: 默认 graceful; 可选 `CLUSTER FORGET <id> FORCE` 强制摘除 (需节点不在唯一副本 group 中).
- **cluster_del_replica**: `CLUSTER DEL_REPLICA <primary_id> <replica_id>` 先从 data group 移除副本, 再 `FORGET`.
- **数据面 gRPC 端口**: 默认为 `rpc_port + 10000`, 可通过 `--cluster-data-port-offset` CLI 参数配置. 启动时校验 `rpc_port ≤ 65535 - offset`.

### 持久化 (延后)
- **AOF (memory 引擎)**: Phase 11.5, 生产路径使用 `--engine aidb` 无需 AOF
- **标准 RDB dump.rdb**: Phase 11.6, 有迁移需求时再做
- **CONFIG REWRITE**: 未实现, 集群配置手动管理

### 功能 (不做)
- **内联命令非数组格式** (`PING\r\n` 用于 telnet/nc): 标准客户端使用 RESP 数组格式

## 设计文档

详见 `/docs/aikv-inventory/` 下各模块设计规格。
