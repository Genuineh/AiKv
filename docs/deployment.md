---
name: aikv-deployment
description: AiKv 构建、配置、单机/集群部署与生产运维指南 (How to Build, Configure & Operate). 包含 Cargo feature 矩阵、CLI 命令行参数、环境变量、单机/集群部署示例、Docker 部署与监控告警接入.
---

# AiKv 部署与运行

本文面向运维工程师与系统开发者, 说明 **如何构建 AiKv、配置 Cargo feature、运行单机与 Redis Cluster 分布式集群、配置 OTLP 监控告警以及执行持久化快照运维**.

架构分层与数据流 (How it works) 见 [ARCHITECTURE.md](../ARCHITECTURE.md); 设计权衡 (Why) 见 [docs/design.md](design.md); 存储内核底层运维见 [AiDb 部署指南](../../aidb/docs/deployment.md).

---

## 1. 系统要求与硬件规划

| 资源维度 | 生产推荐配置 | 最低运行要求 | 运维与规划说明 |
| :--- | :--- | :--- | :--- |
| **Rust 工具链** | Rust **stable** | 声明于 `rust-toolchain.toml` | 包含 `clippy` 与 `rustfmt` |
| **操作系统** | Linux (kernel ≥ 5.4) / macOS | Linux x86_64 / aarch64 | CI 运行在 `ubuntu-latest` |
| **磁盘存储** | 高性能 NVMe SSD | 标准 SSD | 持久化模式 (`--engine aidb`) 依赖磁盘顺序与随机 IOPS |
| **内存容量** | 8 GiB ~ 64 GiB+ | 1 GiB | `memory` 引擎全驻留内存; `aidb` 引擎内存由 MemTable 与 BlockCache 决定 |
| **Protobuf 编译器** | `protoc` (最新 stable) | 系统包管理器版本 | 仅编译 `cluster` 特性时生成 Raft gRPC 桩代码需要 |
| **网络端口规划** | 独占 6379, 16379, 26379, 9191 | 见端口角色表 | 集群模式单节点需规划 3 个互不冲突的通信端口 |

### Monorepo 依赖关系与本地 Patch

AiKv 依赖 sibling 存储引擎 `aidb` (分支 `new/main`). 本地协同开发时, 在 `~/.cargo/config.toml` 中配置本地覆盖:

```toml
[patch."https://github.com/wiqun/AiDb.git"]
aidb = { path = "../aidb" }
```

---

## 2. Cargo Features 构建矩阵

定义声明见 [Cargo.toml](../Cargo.toml):

| Feature | 默认状态 | 包含模块与依赖 | 适用场景 |
| :--- | :--- | :--- | :--- |
| **(none)** | ✅ | 极简单机 RESP 服务, 内存/单机 LSM 引擎 | 最小二进制发布 |
| **`cluster`** | ❌ | `src/cluster/`, `ClusterDataAdapter`, `aidb/cluster` | Redis Cluster 分布式集群模式 |
| **`monitoring`** | ❌ | `MetricsServer` (`/health` HTTP), OTel 指标/链路 | 生产环境 OTLP 监控接入与健康探活 |
| **`compression`** | ❌ | `aidb/compression` (Snap / LZ4 块压缩) | 开启 LSM SSTable 块压缩以降低磁盘占用 |

### 典型构建组合

```bash
# 1. 本地开发与 CI 门禁构建
cargo build --release --features cluster

# 2. 生产环境标准镜像构建 (推荐全功能)
cargo build --release --features cluster,monitoring,compression

# 3. 生产环境基础镜像构建 (无块压缩)
cargo build --release --features cluster,monitoring
```

---

## 3. 命令行参数与环境变量

### 3.1 命令行参数 (`Args`)

权威定义位于 [`src/main.rs`](../src/main.rs):

