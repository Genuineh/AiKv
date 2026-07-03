# AiKv — AI 助手指南

## 项目是什么

**AiKv** 是用 Rust 实现的 **Redis RESP 兼容 KV 服务** (bin + lib).

- **对外**: RESP2/RESP3、Redis 命令、Redis Cluster (MOVED/ASK、CLUSTER 子命令、slot 迁移等).
- **对内**: 不实现 LSM/Raft; 持久化与共识委托 **AiDb** (`Cargo.toml`: `aidb = { path = "../aidb" }`, 集群时 `cluster = ["aidb/cluster"]`).
- **存储**: `MemoryEngine` (内存) 或 `AiDbEngine` (WAL + LSM); 生产集群推荐 `--engine aidb` (当前只开发 AiDbEngine, 内存只是留作未来拓展选项).

**与 AiDb**: Redis 协议与命令在 AiKv; 数据平面与 Multi-Raft 在 AiDb. 本地需 sibling 布局 `../aidb`; CI checkout 同名分支 AiDb 并 link.

## 架构要点

- **客户端兼容**: 标准 RESP 数组; BulkString 二进制安全 (不做 UTF-8 假设).
- **Cluster**: 16384 槽、CRC16、`{hash tag}`、MOVED/ASK、在线 slot 迁移; 与 Redis Cluster 客户端协作.
- **分层**: `protocol` → `server` → `command` → `storage` (memory | AiDb) → `cluster` 协议层 → AiDb MetaRaft / MultiRaft.
- **数据流**:

```text
Redis Client → TCP/RESP/CommandRouter/ClusterRouter
            → KvStorage (MemoryEngine | AiDbEngine)
            → aidb (DB | MetaRaft | MultiRaft | Router)
```

- 存在性 / 删除等语义须走 AiDb `DB::get` 与 tombstone 规则, 不在 storage adapter 绕过.
- 同一 codebase; 开发与 CI 以 `--features cluster` 为主.

## 技术栈与参考

### Redis 8.8 参考标准

排查 AiKv 行为、INFO/可观测性、Cluster 客户端语义时, **以 Redis Open Source 8.8 为对照基准** (非 7.2 旧口径).

| 用途 | 说明 |
|------|------|
| **兼容声明** | INFO `redis_compatible_version:8.8` (目标); `redis_version` 仍为 AiKv 真实版本 |
| **集群** | MOVED/ASK 由 **客户端** 处理 (`redis-cli -c`); **无** 服务端透明转发; 命令统计仅在实际执行节点 |
| **INFO** | `INFO default` / `all` / `everything` / 多 section; 8.8 键名 parity (stub `0`/`-1`); commandstats 八字段; errorstats 错误前缀 |
| **监控镜像** | 生产指标经 OTLP `aikv_*`; 语义对齐 **redis_exporter 解析 INFO** (非引入 `redis_*` 命名) |

字段 stub vs 真源说明: [docs/modules/observability.md](docs/modules/observability.md) §INFO 渲染 · [observability-reference.md](docs/modules/observability-reference.md) §Stub 字段策略

