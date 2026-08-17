# AiKv 开发文档

`docs/` 域的开发文档导航. 项目介绍与快速开始见 [README.md](../README.md).

---

## 阅读路径

- **首次了解** — [README.md](../README.md) → [ARCHITECTURE.md](../ARCHITECTURE.md) → 按需查阅下方模块文档
- **改某域代码** — 查阅 [按域阅读](#按域阅读-modules) → 对应模块文档; 跨域边界见各文档内「不覆盖」部分
- **构建 / 部署 / 运维** — [deployment.md](deployment.md) + [CONTRIBUTING.md](../CONTRIBUTING.md); AI 助手速查见 [AGENTS.md](../AGENTS.md)
- **底层 LSM 存储与 Raft 内核** — 参见 sibling 仓库 [AiDb 开发文档](../../aidb/docs/README.md)

---

## 汇总文档

| 文档 | 核心内容 |
| :--- | :--- |
| [ARCHITECTURE.md](../ARCHITECTURE.md) | 系统分层、KeyLock 并发模型、Subkey 编码与数据流总览 |
| [design.md](design.md) | 跨模块设计决策与技术权衡 (Why)、YAGNI 刻意不实现与决策总表 |
| [deployment.md](deployment.md) | Cargo Features 构建、CLI 参数、单机/集群部署、OTel 监控告警 |
| [CONTRIBUTING.md](../CONTRIBUTING.md) | Git Hooks、测试矩阵、规范要求与 PR 提交流程 |
| [CHANGELOG.md](../CHANGELOG.md) | 历史版本演进与发布变更记录 |
| [AGENTS.md](../AGENTS.md) | AI 助手工作纪律、硬性约束 (Never / Always) 与技术对照表 |

---

## 按域阅读 (modules)

| 模块文档 | 映射源码目录 | 何时查阅与核心职责 |
| :--- | :--- | :--- |
| [01-protocol.md](modules/01-protocol.md) | `src/protocol/` | 改 RESP2/3 解析与编码、缓冲区管理、解析 Limits 与错误恢复 |
| [02-server.md](modules/02-server.md) | `src/server/` | 改 TCP 连接管理、Listener、`HELLO` 协商、ATOM 事务与 `max_clients` |
| [03-storage.md](modules/03-storage.md) | `src/storage/` | 改 `KvStorage` 接口、Memory/AiDb 引擎适配、Subkey 编码与 Raft 批处理 |
| [04-commands-core.md](modules/04-commands-core.md) | `src/command/` | 改 String~ZSet 核心数据结构命令、`CommandRouter` 分发与 `KeyLock` 并发加锁 |
| [05-commands-extended.md](modules/05-commands-extended.md) | `src/command/` | 改 JSON/Lua 引擎、`BlockingRegistry` (BLPOP)、MIGRATE 与持久化管理命令 |
| [06-cluster.md](modules/06-cluster.md) | `src/cluster/` | 改 Redis Cluster 协议、16384 槽位 CRC16、MOVED/ASK 重定向与拓扑同步 |
| [07-observability.md](modules/07-observability.md) | `src/server/` | 改 Slowlog、Latency、`InfoRenderer`、OTel 生产指标与 Tracing 架构 |
| [08-observability-reference.md](modules/08-observability-reference.md) | `src/server/` | 全量 `aikv_*` OTel 指标字典与 Redis 8.8 INFO 字段对齐参考表 |

> **依赖顺序**: `protocol` → `server` → `storage` → `commands-core` → `commands-extended`; `cluster` 依赖 `storage` 与 `aidb/cluster`; `observability` 横切各层.

---

## 构建与测试

构建、Cargo feature 与完整测试矩阵见 [deployment.md](deployment.md) 与 [CONTRIBUTING.md](../CONTRIBUTING.md).
