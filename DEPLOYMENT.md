# AiKv 部署与运行

本文说明 **如何构建、启用 Cargo feature、启动单机/集群、CLI/端口/环境变量、监控 OTLP 与持久化运维**. **是什么、怎么分层** 见 [ARCHITECTURE.md](ARCHITECTURE.md); **设计取舍** 见 [DESIGN.md](DESIGN.md); 命令语义与域实现见 [docs/modules/](docs/modules/).

AiDb 侧 protoc、LSM 数据目录、库侧 cluster API 见 [aidb/DEPLOYMENT.md](../aidb/DEPLOYMENT.md) — 本文 **不重复** LSM/Raft 运维细节.

## 文档分工

| 文档 | 回答 |
|------|------|
| [ARCHITECTURE.md](ARCHITECTURE.md) | 分层、数据流、端口在架构中的角色 |
| [DESIGN.md](DESIGN.md) | 为何 feature gate、为何无 RDB 主路径等 |
| **DEPLOYMENT.md (本篇)** | 构建、运行、CLI、端口、集群部署、监控 OTLP |
| [CONTRIBUTING.md](CONTRIBUTING.md) | CI/hook/贡献流程细节 |
| [docs/modules/](docs/modules/) | 域级实现 |

## 系统要求

| 项 | 要求 |
|----|------|
| Rust | **stable** (见 [rust-toolchain.toml](rust-toolchain.toml); 含 clippy、rustfmt) |
| 操作系统 | Linux / macOS (CI 为 `ubuntu-latest`) |
| Monorepo | sibling 布局 `../aidb` (path 依赖); CI checkout 同名分支 AiDb 并 `ln -sf` |
| 磁盘 | 持久化 (`--engine aidb`) 推荐 SSD; 容量随数据量 |
| 内存 | `memory` 引擎全量驻内存; 生产集群推荐 `--engine aidb` |
| protoc | **`cluster` feature** 本地 clippy/测试需要; 见 [aidb DEPLOYMENT §构建与验证](../aidb/DEPLOYMENT.md#构建与验证) |
| 可选 | `redis-cli` — 运行 [e2e/](e2e/) smoke 与集群运维 |

## Cargo feature 矩阵

定义见 [Cargo.toml](Cargo.toml).

| Feature | 默认 | 启用内容 | 典型用途 |
|---------|------|----------|----------|
| (none) | ✅ | 单机 RESP; 无 `init_cluster`; 无 HTTP `/metrics` | 最小二进制 |
| `cluster` | ❌ | `src/cluster/*`, `storage/cluster_adapter`, `main::init_cluster`; `aidb/cluster` | 开发与 CI 主路径 |
| `monitoring` | ❌ | `MetricsServer` (/health), OTel metrics/traces; `aidb/monitoring` | 生产 OTLP + 探活 |

**常见组合**:

| 场景 | 命令 |
|------|------|
| 开发 / CI (与 `.github/workflows/ci.yml` 一致) | `cargo build --release --features cluster` |
| 生产 + OTLP | `cargo build --release --features cluster,monitoring` |
| 仅验证 monitoring 本地 | `cargo build --features cluster,monitoring` |

> **注意**: `cargo build --release` **不含** cluster/monitoring. `--metrics-port` 等 CLI 始终可解析; `[monitoring]` 下 `:9191` 仅 `/health`, **无** `/metrics`.

## 构建与验证

完整流程见 [AGENTS.md](AGENTS.md)、[.github/README.md](.github/README.md).

```bash
# 确保 ../aidb 存在 (或 CI 等价 link)
./install-hooks.sh   # 可选; pre-commit: fmt + clippy

export RUSTFLAGS='-D warnings'
cargo fmt --check
cargo clippy --all-targets --features cluster
cargo test --workspace --features cluster -- --test-threads=1
```

慢测 (CI 独立 job):

```bash
cargo test --test server --features cluster -- --ignored --test-threads=1
cargo test --test commands --features cluster -- --ignored --test-threads=1
```

E2E (需 `redis-cli`; CI job `e2e`):

```bash
cargo build --release --features cluster
chmod +x e2e/*.sh
./e2e/test_cluster_formation.sh
```

见 [e2e/README.md](e2e/README.md). **`monitoring` 无独立 CI job** — 本地可 `cargo build --features cluster,monitoring`.

## Monorepo 布局

```shell
database/
├── aidb/          # LSM + 可选 MetaRaft/MultiRaft (path 依赖)
└── aikv/          # 本仓库 — RESP bin
    ├── src/main.rs
    └── Cargo.toml # aidb = { path = "../aidb" }
```

## 命令行参数

权威定义: [`src/main.rs`](src/main.rs) `Args`. 监听地址为 **单一** `--bind host:port` (无独立 `--port`).

| 参数 | 默认 | Feature | 说明 |
|------|------|---------|------|
| `--bind` | `127.0.0.1:6379` | 始终 | RESP 客户端 TCP |
| `--engine` | `memory` | 始终 | `memory` \| `aidb` |
| `--data-dir` | — | 始终 | **`aidb` 必填**; **cluster 必填** |
| `--sync-wal` | `false` | `aidb` / cluster | 每条写后 fsync WAL (强持久, 低吞吐) |
| `--backup-dir` | `{data_dir}/backup` | 可选 | SAVE/BGSAVE checkpoint 目标 |
| `--cluster-node-id` | — | `cluster` | u64 节点 ID |
| `--cluster-rpc-addr` | — | `cluster` | MetaRaft gRPC `host:port` |
| `--cluster-peers` | `[]` | `cluster` | 已有节点 RPC 列表 (逗号分隔); 空 = bootstrap |
| `--raft-election-timeout-min` | `1000` | `cluster` | ms |
| `--raft-election-timeout-max` | `2000` | `cluster` | ms |
| `--raft-rpc-timeout-ms` | `500` | `cluster` | 须 < `election_timeout_min` |
| `--raft-heartbeat-interval` | `300` | `cluster` | 须 < `election_timeout_min` |
| `--lifecycle-tick-ms` | `1000` | `cluster` | LifecycleManager tick |
| `--gossip-interval` | `1` | `cluster` | 秒; 拓扑 tick (leader 缓存 + gossip metrics) |
| `--config-auto-save-ms` | `2000` | `cluster` | 集群配置自动保存 |
| `--cluster-data-port-offset` | `10000` | `cluster` | `data_port = rpc_port + offset`; 全集群一致 |
| `--metrics-port` | `9191` | CLI 始终 | HTTP 仅 `[monitoring]` |
| `--metrics-addr` | `127.0.0.1` | CLI 始终 | 同上 |
| `--max-clients` | `10000` | 始终 | `0` = 不限制 |

> **集群门控**: 仅当 **`--cluster-node-id` 与 `--cluster-rpc-addr` 同时提供** 时执行 `init_cluster`. 只设其一则 **静默以单机启动** (无集群).

## 环境变量

| 变量 | 默认 / 条件 | 说明 |
|------|-------------|------|
| `RUST_LOG` | 默认 directive `info` | tracing 过滤 (`EnvFilter`) |
| `AIKV_JSON_LOG` | 默认 `true` | `true` → JSON 日志; `false` → compact |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | 空则跳过 | `[monitoring]` OTel gRPC exporter (**优先**; fallback `AIKV_OTLP_ENDPOINT`) |
| `OTEL_DEPLOYMENT_ENVIRONMENT` | 空 | Resource `deployment.environment` (**优先**; fallback `AIKV_DEPLOYMENT_ENV`) |
| `AIKV_OTLP_ENDPOINT` | — | `OTEL_EXPORTER_OTLP_ENDPOINT` 的 aikv fallback |
| `AIKV_DEPLOYMENT_ENV` | — | `OTEL_DEPLOYMENT_ENVIRONMENT` 的 aikv fallback |
| `AIKV_CLIENT_ADDR` | 从 rpc host + `--bind` port 推导 | 外部可达 `host:port` → MetaRaft `client_addr` |
| `AIKV_CLUSTER_ANNOUNCE_MODE` | 默认 `unknown` | `fixed` \| `unknown` — MOVED / CLUSTER SLOTS 通告; 见 [cluster.md](docs/modules/cluster.md) |

E2E 可选: `WIKV_HOST`, `WIKV_PORT`, `WIKV_CLUSTER_BASE_PORT` ([e2e/utils.sh](e2e/utils.sh)).

## 单机部署

### 内存引擎 (开发 / 测试)

数据 **不持久**; 启动时 stderr WARN.

```bash
cargo run --release --features cluster -- \
  --bind 127.0.0.1:6379 \
  --engine memory
```

### AiDb 引擎 (生产推荐)

```bash
mkdir -p /var/lib/aikv/data
cargo run --release --features cluster,monitoring -- \
  --bind 0.0.0.0:6379 \
  --engine aidb \
  --data-dir /var/lib/aikv/data \
  --metrics-addr 0.0.0.0 \
  --metrics-port 9191
```

验证:

```bash
redis-cli -h 127.0.0.1 -p 6379 PING
# [monitoring] 另开:
curl -s http://127.0.0.1:9191/health
```

## 集群部署

编译 **必须** `--features cluster`. 每个节点需要 **三个端口角色**:

| 角色 | 配置 | 示例 |
|------|------|------|
| RESP 客户端 | `--bind` | `127.0.0.1:6379` |
| MetaRaft 控制面 | `--cluster-rpc-addr` | `127.0.0.1:16379` |
| MultiRaft 数据面 | `rpc_port + --cluster-data-port-offset` | `16379 + 10000 = 26379` |

约束: `rpc_port + offset ≤ 65535`; 各节点 **offset 必须相同**; 变更 offset 需 **重启整个集群**.

### 三节点示例 (bootstrap + join)

**节点 1** (bootstrap, `--cluster-peers` 省略):

```bash
cargo run --release --features cluster,monitoring -- \
  --bind 127.0.0.1:6379 \
  --engine aidb --data-dir /var/lib/aikv/node1 \
  --cluster-node-id 1 \
  --cluster-rpc-addr 127.0.0.1:16379 \
  --metrics-port 9191
```

**节点 2** (join):

```bash
cargo run --release --features cluster,monitoring -- \
  --bind 127.0.0.1:6380 \
  --engine aidb --data-dir /var/lib/aikv/node2 \
  --cluster-node-id 2 \
  --cluster-rpc-addr 127.0.0.1:16380 \
  --cluster-peers 127.0.0.1:16379 \
  --metrics-port 9192
```

**节点 3** (join):

```bash
cargo run --release --features cluster,monitoring -- \
  --bind 127.0.0.1:6381 \
  --engine aidb --data-dir /var/lib/aikv/node3 \
  --cluster-node-id 3 \
  --cluster-rpc-addr 127.0.0.1:16381 \
  --cluster-peers 127.0.0.1:16379 \
  --metrics-port 9193
```

### 初始化拓扑 (redis-cli)

`CLUSTER MEET` 第三参数为 **MetaRaft RPC 端口** (cluster bus), 与 e2e 一致:

```bash
redis-cli -p 6379 CLUSTER MEET 127.0.0.1 6380 16380
redis-cli -p 6379 CLUSTER MEET 127.0.0.1 6381 16381
sleep 2   # MetaRaft 同步可能需要数秒

redis-cli -p 6379 CLUSTER NODES
redis-cli -p 6379 CLUSTER INFO    # 期望 cluster_state:ok
```

分配槽位 (3 主示例):

```bash
redis-cli -p 6379 CLUSTER ADDSLOTS $(seq 0 5460)
redis-cli -p 6380 CLUSTER ADDSLOTS $(seq 5461 10922)
redis-cli -p 6381 CLUSTER ADDSLOTS $(seq 10923 16383)
```

Smart client (**必需** — AiKv 无服务端透明转发, MOVED/ASK 由客户端重试):

```bash
redis-cli -c -p 6379
```

集群应用须使用 cluster-aware 客户端 (如 `redis-cli -c`, Jedis Cluster, go-redis cluster 模式). 非 cluster 模式客户端收到 MOVED 后不会自动重定向.

MOVED/ASK、failover、slot 迁移语义见 [docs/modules/cluster.md](docs/modules/cluster.md). MetaRaft/MultiRaft 实现见 [aidb cluster.md](../aidb/docs/modules/cluster.md).

### 跨 NAT / GUI 客户端 (可选)

WSL2 / Docker 端口转发、LAN 远程 GUI (如 Tiny RDM) 时, 客户端须能连通告地址:

```bash
export AIKV_CLIENT_ADDR=127.0.0.1:6379    # 或 LAN IP:port
export AIKV_CLUSTER_ANNOUNCE_MODE=fixed   # LAN/GUI; 默认 unknown 适合单种子连接
```

`AIKV_CLIENT_ADDR` 由后台 task 同步至 MetaRaft (含 bootstrap 节点).

## 监控与可观测性

### 编译与端点

| 项 | 说明 |
|----|------|
| Feature | **`monitoring`** 必须启用 |
| HTTP | `--metrics-addr`:`--metrics-port` (默认 `127.0.0.1:9191`) — 仅 `/health` |
| 路径 | `/health` (`200 OK`), `/` (说明页) |
| 生产指标 | **OTLP** (`OTEL_EXPORTER_OTLP_ENDPOINT` → Collector → Prom remote write) |
| Tracing | `RUST_LOG`; JSON 默认开 (`AIKV_JSON_LOG`) |
| OTel | `[monitoring]` + `OTEL_EXPORTER_OTLP_ENDPOINT` (gRPC, 如 `http://127.0.0.1:4317`; fallback `AIKV_OTLP_ENDPOINT`) |

全部 `aikv_*` / `aidb_*` (及 cluster 时 `aidb_raft_*`) 经 OTLP 出口. 指标全表见 [observability-reference.md](docs/modules/observability-reference.md).

### 外部监控栈对接 (摘要)

1. 构建 `--features cluster,monitoring`; 设 `OTEL_EXPORTER_OTLP_ENDPOINT` 指向 115 Collector `:4317`.
2. 115 Prometheus **不 scrape** aikv HTTP; node-exporter / cAdvisor 走经典 scrape.
3. 验证: `curl -s http://<host>:9191/health`; Prom 存在 `{service_name="aikv"}` 指标.
4. JSON 日志 (`AIKV_JSON_LOG=true`) 可经 Alloy → Loki; 面板 PromQL 前缀 **`aikv_*`** / **`aidb_*`** (非历史 `wiqun_kv_*`).

详情: [docs/modules/observability.md](docs/modules/observability.md).

### 告警参考 (可选)

| 指标 | 说明 | 提示 |
|------|------|------|
| `aikv_connected_clients` | 当前连接 | 接近 `--max-clients` |
| `aikv_commands_total{status="error"}` | 错误命令 | 突增 |
| `aikv_command_duration_seconds` | 延迟直方图 | p99 关注 |
| `aikv_slow_queries_total` | 慢查询 | 突增 |
| `aikv_cluster_redirects_total` | MOVED/ASK | 突增可能路由/迁移问题; 稳定后接近 0 |
| `aikv_gossip_messages_total` | 拓扑 tick | 低频 baseline; 无数据查 metrics 注入与 OTel 部署 |
| `aikv_failover_total` | 主从切换 | 正常应为 0 / 无 series |
| `aidb_raft_rpc_total` | Raft 复制 RPC | 见 aidb observability; Grafana **Cluster** 行 |
| `aikv_blocked_clients` | 阻塞客户端 (BLPOP 等) | 突增可能客户端堆积 |

## 备份与持久化运维

- 仅 **`--engine aidb`**; `memory` 返回 `ERR Persistence not supported on memory engine`.
- **非** Redis RDB/AOF 主路径 — `SAVE`/`BGSAVE` 走 AiDb **checkpoint** (见 [DESIGN.md §决策总表](DESIGN.md#决策总表) — 持久化主路径).
- 默认 checkpoint 目录: `{data_dir}/backup/` (可用 `--backup-dir` 覆盖).

```bash
redis-cli SAVE      # 同步 flush + checkpoint
redis-cli BGSAVE    # 后台 checkpoint
redis-cli LASTSAVE
```

AiDb `BackupManager` 全量备份 API 见 [aidb DEPLOYMENT §备份](../aidb/DEPLOYMENT.md#备份与恢复); AiKv `BGSAVE` **不** 经 `BackupManager`.

命令细节: [commands-extended.md](docs/modules/commands-extended.md).

## Docker 简例

```dockerfile
FROM rust:bookworm AS builder
WORKDIR /build
COPY aikv/ aikv/
COPY aidb/ aidb/
WORKDIR /build/aikv
RUN cargo build --release --features cluster,monitoring

FROM debian:bookworm-slim
COPY --from=builder /build/aikv/target/release/aikv /usr/local/bin/
EXPOSE 6379 9191
CMD ["aikv", "--bind", "0.0.0.0:6379", "--engine", "aidb", \
     "--data-dir", "/data", "--metrics-addr", "0.0.0.0", "--metrics-port", "9191"]
```

集群 compose 需为每节点映射 RESP、RPC、data 端口及独立 `--data-dir`; 可参考 [e2e/utils.sh](e2e/utils.sh) 端口间距.

## 相关文档

- [ARCHITECTURE.md](ARCHITECTURE.md) — 分层、启动顺序、AiDb 边界
- [DESIGN.md](DESIGN.md) — feature gate、持久化 why
- [AGENTS.md](AGENTS.md) — AI 助手与 CI 速查
- [aidb/DEPLOYMENT.md](../aidb/DEPLOYMENT.md) — protoc、aidb 数据目录、库侧 cluster
- [docs/modules/](docs/modules/) — 域级实现
- [ISSUES.md](ISSUES.md) — 待核实项

## 待核实

- 无 `[monitoring]` 时 runtime metrics 不自动 refresh — 见 [ISSUES.md#ISSUE-021](ISSUES.md#issue-021-refresh_runtime_metrics-仅-monitoring-后台-tick).
- metrics 后台 tick **15s** (非 1s spec) — 见 [ISSUES.md#ISSUE-022](ISSUES.md#issue-022-metrics-refresh-周期-15s-vs-设计-spec-1s).
