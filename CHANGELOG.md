# Changelog

本项目的所有重要变更都会记录在此文件中.

格式基于 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/),
版本号遵循 [语义化版本](https://semver.org/lang/zh-CN/).

## [Unreleased]

### Fixed

- 集群启用时拒绝 `engine=memory`, 避免 `init_cluster` panic 与绕过 Raft 的内存写入 (`#77`).
- 修复 `MULTI/EXEC` 跨 slot 部分执行与事务期间客户端命令插队问题, 并对齐 Redis 运行时错误不回滚语义 (`#78`).
- WATCH 版本改为存储层 meta key (含 DB), 随写入 apply 递增, 支持跨连接冲突检测 (`#79`).
