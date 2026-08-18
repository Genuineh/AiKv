# Changelog

本项目的所有重要变更都会记录在此文件中.

格式基于 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/),
版本号遵循 [语义化版本](https://semver.org/lang/zh-CN/).

## [Unreleased]

### Added

### Changed

- 全面重构并优化项目核心文档与模块文档体系
- 统一贡献指南结构并全面规范化 Markdown 文档与标点
- 移除模块文档 frontmatter 中非标准的 `depends_on` 字段
- 命令层 tracing span 统一收敛为 `debug` 级别, 新增 `tests/span_contract.rs` 契约测试

### Fixed

- `CLUSTER COUNTKEYSINSLOT` / `GETKEYSINSLOT` 不再在 tokio 运行时内 `block_on` panic 断连
- `BLPOP` / `BRPOP` / `BLMOVE` / `BZPOPMIN` / `BZPOPMAX` 对齐 Redis 8.8: `timeout=0` 无限阻塞, 负数 `timeout` 返回 `ERR timeout is negative`
