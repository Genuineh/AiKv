# 贡献指南

本文说明 **如何本地验证、通过门禁、运行测试与提交 PR**. 项目概览见 [README.md](README.md); 构建与 feature 见 [DEPLOYMENT.md](DEPLOYMENT.md); CI 流程图与 job 详表见 [.github/README.md](.github/README.md).

## 仓库结构

```shell
src/
├── main.rs      # CLI 入口
├── lib.rs       # 库入口 (导出 protocol/server/command/storage/cluster)
├── error.rs     # Error / Result
├── protocol/    # RESP2/3 编解码
├── server/      # TCP Listener/Connection, INFO/slowlog/latency
├── command/     # CommandRouter, 数据结构/JSON/Lua/持久化命令
├── storage/     # KvStorage, MemoryEngine, AiDbEngine
└── cluster/     # cluster feature — MOVED/ASK, CLUSTER 子命令
```

实现细节见 [docs/modules/](docs/modules/); 分层架构见 [ARCHITECTURE.md](ARCHITECTURE.md).

## 工具链与 Monorepo

[`rust-toolchain.toml`](rust-toolchain.toml) 固定 **stable**, 含 `clippy` / `rustfmt`, 与 GitHub Actions 一致. 进入仓库目录后 `rustup` 会自动切换; 可用 `rustup show` 确认.

**path 依赖**: `Cargo.toml` 中 `aidb = { path = "../aidb" }`. 本地需 sibling 布局:

```text
parent/
├── aidb/    # wiqun/AiDb
└── aikv/    # wiqun/AiKv
```

CI 会 checkout 同名分支的 `wiqun/AiDb` 并 `ln -sf` 到 `../aidb`; 只改 aikv 时, 远程也应有对应分支的 AiDb.

开发与 CI 以 **`--features cluster`** 为主路径. `cluster` 启用 `aidb/cluster` (gRPC), 本地 clippy/测试需本机 **protoc**:

```bash
# Debian/Ubuntu
sudo apt-get install -y protobuf-compiler
```

