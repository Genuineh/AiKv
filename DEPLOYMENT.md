# AiKv 部署指南

AiKv 是 RESP 兼容的 KV 服务, 支持单机和集群模式.

## 系统要求

- Rust 工具链: stable 分支 (见 `rust-toolchain.toml`)
- 操作系统: Linux / macOS
- 磁盘 (持久化模式): SSD 推荐, 取决于数据量
- 内存: MemoryEngine 全量数据在内存, 评估数据规模预留

## 构建

```bash
git clone <repo-url>
cd AiKv

# 单机版
cargo build --release

# 集群版
cargo build --release --features cluster
```

## 单机部署

### 内存模式

```bash
./target/release/aikv \
  --engine memory \
  --port 6379 \
  --bind 127.0.0.1
```

### 持久化模式

```bash
./target/release/aikv \
  --engine aidb \
  --data-dir /var/lib/aikv/data \
  --port 6379
```

生产建议启用监控:

```bash
./target/release/aikv \
  --engine aidb \
  --data-dir /var/lib/aikv/data \
  --port 6379 \
  --metrics-port 9191
```

## 集群部署

### 节点启动 (3 节点示例)

```bash
# 节点 1
./target/release/aikv \
  --engine memory \
  --port 6379 \
  --cluster-mode \
  --cluster-addr 127.0.0.1:6380 \
  --node-id node-6379 \
  --metrics-port 9191

# 节点 2
./target/release/aikv \
  --engine memory \
  --port 6381 \
  --cluster-mode \
  --cluster-addr 127.0.0.1:6382 \
  --node-id node-6381 \
  --metrics-port 9192

# 节点 3
./target/release/aikv \
  --engine memory \
  --port 6383 \
  --cluster-mode \
  --cluster-addr 127.0.0.1:6384 \
  --node-id node-6383 \
  --metrics-port 9193
```

### 初始化集群

```bash
# 在节点 1 上初始化集群
redis-cli -p 6379 CLUSTER MEET 127.0.0.1 6382
redis-cli -p 6379 CLUSTER MEET 127.0.0.1 6384

# 验证集群状态
redis-cli -p 6379 CLUSTER INFO
redis-cli -p 6379 CLUSTER NODES
```

### 分配槽位

```bash
# 分配槽位到节点 1 (0-5460)
redis-cli -p 6379 CLUSTER ADDSLOTS {0..5460}

# 分配槽位到节点 2 (5461-10921)
redis-cli -p 6381 CLUSTER ADDSLOTS {5461..10921}

# 分配槽位到节点 3 (10922-16383)
redis-cli -p 6383 CLUSTER ADDSLOTS {10922..16383}
```

> 数据面 gRPC 端口 = RPC 端口 + --cluster-data-port-offset (默认 10000). 所有节点必须使用相同的偏移值; 偏移变更需重启整个集群.

### 集群客户端

```bash
redis-cli -c -p 6379

# 自动 MOVED 重定向
127.0.0.1:6379> SET foo bar
-> Redirected to slot [12182] located at 127.0.0.1:6383
OK
```

### WSL2 + Windows GUI 客户端 (Tiny RDM 等)

Docker 集群在 WSL2 中运行时, Windows 宿主机经 `127.0.0.1` 端口转发访问, **不能** 使用 WSL 网卡 IP (`192.168.x.x`) 作为集群拓扑通告地址.

推荐环境变量:

| 变量 | 默认 (WSL2) | 作用 |
|------|------------|------|
| `AIKV_EXTERNAL_HOST` | `127.0.0.1` | 写入 MetaRaft `client_addr`, 显示在 `CLUSTER NODES` |
| `AIKV_CLUSTER_ANNOUNCE_MODE` | WSL2: `unknown`; Linux 物理机: **`fixed`** | `CLUSTER SLOTS`/MOVED; LAN/GUI 请用 `fixed` |
| `AIKV_CLIENT_ADDR` | 从 bind 推导 | 外部可达 `host:port`, 同步至 MetaRaft |

局域网远程客户端需显式:

```bash
export AIKV_EXTERNAL_HOST=<主机 LAN IP>
export AIKV_CLUSTER_ANNOUNCE_MODE=fixed
# 然后按上文启动各 aikv 节点
```

## Docker 部署

### 单机

```dockerfile
FROM rust:latest AS builder
WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
COPY --from=builder /app/target/release/aikv /usr/local/bin/
EXPOSE 6379 9191
CMD ["aikv", "--engine", "aidb", "--data-dir", "/data", "--port", "6379"]
```

### docker-compose (可观测性栈)

可按需编写 docker-compose, 将 aikv 与 Prometheus / OTel Collector 等服务编排在一起. 示例:

```yaml
services:
  aikv:
    build: .
    ports:
      - "6379:6379"
      - "9191:9191"
    environment:
      - AIKV_JSON_LOG=true
  prometheus:
    image: prom/prometheus
    ports:
      - "9090:9090"
    volumes:
      - ./prometheus.yml:/etc/prometheus/prometheus.yml
```

## 命令行参数参考

| 参数 | 默认值 | 说明 |
|------|--------|------|
| `--bind` | 127.0.0.1 | 监听地址 |
| `--port` | 6379 | 监听端口 |
| `--engine` | memory | 存储引擎: `memory` / `aidb` |
| `--data-dir` | - | AiDb 数据目录 |
| `--metrics-port` | 9191 | Prometheus `/metrics` 端口 |
| `--metrics-addr` | 127.0.0.1 | Metrics 监听地址 |
| `--cluster-mode` | - | 启用集群模式 |
| `--cluster-addr` | - | 集群内部通信地址 |
| `--node-id` | - | 节点唯一标识 |
| `--cluster-data-port-offset` | `10000` | 数据面总线端口偏移, data_port = rpc_port + offset, 所有节点必须一致 |

## 监控告警

### 关键指标

| 指标 | 说明 | 告警阈值 |
|------|------|---------|
| `aikv_connected_clients` | 连接数 | > 预期上限 |
| `aikv_commands_total{status="error"}` | 错误命令数 | 突增 > 10% |
| `aikv_commands_total{status="ok"}` | 成功命令数 | 突降 (可能服务异常) |
| `kv_command_duration_seconds` | 命令延迟 | p99 > 100ms |
| `kv_slow_queries_total` | 慢查询数 | 突增 |
| `aikv_cluster_redirects_total` | MOVED/ASK 数 | 突增 (路由问题) |

### 健康检查

```
GET /health → 200 OK
```

## 备份

### 手动备份

```bash
# 持久化模式
redis-cli SAVE
redis-cli BGSAVE
redis-cli LASTSAVE
```
