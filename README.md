# AiKv

用 Rust 实现的 **Redis RESP 兼容 KV 网络服务** (bin + lib, 当前 **0.10.5**). 对外提供 RESP2/3 与 Redis 命令语义; 存储可选内存或 AiDb LSM 持久化; 集群与 HTTP 指标通过 Cargo feature 按需启用.

## 特性

**服务与协议** (始终可用):

- RESP2/RESP3 (HELLO 协商), Pipeline, Tokio 异步 TCP
- 数据结构: String, Hash, List, Set, Sorted Set, Key/DB 管理
- JSON 命令, Lua `EVAL`/`EVALSHA`/`SCRIPT`
- 阻塞列表/ZSet: BLPOP/BRPOP/BLMOVE, BZPOPMIN/BZPOPMAX
- 持久化运维: SAVE/BGSAVE/LASTSAVE/SHUTDOWN (aidb 路径走 `Checkpoint`, 非标准 RDB 主路径)
- 可观测: INFO, SLOWLOG, LATENCY 直方图

**存储引擎**:

| 引擎 | CLI | 说明 |
|------|-----|------|
| 内存 | `--engine memory` (默认) | 开发/测试; 重启数据丢失 |
| AiDb | `--engine aidb --data-dir <path>` | WAL + LSM; **生产与集群推荐** |

**可选能力** (feature):

| 能力 | Feature | 说明 |
|------|---------|------|
| Redis Cluster | `cluster` | MOVED/ASK, CLUSTER 子命令, slot 迁移; `aidb/cluster` |
| OTel / 探活 | `monitoring` | OTLP metrics/traces 导出, `:9191` `/health`; `aidb/monitoring` |

Feature 组合、CLI 全表与集群多节点示例见 [DEPLOYMENT.md](DEPLOYMENT.md).

## 与 AiDb

[AiDb](../aidb/) 提供同步 LSM 引擎与 MetaRaft/Multi-Raft 共识; AiKv 在其上实现 TCP、Redis 命令与 Cluster **客户端协议**. Monorepo 内 `aidb = { path = "../aidb" }`; 本地开发需 sibling 布局 `../aidb`.

## 快速开始

```bash
# 确保 ../aidb 存在
cargo build --release --features cluster
./target/release/aikv --bind 127.0.0.1:6379

# 另开终端
redis-cli -p 6379 PING
```

持久化 (生产推荐):

```bash
./target/release/aikv --bind 127.0.0.1:6379 --engine aidb --data-dir /tmp/aikv-data
```

集群部署与 `monitoring` OTLP 见 [DEPLOYMENT.md](DEPLOYMENT.md).

## 示例

内嵌 memory Server 演示 (无需单独起二进制):

```bash
cargo run --example basic
```

| 示例 | 说明 | 运行 |
|------|------|------|
| `basic` | PING/SET/GET/HSET/INFO 等 | `cargo run --example basic` |
| `cluster` | CRC16 槽位 / hash tag | `cargo run --features cluster --example cluster` |

详见 [examples/README.md](examples/README.md).

## E2E 测试

基于 `redis-cli` 的 shell smoke 测试 (需先 `cargo build --release --features cluster`):

```bash
./e2e/test_basic.sh
```

详见 [e2e/README.md](e2e/README.md).

## 文档

开发文档 hub: [docs/README.md](docs/README.md) (汇总文档 + modules WHEN 路由).

| 文档 | 内容 |
|------|------|
| [ARCHITECTURE.md](ARCHITECTURE.md) | 分层、数据流、与 AiDb 边界 |
| [DESIGN.md](DESIGN.md) | 跨模块设计决策与已知限制 |
| [DEPLOYMENT.md](DEPLOYMENT.md) | 构建、feature、CLI、集群部署、监控 |
| [AGENTS.md](AGENTS.md) | AI 助手与 CI 入口 |
| [CONTRIBUTING.md](CONTRIBUTING.md) | hooks、CI、测试矩阵、提交/PR 规范 |
| [CHANGELOG.md](CHANGELOG.md) | 版本变更记录 |
| [docs/modules/protocol.md](docs/modules/01-protocol.md) | RESP parser/encoder |
| [docs/modules/server.md](docs/modules/02-server.md) | TCP Listener/Connection |
| [docs/modules/storage.md](docs/modules/03-storage.md) | KvStorage, MemoryEngine, AiDbEngine |
| [docs/modules/commands-core.md](docs/modules/04-commands-core.md) | 核心数据结构命令, Router |
| [docs/modules/commands-extended.md](docs/modules/05-commands-extended.md) | JSON, Lua, SAVE, INFO, MIGRATE |
| [docs/modules/cluster.md](docs/modules/06-cluster.md) | MOVED/ASK, CLUSTER 子命令 |
| [docs/modules/observability.md](docs/modules/07-observability.md) | slowlog, INFO, OTLP metrics |
| [ISSUES.md](ISSUES.md) | 待核实项 |

## 已知限制

非 100% Redis 命令兼容; 无标准 `dump.rdb` / memory AOF 主路径. 详情见 [DESIGN.md](DESIGN.md) §与 Redis.

## 待核实

集群 failover/stub 子命令与可观测性默认差异 — 见 [ISSUES.md](ISSUES.md).

## 许可

[MIT](LICENSE) (见 [Cargo.toml](Cargo.toml)).
