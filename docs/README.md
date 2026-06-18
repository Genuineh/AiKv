# AiKv 开发文档

`docs/` 域的开发文档导航. 项目介绍与快速开始见 [README.md](../README.md).

## 阅读路径

- **首次了解** — [README.md](../README.md) → [ARCHITECTURE.md](../ARCHITECTURE.md) → 按需打开下方 modules
- **改某域代码** — 查 [按域阅读](#按域阅读-modules) WHEN → 对应 module; 跨域边界见 module 内「不覆盖」
- **构建 / 测试 / PR** — [DEPLOYMENT.md](../DEPLOYMENT.md) + [CONTRIBUTING.md](../CONTRIBUTING.md); AI 助手速览见 [AGENTS.md](../AGENTS.md)
- **底层 LSM / Raft** — 存储与共识在 sibling [AiDb](../../aidb/docs/README.md); AiKv 侧重 RESP 与命令层

## 汇总文档

| 文档 | 内容 |
|------|------|
| [ARCHITECTURE.md](../ARCHITECTURE.md) | 分层、数据流、与 AiDb 边界 |
| [DESIGN.md](../DESIGN.md) | 跨模块设计决策与已知限制 |
| [DEPLOYMENT.md](../DEPLOYMENT.md) | 构建、feature、CLI、集群部署、监控 |
| [CONTRIBUTING.md](../CONTRIBUTING.md) | hooks、CI、测试矩阵、提交/PR 规范 |
| [CHANGELOG.md](../CHANGELOG.md) | 版本变更记录 |
| [AGENTS.md](../AGENTS.md) | AI 助手与 CI 入口 |
| [ISSUES.md](../ISSUES.md) | 待核实与已知疑点 |

## 按域阅读 (modules)

| Module | 何时读 |
|--------|--------|
| [protocol.md](modules/protocol.md) | 改 `protocol/*`; RESP2/3 parse/encode、buffer/深度限制、ProtocolVersion |
| [server.md](modules/server.md) | 改 `server/{listener,connection,config}`; TCP 读写/pipeline、HELLO、ATOM 事务、`max_clients` |
| [storage.md](modules/storage.md) | 改 `storage/*`; KvStorage、MemoryEngine/AiDbEngine、TTL/StoredValue、集群数据面 Raft 写 |
| [commands-core.md](modules/commands-core.md) | 改核心数据结构命令与 Router; String~ZSet/Key/DB、WRONGTYPE、KeyLock、CROSSSLOT 前路由 |
| [commands-extended.md](modules/commands-extended.md) | 改 JSON/Lua/阻塞/MIGRATE/SAVE/INFO/CONFIG 等扩展命令与 router extended dispatch |
| [cluster.md](modules/cluster.md) | 改 `cluster/*`、`init_cluster`; MOVED/ASK、CLUSTER 子命令、slot 迁移/failover (`cluster` feature) |
| [observability.md](modules/observability.md) | 改 slowlog/latency/info/metrics; INFO/SLOWLOG/LATENCY、`/metrics` (`monitoring`); 指标表 → [observability-reference.md](modules/observability-reference.md) |

依赖顺序: protocol → server → storage → commands-core → commands-extended; cluster 依赖 storage + aidb cluster; observability 横切 (INFO 命令 dispatch 仍见 commands-extended).

## 构建与测试

构建、Cargo feature 与完整测试矩阵见 [DEPLOYMENT.md](../DEPLOYMENT.md) 与 [CONTRIBUTING.md](../CONTRIBUTING.md).

## 待核实

详情见 [ISSUES.md](../ISSUES.md) (module 内一行引用, 不在此展开).
