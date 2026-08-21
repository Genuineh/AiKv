# Changelog

本项目的所有重要变更都会记录在此文件中.

格式基于 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/),
版本号遵循 [语义化版本](https://semver.org/lang/zh-CN/).

## [Unreleased]

### Added

### Changed

- 存储层 (回传法): 集群 Plain / 迁移 PUT / `write_batch` 去掉 propose 前 `get_local`/`exists`; 消费 aidb `Response::WriteStats` 更新 `DbKeyCounters`; 单机 `AiDbEngine` 直接消费 `put`→`bool` / `EngineWriteStats` / `key_exists`; 写成功后无论 `inserted` 均 `expire_gate.release`; 数据写响应 fail-fast (同版本兼容)
- 存储层: 引入内存原子键计数器 `DbKeyCounters` 与冷启动同步 Rebuild, 消除后台 15s 指标任务 (`refresh_runtime_metrics`) 与 `DBSIZE` 命令的全库 SSTable 迭代扫描, 键计数复杂度降为 $O(1)$
- 连接层: 公开 `RespValue::encode_into`, 引入每连接 `write_buf` 复用与 Pipeline 批末聚合写 (`flush_responses`); 大响应后 capacity >64 KiB 收缩回 8 KiB
- 拆分 `src/server/otel_metrics.rs` 为 `otel_metrics/{mod,helpers,testutil}.rs`
- 拆分 `src/command/zset.rs` 为 `zset/{mod,helpers}.rs`
- 拆分 `src/command/script/execute.rs` 为 `execute/{mod,string,hash,list,set,zset}.rs`
- 拆分 `src/command/router.rs` 为 `router/{mod,keylock,stats}.rs`
- 拆分 `src/server/connection.rs` 为 `connection/{mod,protocol,monitor,atom}.rs`
- 拆分超限源文件: `src/cluster/commands/` (按 CLUSTER 子命令), `src/command/jsonpath/` (parser/eval/filter/mutate), `src/storage/cluster_batcher.rs` (写凑批 actor); `tests/cluster_*` 改为薄入口, 测试体迁入 `tests/modules/cluster/`
- 全面重构并优化项目核心文档与模块文档体系
- 统一贡献指南结构并全面规范化 Markdown 文档与标点
- 移除模块文档 frontmatter 中非标准的 `depends_on` 字段
- 命令层 tracing span 统一收敛为 `debug` 级别, 新增 `tests/span_contract.rs` 契约测试
- 序列化: `StoredValue` / DUMP 从 `bincode 1.x` 改为 `postcard 1.x`; DUMP 版本 `0` → `1` (开发期不兼容)
- GitHub CI: 各 job 先 `actions/checkout`, 再调用 prepare (不再在 composite 内 checkout)

### Fixed

- 存储层: `ClusterDataAdapter::write_batch` 改为 propose 后消费 `WriteStats` (不再前置 `exists`); Batcher reverse-dedup 按 key last-write-wins, 修复 Put↔Delete 时序颠倒导致 key 丢失
- 集群: `SETSLOT MIGRATING` / `REBALANCE` 拷贝或收尾失败自动 `Cancel`, 新增 `SETSLOT CANCEL`; `CLUSTER MEET` 对 ForwardToLeader 类错误退避重试; e2e 识别 NotLeader/CLUSTERDOWN 并在可执行节点上跑 MEET/迁移

- 依赖安全: `anyhow` 1.0.102 → 1.0.104 (RUSTSEC-2026-0190); 增加 `.cargo/audit.toml` (bincode ignore)
- 移除 tonic 0.11 传递依赖 h2 0.3.27 (RUSTSEC-2026-0258); 删除 SKIP_SECURITY 逃生门
- deny.toml 允许 Zlib (foldhash)
- `CLUSTER COUNTKEYSINSLOT` / `GETKEYSINSLOT` 不再在 tokio 运行时内 `block_on` panic 断连
- `BLPOP` / `BRPOP` / `BLMOVE` / `BZPOPMIN` / `BZPOPMAX` 对齐 Redis 8.8: `timeout=0` 无限阻塞, 负数 `timeout` 返回 `ERR timeout is negative`
- Lua `redis.call` 热路径 span `cmd_lua_redis_call` 降为 `debug` 级别
- 依赖安全: `crossbeam-epoch` 0.9.18 → 0.9.20 (RUSTSEC-2026-0204), `h2` 0.4.15 → 0.4.16 (RUSTSEC-2026-0258 部分), `deny.toml` 豁免 wit-bindgen/hashbrown 构建链重复版本
