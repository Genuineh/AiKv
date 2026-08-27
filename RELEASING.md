---
name: releasing
description: AiKv 版本预检、候选门禁与发布流程
---

# AiKv 发布手册

本文描述 `aikv` crate 与 AiKv 二进制的 `1.x` 发布流程. 发布范围以 [兼容矩阵](docs/compatibility.md) 为准, 发布前必须同步检查 [安全政策](SECURITY.md) 与 [部署边界](docs/deployment.md).

## 1. 发布前提

1. 确认 `Cargo.toml` 的版本, `Cargo.lock` 的依赖版本和 `CHANGELOG.md` 的版本条目一致.
2. 确认依赖 `aidb 1.0.0` 已经可以从 registry 解析. AiDb 仍处于本地或未发布状态时, 不得把 AiKv 当作可发布候选.
3. 确认 `1.0.0` 的稳定范围仅包括已文档化的 Rust public API, CLI/config 配置面, RESP2/RESP3 array framing, Pipeline 以及 [兼容矩阵](docs/compatibility.md) 列出的命令. 未文档化的内部实现, 未列出的 Redis 命令和 AiDb 内部 API 不属于稳定承诺.
4. 确认已阅读 [安全政策](SECURITY.md): v1 无内建 `AUTH`, `ACL`, `TLS`, 且 `RESP`, `MetaRaft`, `MultiRaft` 端口不能暴露到不可信网络.

## 2. 预检

预检用于本地发现问题, 可以在尚未提交的工作树执行. 需要打包或 dry-run 时可使用 `--allow-dirty`, 但这不构成发布候选:

```bash
cargo fmt --check
RUSTFLAGS='-D warnings' cargo clippy --all-targets --features cluster,monitoring
cargo test --features cluster,monitoring -- --test-threads=1
cargo package --allow-dirty
cargo publish --dry-run --allow-dirty
```

预检还应确认以下内容:

- `CHANGELOG.md` 的 `1.0.0` 日期使用实际 UTC 日期, 并记录 API/RESP 稳定范围与数据兼容边界.
- Linux x86_64 是正式支持平台; 其他平台仅 best-effort.
- 从 v1 之前版本升级时, 数据目录, `DUMP`, Raft snapshot 和已有集群均不可原地升级或滚动升级. 必须准备新部署及经过验证的迁移或恢复方案, 不得混用不同版本节点或持久化产物.

## 3. 最终门禁

最终门禁只允许针对已 commit, 已 Tag 且已由用户 push 的候选执行. 最终门禁不得使用 `--allow-dirty`, 也不得在未提交或未推送的工作树上代替候选验证:

```bash
git status --short
git log -1 --oneline
TAG="v1.0.0"
test "$(git describe --exact-match --tags HEAD)" = "$TAG"
git ls-remote --tags origin "$TAG"

cargo fmt --check
RUSTFLAGS='-D warnings' cargo clippy --all-targets --features cluster,monitoring
cargo test --features cluster,monitoring -- --test-threads=1
cargo publish --locked
```

`git status --short` 必须无输出, `git describe --exact-match --tags HEAD` 必须指向目标版本 Tag, `git ls-remote` 必须能看到用户已 push 的目标 Tag. 发布命令由用户在确认候选后执行; 本手册不授权自动 commit, Tag, push 或 publish.

## 4. 发布后核对

- 从 registry 查询 `aikv` 的版本与 checksum, 确认包内容不含本地路径或未声明文件.
- 在 Linux x86_64 上使用发布包完成一次单机启动和 RESP `PING` 验证.
- 集群发布验证必须确认 `MetaRaft` 和 `MultiRaft` 端口只对受信任节点开放.
- 将安全问题引导至 [Private Vulnerability Reporting](https://github.com/wiqun/AiKv/security/advisories/new). 除非另有明确确认, 不承诺固定响应 SLA.
