# AiKv 观测栈 (Observability Stack)

本目录包含与 `docker-compose.otel.yaml` 配套的配置，用于指标、日志、链路追踪与持续剖析的一键部署。

## 架构概览

| 数据类型   | 采集/接收           | 存储/后端    | 可视化        |
|-----------|---------------------|-------------|---------------|
| Redis 指标 | redis-exporter      | Prometheus  | Grafana       |
| 系统指标   | node-exporter       | Prometheus  | Grafana       |
| OTLP 指标  | OTel Collector      | Prometheus  | Grafana       |
| 链路追踪   | OTel Collector      | Jaeger+Tempo| Grafana Explore |
| 日志       | Promtail            | Loki        | Grafana       |
| Profile    | Alloy (eBPF)        | Pyroscope   | Grafana Profiles |

## 快速开始

```bash
# 仅启动观测栈（推荐配合 ak 使用）
docker compose -f docker-compose.otel.yaml up -d
ak quick

# 启动观测栈 + 本 compose 的 AiKv 实例
docker compose -f docker-compose.otel.yaml --profile app up -d
```

| 服务 | 地址 | 用途 |
|-----|------|------|
| Grafana | http://localhost:3000 | 统一可视化仪表盘 |
| Prometheus | http://localhost:9090 | 指标查询与告警 |
| Jaeger UI | http://localhost:16686 | 链路追踪查看 |
| Tempo API | http://localhost:3200 | TraceQL 直连查询 |
| Loki | http://localhost:3100 | 日志聚合查询 |
| Pyroscope | http://localhost:4040 | 性能剖析火焰图 |
| Alloy UI | http://localhost:12345 | eBPF 采集状态 |