| 参数项 | 默认值 | 依赖 Feature / 引擎 | 说明 |
| :--- | :--- | :--- | :--- |
| `--bind` | `127.0.0.1:6379` | 始终有效 | RESP 客户端 TCP 监听地址 (`host:port`) |
| `--engine` | `memory` | 始终有效 | 存储引擎类型: `memory` (开发测试) \| `aidb` (生产推荐) |
| `--data-dir` | — | `aidb` / `cluster` 必填 | 数据持久化与 WAL 存放根目录 |
| `--sync-wal` | `false` | `aidb` / `cluster` | 是否每条写操作强制 fsync WAL (强持久, 吞吐下降) |
| `--aidb-preset` | `default` | `aidb` | LSM 参数预设: `default` \| `high-write` \| `high-read` |
| `--backup-dir` | `{data_dir}/backup` | 可选 | `SAVE` / `BGSAVE` Checkpoint 快照输出目录 |
| `--cluster-node-id` | — | `cluster` | 当前节点的唯一 u64 节点 ID |
| `--cluster-rpc-addr` | — | `cluster` | MetaRaft 控制面 gRPC 监听地址 (`host:port`) |
| `--cluster-peers` | `[]` | `cluster` | 集群已知节点 RPC 列表 (逗号分隔; 空表示引导节点) |
| `--raft-election-timeout-min` | `1000` | `cluster` | Raft 最小选举超时 (毫秒) |
| `--raft-election-timeout-max` | `2000` | `cluster` | Raft 最大选举超时 (毫秒) |
| `--raft-rpc-timeout-ms` | `500` | `cluster` | Raft RPC 请求超时 (毫秒, 须小于最小选举超时) |
| `--raft-heartbeat-interval` | `300` | `cluster` | Raft Leader 心跳间隔 (毫秒) |
| `--lifecycle-tick-ms` | `1000` | `cluster` | LifecycleManager 生命周期巡检周期 (毫秒) |
| `--gossip-interval` | `1` | `cluster` | 拓扑轻量刷新与 Gossip 指标周期 (秒) |
| `--config-auto-save-ms` | `2000` | `cluster` | 集群节点拓扑状态自动持久化间隔 (毫秒) |
| `--cluster-data-port-offset` | `10000` | `cluster` | MultiRaft 数据面端口偏移 (`data_port = rpc_port + offset`) |
| `--metrics-addr` | `127.0.0.1` | CLI 始终有效 | HTTP 探活监听 IP (仅在 `monitoring` 生效) |
| `--metrics-port` | `9191` | CLI 始终有效 | HTTP 探活监听端口 (仅在 `monitoring` 生效) |
| `--max-clients` | `10000` | 始终有效 | 最大并发客户端连接数 (`0` 表示无限制) |

> **集群模式启动门控**: 仅当 **`--cluster-node-id` 与 `--cluster-rpc-addr` 同时提供** 时才会初始化集群状态机; 仅提供其一将退化为单机模式运行.

### 3.2 环境变量配置

| 环境变量 | 默认值 | 作用说明 |
| :--- | :--- | :--- |
| `RUST_LOG` | `info` | Tracing 日志级别过滤指令 |
| `AIKV_JSON_LOG` | `true` | 是否以 JSON 格式输出结构化日志 (`true` \| `false`) |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | 空 (跳过) | OpenTelemetry Collector gRPC 收集端点 (如 `http://127.0.0.1:4317`) |
| `OTEL_SERVICE_NAME` | `aikv` | OTel Resource 中的服务名称 |
| `OTEL_DEPLOYMENT_ENVIRONMENT` | 空 | OTel Resource 中的部署环境 (`production` / `staging`) |
| `AIKV_CLIENT_ADDR` | 自动推导 | 外部客户端访问该节点实际可达的 `host:port` (用于跨 NAT/容器通告) |
| `AIKV_CLUSTER_ANNOUNCE_MODE` | `unknown` | 通告模式: `unknown` (动态推导) \| `fixed` (固定使用 `AIKV_CLIENT_ADDR`) |
| `AIKV_OTEL_SAMPLE_RATIO` | `1.0` | 链路追踪采样率 (`0.0` ~ `1.0`) |

---

## 4. 单机部署实践

