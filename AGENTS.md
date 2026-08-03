# AiKv — AI 助手指南

## 进仓必读

1. 先读 [CONTEXT.md](CONTEXT.md) — 领域术语以此为准, 不发明同义词
2. 通用沟通 / 代码原则 / Git / 文档同步由 **vibe-coding** 全局 Rules 提供 (`~/.cursor/rules/`), 本文件不重复
3. 开发流程按下方「工作流」走; Skills 来自 vibe-coding (`code-review` / `grilling` / `plan-review` / `handoff`)
4. 本地需 sibling: `../aidb` (`Cargo.toml` path 依赖; 集群时 `cluster = ["aidb/cluster"]`)

## 项目是什么

**AiKv** 是 Rust **Redis RESP 兼容 KV 服务** (bin + lib).

- **对外**: RESP2/RESP3、Redis 命令、Redis Cluster (MOVED/ASK、CLUSTER、slot 迁移)
- **对内**: 不实现 LSM/Raft; 持久化与共识委托 **AiDb**
- **存储**: 生产推荐 `--engine aidb` (`AiDbEngine`); `MemoryEngine` 仅开发/拓展预留

```text
Redis Client → TCP/RESP → CommandRouter/ClusterRouter
            → KvStorage (MemoryEngine | AiDbEngine)
            → aidb (DB | MetaRaft | MultiRaft)
```

排查 **Redis 兼容性** (命令、MOVED、INFO、commandstats) → 本仓库; 排查 **LSM/Raft/Compaction** → [../aidb/AGENTS.md](../aidb/AGENTS.md).

## 技术栈与参考

拿不准时: 先查「权威对照」, 再对照「刻意差异」; 术语见 [CONTEXT.md](CONTEXT.md).

**验收基准**: Redis Open Source **8.8** (非 7.2). 协议栈: **RESP (aikv) → 命令/存储适配 → gRPC+Raft+LSM (aidb)**.

