# AiKv — AI 助手指南

## 项目简介

**AiKv** 是 Rust **Redis RESP 兼容 KV 服务** (bin + lib).

- **对外**: RESP2/RESP3、Redis 命令、Redis Cluster (MOVED/ASK、CLUSTER、slot 迁移)
- **对内**: 不实现 LSM/Raft; 持久化与共识委托 **AiDb**
- **存储**: 生产推荐 `--engine aidb` (`AiDbEngine`); `MemoryEngine` 当前仅作扩展预留, 不开发相关功能.

```text
Redis Client → TCP/RESP → CommandRouter/ClusterRouter
            → KvStorage (MemoryEngine | AiDbEngine)
            → aidb (DB | MetaRaft | MultiRaft)
```

## 技术栈与参考

仓库专业术语见 [CONTEXT.md](CONTEXT.md).

**验收基准**: Redis Open Source **8.8** (非 7.2). 协议栈: **RESP (aikv) → 命令/存储适配 → gRPC+Raft+LSM (aidb)**.

| 层 | 本项目选型 | 主流实现参考 | 取舍 / 差异 |
|----|-----------|-------------|------------|
| **RESP** | 自研 `src/protocol/` (RESP2/3, pipeline); **未** 用 `resp-rs` | [Redis RESP 规范](https://redis.io/docs/reference/protocol-spec/) · [Redis 8.8 Commands](https://redis.io/docs/latest/commands/) | 不支持 telnet 式非数组内联命令 (`PING\r\n`) |
| **类型 → KV** | `StoredValue` / `ValueType` 序列化后写入 AiDb 扁平 key (`{db_index}:{user_key}`) | [Kvrocks](https://github.com/apache/kvrocks) 的 Redis-on-LSM 编码思路 | 存储引擎是 AiDb 自研 LSM, 非 Kvrocks/RocksDB 直用 |
| **Cluster** | MOVED/ASK、16384 slot、CLUSTER 子命令、slot 迁移状态机 | [Redis 8.8 Cluster spec](https://redis.io/docs/latest/operate/oss_and_stack/reference/cluster-spec/) · `redis-cli -c` 客户端行为 | **无** 服务端透明转发; 命令统计仅在实际执行节点 |
| **Gossip** | 轻量拓扑 tick: 刷新 leader 路由缓存 + gossip metrics; NODES 读 MetaRaft | Redis Cluster gossip 的**语义标签** (非实现) | 故障判定走 MetaRaft, 非 SWIM/Serf; 不引入完整 Cassandra 式 gossip |
| **INFO / 监控** | INFO 8.8 键名 parity (部分 stub `0`/`-1`); commandstats 八字段; 生产经 OTLP `aikv_*` | [Redis INFO](https://redis.io/docs/latest/commands/info/) · [observability-reference.md](docs/modules/08-observability-reference.md) | `redis_version` 为 AiKv 真实版本; 指标语义对齐 redis_exporter 解析 INFO, **不**引入 `redis_*` Prom 命名 |
| **共识 / LSM** | 委托 **AiDb** (OpenRaft + tonic + 自研 LSM) | [../aidb/AGENTS.md](../aidb/AGENTS.md) | 不在本仓实现 Raft / Compaction / WAL |

> **决策说明**: 上述选型仅为参考, 先由 AI 列出候选方案优劣, 再供开发者决策.

## 本仓硬约束

- **Cluster**: MOVED/ASK 由客户端处理 (`redis-cli -c`); 服务端不做透明转发
- **存在性 / 删除**: 须走 AiDb `DB::get` 与 tombstone 规则, 不在 storage adapter 绕过
- **Span**: 入口 span (`kv_command`, `cmd_string`) 用 `level = "debug"`; `batcher_batch_done` (target `perf`) 生产默认不启用
- **Batcher** (`cluster_adapter.rs`): `SET_BATCH_MAX_OPS=512`, `SET_BATCH_MAX_DELAY=1ms`, `DEFAULT_EAGER_FLUSH=48`; 调 `eager_flush` 须权衡吞吐与尾延迟, 经 `ClusterDataAdapter::DEFAULT_EAGER_FLUSH` 引用
- **指标**: 生产经 OTLP `aikv_*`; INFO 目标对齐 Redis 8.8 (部分字段 stub)
- **测试纪律**: 修 bug 必带回归测 ([CONTRIBUTING.md §回归测试](CONTRIBUTING.md#回归测试-必带)); 新测写法与落点 ([tests/README.md §测试写法与范围 (硬性)](tests/README.md#测试写法与范围-硬性)); 验证默认 `--features cluster`, `RUSTFLAGS='-D warnings'` + `--test-threads=1` (aidb 依赖 git + 本地 patch, 见 [CONTRIBUTING.md](CONTRIBUTING.md))
- **文档同步 (强制)**: 改公共 API / 行为 / 模块边界必须同步对应 `docs/modules/*.md` 与根文档; commit 消息修 bug 须带 GitHub Issue 引用 (`Fixes #NN`); 不满足不进 commit (见 [CONTRIBUTING.md §文档同步](CONTRIBUTING.md#文档同步-硬性))

## 进一步阅读

- [ARCHITECTURE.md](ARCHITECTURE.md) · [DESIGN.md](DESIGN.md) · [docs/README.md](docs/README.md)
- [docs/modules/06-cluster.md](docs/modules/06-cluster.md) · [docs/modules/07-observability.md](docs/modules/07-observability.md) · [docs/modules/08-observability-reference.md](docs/modules/08-observability-reference.md)
- [CONTRIBUTING.md](CONTRIBUTING.md) · `[.github/README.md](.github/README.md)`
- [../aidb/AGENTS.md](../aidb/AGENTS.md) — LSM / Raft 入口; 排查 **LSM/Raft/Compaction** → 此处