### 4.1 内存引擎模式 (仅限开发与集成测试)

```bash
cargo run --release --features cluster -- \
  --bind 127.0.0.1:6379 \
  --engine memory
```

### 4.2 AiDb 引擎模式 (单机生产持久化)

```bash
mkdir -p /var/lib/aikv/data

cargo run --release --features cluster,monitoring,compression -- \
  --bind 0.0.0.0:6379 \
  --engine aidb \
  --data-dir /var/lib/aikv/data \
  --aidb-preset high-write \
  --metrics-addr 0.0.0.0 \
  --metrics-port 9191
```

**连接与验证**:
```bash
# 1. RESP 验证
redis-cli -h 127.0.0.1 -p 6379 PING
redis-cli -h 127.0.0.1 -p 6379 SET mykey "hello world"
redis-cli -h 127.0.0.1 -p 6379 GET mykey

# 2. HTTP 探活验证
curl -s http://127.0.0.1:9191/health
```

---

## 5. Redis Cluster 分布式集群部署

### 5.1 节点端口角色规划

每个集群节点在物理机或容器上需要分配 **3 个端口角色**:

```
[Client (redis-cli -c)] ---> 6379  (RESP Client Port: --bind)
[MetaRaft Leader/Peer]  ---> 16379 (MetaRaft Control Port: --cluster-rpc-addr)
[MultiRaft Data Shard]  ---> 26379 (MultiRaft Data Port: rpc_port + offset 10000)
```

> **约束**: 保证 `rpc_port + offset ≤ 65535`, 全集群所有节点的 `--cluster-data-port-offset` 必须严格一致.

### 5.2 三节点集群启动示例

**节点 1 (Bootstrap 引导节点)**:
```bash
cargo run --release --features cluster,monitoring -- \
  --bind 127.0.0.1:6379 \
  --engine aidb --data-dir /var/lib/aikv/node1 \
  --cluster-node-id 1 \
  --cluster-rpc-addr 127.0.0.1:16379 \
  --metrics-port 9191
```

**节点 2 (加入节点)**:
```bash
cargo run --release --features cluster,monitoring -- \
  --bind 127.0.0.1:6380 \
  --engine aidb --data-dir /var/lib/aikv/node2 \
  --cluster-node-id 2 \
  --cluster-rpc-addr 127.0.0.1:16380 \
  --cluster-peers 127.0.0.1:16379 \
  --metrics-port 9192
```

**节点 3 (加入节点)**:
```bash
cargo run --release --features cluster,monitoring -- \
  --bind 127.0.0.1:6381 \
  --engine aidb --data-dir /var/lib/aikv/node3 \
  --cluster-node-id 3 \
  --cluster-rpc-addr 127.0.0.1:16381 \
  --cluster-peers 127.0.0.1:16379 \
  --metrics-port 9193
```

### 5.3 集群拓扑初始化与槽位分配

通过标准 `redis-cli` 执行初始化:

```bash
# 1. 节点握手 (第三参数指定 MetaRaft RPC 端口)
redis-cli -p 6379 CLUSTER MEET 127.0.0.1 6380 16380
redis-cli -p 6379 CLUSTER MEET 127.0.0.1 6381 16381
sleep 2

# 2. 检查节点拓扑
redis-cli -p 6379 CLUSTER NODES

# 3. 分配 16384 个槽位 (3 主节点平均分配)
redis-cli -p 6379 CLUSTER ADDSLOTS $(seq 0 5460)
redis-cli -p 6380 CLUSTER ADDSLOTS $(seq 5461 10922)
redis-cli -p 6381 CLUSTER ADDSLOTS $(seq 10923 16383)

# 4. 验证集群状态
redis-cli -p 6379 CLUSTER INFO # 应输出 cluster_state:ok
```

### 5.4 智能客户端访问 (Smart Client)

由于 AiKv 不做服务端透明转发, **客户端必须使用集群模式连接**:

```bash
redis-cli -c -p 6379
```