| 层 | 本项目选型 | 权威对照 (拿不准时查) | 刻意差异 (勿照抄) |
|----|------------|----------------------|-------------------|
| **RESP** | 自研 `src/protocol/` (RESP2/3, pipeline); **未** 用 `resp-rs` | [Redis RESP 规范](https://redis.io/docs/reference/protocol-spec/) · [Redis 8.8 Commands](https://redis.io/docs/latest/commands/) | 不支持 telnet 式非数组内联命令 (`PING\r\n`) |
| **类型 → KV** | `StoredValue` / `ValueType` 序列化后写入 AiDb 扁平 key (`{db_index}:{user_key}`) | [Kvrocks](https://github.com/apache/kvrocks) 的 Redis-on-LSM 编码思路 | 存储引擎是 AiDb 自研 LSM, 非 Kvrocks/RocksDB 直用 |
| **Cluster** | MOVED/ASK、16384 slot、CLUSTER 子命令、slot 迁移状态机 | [Redis 8.8 Cluster spec](https://redis.io/docs/latest/operate/oss_and_stack/reference/cluster-spec/) · `redis-cli -c` 客户端行为 | **无** 服务端透明转发; 命令统计仅在实际执行节点 |
| **Gossip** | 轻量拓扑 tick: 刷新 leader 路由缓存 + gossip metrics; NODES 读 MetaRaft | Redis Cluster gossip 的**语义标签** (非实现) | 故障判定走 MetaRaft, 非 SWIM/Serf; 不引入完整 Cassandra 式 gossip |
| **INFO / 监控** | INFO 8.8 键名 parity (部分 stub `0`/`-1`); commandstats 八字段; 生产经 OTLP `aikv_*` | [Redis INFO](https://redis.io/docs/latest/commands/info/) · [observability-reference.md](docs/modules/08-observability-reference.md) | `redis_version` 为 AiKv 真实版本; 指标语义对齐 redis_exporter 解析 INFO, **不**引入 `redis_*` Prom 命名 |
| **共识 / LSM** | 委托 **AiDb** (OpenRaft + tonic + 自研 LSM) | [../aidb/AGENTS.md](../aidb/AGENTS.md) | 不在本仓实现 Raft / Compaction / WAL |

字段 stub vs 真源: [docs/modules/observability.md](docs/modules/07-observability.md).

## 工作流 (vibe-coding)

按任务类型选入口 (详见全局 rule `workflows-routing`):

| 类型 | 流程 |
|------|------|
| **新功能 / 大改** (多文件、新模块、架构) | brainstorming → writing-plans → **plan-review** → 开分支 → implement → **code-review** → **documentation-sync** → 用户确认后 commit |
| **小改 / bug** (已知问题、单点) | **grilling** → 开分支 → implement → **code-review** → **documentation-sync** → 用户确认后 commit |

补充约定:

- plan / spec 是工作区根 `superpower/` 下的**过程制品**, 不进本仓、**也不从本仓文档引用**; 对仓库仍有效的结论须写入本仓 `docs/` / DESIGN / ARCHITECTURE (见 `documentation-sync`)
- 开分支: 共识之后、改代码之前从原分支拉新分支; 纯文档微调或用户要求就地改时可跳过 (先问); 计划完成后经允许再 squash 回原分支
- **code-review** 通过后做 **documentation-sync**, 再请用户确认; 只在用户明确要求时 commit; 不推远程
- 会话切换用 **handoff** → 写工作区根 `CHAT.md`
- 压测 / 部署在 **aifactory** (`../aifactory`): 灵活部署 `scripts/up-*.sh`, 对照压测 `benchmark/` (AiKv compose 在 `benchmark/aikv/`); 本仓专注协议与服务逻辑

不确定大改还是小改时, 先问用户.

## 本仓硬约束

- **Cluster**: MOVED/ASK 由客户端处理 (`redis-cli -c`); 服务端不做透明转发
- **存在性 / 删除**: 须走 AiDb `DB::get` 与 tombstone 规则, 不在 storage adapter 绕过
- **Span**: 入口 span (`kv_command`, `cmd_string`) 用 `level = "debug"`; `batcher_batch_done` (target `perf`) 生产默认不启用
- **Batcher** (`cluster_adapter.rs`): `SET_BATCH_MAX_OPS=512`, `SET_BATCH_MAX_DELAY=1ms`, `DEFAULT_EAGER_FLUSH=48`; 调 `eager_flush` 须权衡吞吐与尾延迟, 经 `ClusterDataAdapter::DEFAULT_EAGER_FLUSH` 引用
- **指标**: 生产经 OTLP `aikv_*`; INFO 目标对齐 Redis 8.8 (部分字段 stub)
- 修 bug **必带** 回归测: [CONTRIBUTING.md §回归测试](CONTRIBUTING.md#回归测试-必带)
- 新测写法与落点 (硬性): [tests/README.md §测试写法与范围 (硬性)](tests/README.md#测试写法与范围-硬性)
- 不支持 telnet 式非数组内联命令 (`PING\r\n`)
- 验证: 需 sibling `../aidb`; 默认带 `--features cluster`; `RUSTFLAGS='-D warnings'`; 测试加 `--test-threads=1`; 完整命令见 [CONTRIBUTING.md](CONTRIBUTING.md)

## 进一步阅读

- [ARCHITECTURE.md](ARCHITECTURE.md) · [DESIGN.md](DESIGN.md) · [docs/README.md](docs/README.md)
- [docs/modules/cluster.md](docs/modules/06-cluster.md) · [docs/modules/observability.md](docs/modules/07-observability.md) · [docs/modules/observability-reference.md](docs/modules/08-observability-reference.md)
- [CONTRIBUTING.md](CONTRIBUTING.md) · [`.github/README.md`](.github/README.md)
- [../aidb/AGENTS.md](../aidb/AGENTS.md) — LSM / Raft 入口