详见 [aidb/DEPLOYMENT.md §构建与验证](../aidb/DEPLOYMENT.md#构建与验证).

## Git hooks

推送前建议安装 pre-commit (fmt + clippy, **不含 test**):

```bash
./install-hooks.sh   # 软链 hooks/* → .git/hooks/
```

[`hooks/pre-commit`](hooks/pre-commit) 依次执行:

1. 检查 `../aidb/Cargo.toml` 存在
2. `cargo fmt --check`
3. `cargo clippy --all-targets --features cluster` (`RUSTFLAGS='-D warnings'`)

**注意**: hook **不跑** `cargo test`; 测试在 CI (或 push 前手动) 执行.

## 本地验证 vs CI

| 层级 | 做什么 | 何时失败 |
|------|--------|----------|
| pre-commit | fmt + clippy (`--features cluster`) | `git commit` |
| CI `test-cluster` | link aidb → fmt → clippy (cluster) → `cargo test --workspace --features cluster -- --test-threads=1` | push / PR |
| CI `test-server-stress` | `--test server -- --ignored` (TCP 压测) | `test-cluster` 通过后 |
| CI `test-commands-slow` | `--test commands -- --ignored` (TTL 慢测) | `test-cluster` 通过后 |
| CI `e2e` | release 构建 + `e2e/test_cluster_*.sh` (需 redis-cli) | `test-cluster` 通过后 |
| Security | `cargo audit` + `cargo deny check` | push / PR / 每日 cron |

Security ([`.github/workflows/security.yml`](.github/workflows/security.yml)) 与主 CI **并行、互不阻塞**. 同一分支新 push 会 cancel 未完成的旧 CI run.

触发分支: `main`, `new/main`, `new/wiqun` (见 [`.github/workflows/ci.yml`](.github/workflows/ci.yml)).

### 推送前推荐命令

```bash
export RUSTFLAGS='-D warnings'
cargo fmt --check
cargo clippy --all-targets --features cluster   # 需 protoc
cargo test --workspace --features cluster -- --test-threads=1
```

慢测 (与 CI `test-server-stress` / `test-commands-slow` 一致):

```bash
cargo test --test server --features cluster -- --ignored --test-threads=1
cargo test --test commands --features cluster -- --ignored --test-threads=1
```

与 [AGENTS.md](AGENTS.md) 速查块相同; job 细节见 [.github/README.md](.github/README.md).

## 完整测试矩阵

README 仅链入本篇; 集成测 **推荐** `--test-threads=1`. 分层说明见 [`tests/README.md`](tests/README.md).

### 按层级

| 层级 | 命令 | 说明 |
|------|------|------|
| **L0** | `cargo test --lib --features cluster` | `src/**` 单元测试 |
| **L1** | `cargo test --test resp --features cluster -- --test-threads=1` | RESP golden + parser 边界 |
| **L1** | `cargo test --test storage --features cluster -- --test-threads=1` | MemoryEngine + AiDb adapter |
| **L1** | `cargo test --test commands --features cluster -- --test-threads=1` | CommandRouter 全命令族 |
| **L2** | `cargo test --test server --features cluster -- --test-threads=1` | TCP listener + 内联命令 smoke |
| **L3 cluster** | 见下表 | 集群协议/路由/集成 (需 `cluster` feature) |

### L1 模块入口

```bash
cargo test --test resp --features cluster -- --test-threads=1
cargo test --test storage --features cluster -- --test-threads=1
cargo test --test commands --features cluster -- --test-threads=1
cargo test --test server --features cluster -- --test-threads=1
```

### Cluster integration target (`--features cluster`)

```bash
cargo test --test cluster_commands --features cluster -- --test-threads=1
cargo test --test cluster_creategroup --features cluster -- --test-threads=1
cargo test --test cluster_integration --features cluster -- --test-threads=1
cargo test --test cluster_routing --features cluster -- --test-threads=1
cargo test --test cluster_skeleton --features cluster -- --test-threads=1
```

### `#[ignore]` 慢测与压测

默认 `cargo test` **跳过** 带 `#[ignore]` 的用例; CI 在独立 job 中通过 `--ignored` 运行. 新增慢/压测须使用统一 reason 前缀:

| 前缀 | 含义 | 示例 |
|------|------|------|
| `slow:` | 真实等待或长时间 hold (秒~分钟) | TTL `PX` 过期等待 |
| `stress:` | 大数据集、恶意输入或高吞吐 | TCP 慢 send、大 pipeline |

写法: `#[ignore = "slow: …"]` 或 `#[ignore = "stress: …"]`. **禁止** 裸 `#[ignore]`.

| 测试 | 标签 | test target | CI job |
|------|------|-------------|--------|
| `test_tcp_malicious_slow_send` | stress | `server` | `test-server-stress` |
| `test_tcp_pipeline_large_buffer` | stress | `server` | `test-server-stress` |
| `test_px_expiry_real_wait` | slow | `commands` | `test-commands-slow` |

`test-cluster` 默认跳过上述用例. 本地:

```bash
cargo test --test server --features cluster -- --ignored --test-threads=1
cargo test --test commands --features cluster -- --ignored --test-threads=1
```

### Feature 与 CI

| Feature | 本地验证 | CI |
|---------|----------|-----|
| `cluster` | clippy + 全量 test (主路径) | `test-cluster` 及下游 3 job |
| `monitoring` | `cargo build --features cluster,monitoring` | **无独立 job** |
| default (无 feature) | `cargo build` | **无独立 job** |

### CI 全量 (与 push 门禁一致)

```bash
cargo test --workspace --features cluster -- --test-threads=1
```

### E2E

**pytest (新用例优先)** — 单节点 memory smoke, 需 `redis-cli` + Python 3.10+:

```bash
pip install -r e2e/requirements.txt
pytest e2e/ -v
```

文件放 `e2e/test_*.py`; 文件头 `# @component aikv-{domain}` (与 testviz B2-v1 一致). 慢/压测用 `@pytest.mark.slow` / `@pytest.mark.stress`. 详见 [e2e/README.md](e2e/README.md).

**shell (存量)** — 本地需 `redis-cli`; 多数脚本用 memory 引擎:

```bash
cargo build --release --features cluster
chmod +x e2e/*.sh
./e2e/test_basic.sh
# … 共 21 个 test_*.sh, 见 e2e/README.md
```

**CI `e2e` job**: `e2e/test_cluster_*.sh` (9 个) + `pytest e2e/`. Cluster shell: formation, routing, slots, failover, forget, announce, 3node_routing, data_consistency, aidb_persistence.

| 场景 | 落点 |
|------|------|
| 单节点 TCP smoke | `e2e/test_*.py` (pytest) |
| 多节点集群 / failover | `e2e/test_cluster_*.sh` (shell, 暂不重写) |

Aidb 持久化 roundtrip 由 L1 `cargo test --test storage` 覆盖; 详见 [e2e/README.md](e2e/README.md).

### 示例

| 示例 | 命令 |
|------|------|
| basic | `cargo run --example basic` |
| cluster | `cargo run --features cluster --example cluster` |

见 [examples/README.md](examples/README.md).

## 开发与 PR 规范

1. **TDD (建议)**: 先写测试 → 实现 → 重构.
2. **提交格式**: `type: 中文描述` — `feat`, `fix`, `refactor`, `test`, `docs`, `chore`, `perf`.
3. **修 bug (必带回归测)**: 见下节; `docs:` / doc-only 关闭 ISSUE 可豁免.
4. **用户面向变更**: 更新 [CHANGELOG.md](CHANGELOG.md) 对应版本或 `[Unreleased]`.
5. **PR**: CI + Security 须绿; 相关文档一并更新.

### PR 检查清单

- [ ] `cargo fmt --check` 通过 (或已跑 `./install-hooks.sh`)
- [ ] `cargo clippy --all-targets --features cluster` 无警告 (`RUSTFLAGS='-D warnings'`)
- [ ] `cargo test --workspace --features cluster -- --test-threads=1` 通过
- [ ] 若修 bug: 回归测已添加且本地通过 (见下节)
- [ ] 若改 TCP 压测/TTL 慢测相关: 对应 `--ignored` job 命令通过
- [ ] 用户面向 API/行为变更已写 CHANGELOG
- [ ] 模块文档或根文档已更新 (若适用)

## 回归测试 (必带)

所有 **bugfix PR** (`fix:`、修 ISSUE、行为修正) **必须** 在同一 PR 内附带可复现回归测. **豁免**: 纯文档变更 (`docs:`) 或 doc-only 关闭 ISSUE.

| 规则 | 说明 |
|------|------|
| 同一 PR | 测试与修复同 PR; 建议先红后绿 |
| 注释 | 测试顶部写明 bug 现象、期望行为; 若有 ISSUE 则引用 |
| `@component` | entry 文件加 `//! @component aikv-{domain}` (与 testviz B2-v1 一致) |
| 命名 | 描述性 `test_*`; 见 [`tests/README.md`](tests/README.md) |

### 放置决策

| 场景 | 落点 |
|------|------|
| 单命令/路由 | `tests/modules/command/{域}.rs` |
| TCP/server | `tests/modules/server/` |
| storage/持久化 | `tests/modules/storage/` |
| 集群协议/路由 | `tests/cluster_*.rs` |

示例 (B1.3): ISSUE-002 生产 Options → [`tests/modules/storage/prod_options.rs`](tests/modules/storage/prod_options.rs).

aikv **无** 独立 `tests/regression/` 入口; 回归测放在对应模块或 cluster integration test 中 (与 aidb L4 分工不同, 见 [aidb CONTRIBUTING](../aidb/CONTRIBUTING.md)).

## 相关文档

| 文档 | 内容 |
|------|------|
| [DEPLOYMENT.md](DEPLOYMENT.md) | 构建、feature、CLI、集群部署 |
| [.github/README.md](.github/README.md) | CI / Security 详表 |
| [tests/README.md](tests/README.md) | 测试分层与新增约定 |
| [e2e/README.md](e2e/README.md) | E2E smoke 脚本 |
| [CHANGELOG.md](CHANGELOG.md) | 版本变更记录 |
| [ISSUES.md](ISSUES.md) | 待核实项 |