**官方文档:** [Redis 8.8 Commands](https://redis.io/docs/latest/commands/) · [INFO](https://redis.io/docs/latest/commands/info/) · [Redis Cluster spec](https://redis.io/docs/latest/operate/oss_and_stack/reference/cluster-spec/)

**本项目 spec / 设计:**

- [docs/superpowers/specs/2026-06-25-redis-alignment-cluster-info-otel-design.md](docs/superpowers/specs/2026-06-25-redis-alignment-cluster-info-otel-design.md) — 集群路由 + INFO 8.8 + OTel 对齐 (**P0–P3 已落地**, INFO 字段级 parity + stub 策略见 observability 文档)
- [DESIGN.md](DESIGN.md) — 与 Redis 8.8 一致的取舍 (MOVED、INFO 真源、OTLP)
- [docs/modules/observability-reference.md](docs/modules/observability-reference.md) — INFO ↔ `aikv_*` ↔ redis_exporter 三列对照 (P2 已补全)

**对比排查建议:** 同场景下并排 `redis-cli INFO` / `INFO commandstats` 与 AiKv; 集群用 `redis-cli -c`; 指标对照 INFO 字段而非臆测 PromQL.

| 层级 / 领域 | 本项目 | 可参考 (设计/实现, 非直接依赖) |
|-------------|--------|--------------------------------|
| **客户端 RESP** | 自研 `src/protocol/` (RESP2/3, pipeline, 安全边界); **未** 使用 `resp-rs` crate | [Redis RESP 规范](https://redis.io/docs/reference/protocol-spec/); `resp-rs` 的零拷贝/API 设计 |
| **Redis 类型 → KV** | `StoredValue` / `ValueType` 序列化后写入 AiDb 扁平 key (`{db_index}:{user_key}`) | Kvrocks、Pika/BlackWidow 的编码与兼容性思路 |
| **Cluster 协议** | MOVED/ASK、16384 slot、CLUSTER 子命令、slot 迁移状态机 | **Redis 8.8** Cluster 行为与客户端约定 (见上节) |
| **Gossip** | **轻量拓扑 tick**: 刷新 leader 路由缓存 + gossip metrics; **NODES 读 MetaRaft**; 故障判定走 MetaRaft, 非 Serf 级 SWIM | Redis Cluster 的 gossip 语义; 完整 Serf/Cassandra 式 gossip 未引入 |
| **共识 / RPC / LSM** | 委托 **AiDb** (OpenRaft + tonic + 自研 LSM) | 见 AiDb `AGENTS.md` |

协议栈: **RESP (aikv) → 命令/存储适配 → gRPC+Raft+LSM (aidb)**.

## 开发与 CI

流程见 [`.github/README.md`](.github/README.md).

```bash
# 确保 ../aidb 存在
./install-hooks.sh
export RUSTFLAGS='-D warnings'
cargo fmt --check
cargo clippy --all-targets --features cluster
cargo test --workspace --features cluster -- --test-threads=1
```

慢测 (CI: `test-server-stress`, `test-commands-slow`):

```bash
cargo test --test server --features cluster -- --ignored --test-threads=1
cargo test --test commands --features cluster -- --ignored --test-threads=1
```

修 bug **必带** 回归测: 见 [CONTRIBUTING.md §回归测试](CONTRIBUTING.md#回归测试-必带).

## Span 级别约定

**入口 span** (`kv_command`, `cmd_string`) 级别为 **`level = "debug"`**. 原因与 AiDb 相同: OTLP 开启时每个 span 被 `tracing_opentelemetry` layer 转为 OTel span, per-span 生命周期回调累积开销. 生产 `RUST_LOG=info` 不创建入口 span, 需调试时设 `RUST_LOG=debug`.

**`batcher_batch_done`** (target: `perf`) 用于测量批次延迟, 生产不启用 (`RUST_LOG=info` 不含 `perf=info`).

## Batcher 调优常量 (`cluster_adapter.rs`)

| 常量/字段 | 值 | 说明 |
|-----------|----|------|
| `SET_BATCH_MAX_OPS` | 128 | 批上限 |
| `SET_BATCH_MAX_DELAY` | 1ms | 凑批等待上限 |
| `ClusterDataAdapter::eager_flush` | 12 (默认) | 已达该数则不等 timeout, 立即 propose; 构造时传入 |

调整 `eager_flush` 需权衡吞吐与延迟: 过小则 Raft per-batch 开销摊薄不够, 过大则增加尾部延迟. 当前默认值 12 在 50c 集群下平衡.

## 已知限制

**集群**

- `CLUSTER FORGET` / `DEL_REPLICA` 等行为见 CHANGELOG; FORCE 路径有 guard.
- 数据面 gRPC: `rpc_port + --cluster-data-port-offset` (默认 10000).

**持久化 (未做或延后)**

- memory 引擎 AOF; 标准 `dump.rdb` 暂不实现持久化; `CONFIG REWRITE` — 生产推荐 `--engine aidb`.

**协议**

- 不支持 telnet 式非数组内联命令 (`PING\r\n`).

## 进一步阅读

- [README.md](README.md)
- [ARCHITECTURE.md](ARCHITECTURE.md)
- [DESIGN.md](DESIGN.md)
- [docs/superpowers/specs/2026-06-25-redis-alignment-cluster-info-otel-design.md](docs/superpowers/specs/2026-06-25-redis-alignment-cluster-info-otel-design.md)
- [docs/README.md](docs/README.md) — 按域 WHEN 与 modules 导航
- [.github/README.md](.github/README.md)

