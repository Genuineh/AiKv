# Changelog

本项目的所有重要变更都会记录在此文件中.

格式基于 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/),
版本号遵循 [语义化版本](https://semver.org/lang/zh-CN/).

## [Unreleased]

### Added

### Changed

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

### Fixed

- 移除 tonic 0.11 传递依赖 h2 0.3.27 (RUSTSEC-2026-0258); 删除 SKIP_SECURITY 逃生门
- deny.toml 允许 Zlib (foldhash)
- `CLUSTER COUNTKEYSINSLOT` / `GETKEYSINSLOT` 不再在 tokio 运行时内 `block_on` panic 断连
- `BLPOP` / `BRPOP` / `BLMOVE` / `BZPOPMIN` / `BZPOPMAX` 对齐 Redis 8.8: `timeout=0` 无限阻塞, 负数 `timeout` 返回 `ERR timeout is negative`
- Lua `redis.call` 热路径 span `cmd_lua_redis_call` 降为 `debug` 级别
- 依赖安全: `crossbeam-epoch` 0.9.18 → 0.9.20 (RUSTSEC-2026-0204), `h2` 0.4.15 → 0.4.16 (RUSTSEC-2026-0258 部分), `deny.toml` 豁免 wit-bindgen/hashbrown 构建链重复版本