当客户端请求的 Key 处于其他节点分配的槽位时, 服务端将返回 `-MOVED <slot> <target_addr>`, 由智能客户端自动更新本地路由缓存并向目标节点重试.

---

## 6. 监控与可观测性接入运维

### 6.1 架构与导出机制

```mermaid
flowchart LR
    Aikv[AiKv 实例] -->|OTLP gRPC: 4317| Collector[OpenTelemetry Collector]
    Collector -->|Remote Write| Prometheus[(Prometheus)]
    Collector -->|OTLP| Jaeger[(Jaeger / Tempo)]
    K8s[K8s 探针] -->|HTTP GET: 9191| Health[AiKv /health]
```

1. **编译要求**: 必须启用 `--features monitoring`;
2. **端点配置**: 设置 `OTEL_EXPORTER_OTLP_ENDPOINT=http://<collector_host>:4317`;
3. **健康探针**: Kubernetes Liveness / Readiness 探针配置 `http://<aikv_ip>:9191/health`;
4. **日志输出**: 结构化 JSON 日志通过标准输出由 Fluentbit / Vector 收集至 Loki.

### 6.2 关键生产监控告警指标

| 告警指标项 | 指标名 | 关注阈值与排查建议 |
| :--- | :--- | :--- |
| **连接数过载** | `aikv_connected_clients` | 达到 `--max-clients` 的 80% 时预警连接池泄漏 |
| **命令错误率突增** | `aikv_commands_total{status="error"}` | 非预期的 WRONGTYPE 或协议错误突增 |
| **P99 延迟劣化** | `aikv_command_duration_seconds` | 关注 LSM Compaction 阻塞或磁盘 IO 瓶颈 |
| **慢查询激增** | `aikv_slow_queries_total` | 业务大 Key 扫描或复杂 Lua 脚本执行 |
| **集群重定向异常** | `aikv_cluster_redirects_total` | 客户端未开启 Cluster 模式或槽位迁移期间短暂升高 |
| **主从切换事件** | `aikv_failover_total` | 发生非预期的 Raft Leader 重新选举 |
| **阻塞连接堆积** | `aikv_blocked_clients` | `BLPOP` / `BRPOP` 等待队列异常堆积 |

全部 `aikv_*` 指标全量清单参考 [08-observability-reference.md](modules/08-observability-reference.md).

---

## 7. 持久化与快照运维

- 持久化仅在 `--engine aidb` 模式下生效;
- `SAVE` / `BGSAVE` 基于 AiDb 的硬链接 Checkpoint 实现秒级一致性快照, 默认保存在 `{data_dir}/backup/` 目录下;
- 快照目录包含一致性 SSTable 硬链接与 MANIFEST 元数据, 可直接拷贝用于离线灾备与数据恢复.

```bash
# 触发后台快照
redis-cli -p 6379 BGSAVE

# 查询最后一次快照成功时间戳
redis-cli -p 6379 LASTSAVE
```

---

## 8. Docker 与 AiFactory 生产部署

### 8.1 生产 Dockerfile 参考

```dockerfile
FROM rust:bookworm AS builder
WORKDIR /build
COPY aikv/ aikv/
COPY aidb/ aidb/
WORKDIR /build/aikv
RUN cargo build --release --features cluster,monitoring,compression

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/aikv/target/release/aikv /usr/local/bin/
EXPOSE 6379 16379 26379 9191
CMD ["aikv", "--bind", "0.0.0.0:6379", "--engine", "aidb", \
     "--data-dir", "/data", "--metrics-addr", "0.0.0.0", "--metrics-port", "9191"]
```

### 8.2 AiFactory 集成套件

在协同套件 `aifactory` 中提供了一键式集群与监控编排:
- **单机容器启动**: `aifactory/scripts/up-single.sh`
- **参数化集群编排**: `aifactory/scripts/up-cluster.sh --shards 3 --replicas 1`
- **性能基准压测**: `aifactory/benchmark/benchmark.sh aikv`
- **停止与清理**: `aifactory/scripts/down.sh`
