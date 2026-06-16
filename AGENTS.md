# AiKv — AI 助手指南

Redis RESP 兼容 KV 服务 (Rust bin crate, v0.10.x). 存储引擎: 内存或 path 依赖的 **AiDb**; 集群模式委托 AiDb MetaRaft / Multi-Raft.

## 依赖布局

```text
parent/
├── aidb/    # path = "../aidb"
└── aikv/
```

CI 会 checkout 同名分支的 `wiqun/AiDb` 并 link 为 `../aidb`. 只改 aikv 时远程 AiDb 需有对应分支.

## 架构要点

- **协议**: RESP2/3, tokio TCP (`src/protocol/`, `src/server/`)
- **命令**: `CommandRouter` — String/Hash/List/Set/ZSet/Key/JSON/Lua/阻塞命令/集群命令
- **存储**: `MemoryEngine` / `AiDbEngine` (`src/storage/`)
- **集群** (`cluster` feature): MOVED/ASK、Gossip、CLUSTER 子命令; 底层 AiDb 集群
- **持久化**: SAVE/BGSAVE 等兼容命令 ✅; memory 引擎 AOF、标准 RDB dump ❌ (AiDb 引擎用 WAL+Checkpoint)

## 开发与 CI

详见 [`.github/README.md`](.github/README.md). 摘要:

```bash
# 确保 ../aidb 存在
./install-hooks.sh
export RUSTFLAGS='-D warnings'
cargo fmt --check
cargo clippy --all-targets --features cluster
cargo test --workspace --features cluster
```

- CI job: `test-cluster` → `test-server-stress` / `test-commands-slow` / `e2e`
- pre-commit **不跑** test; 慢测见 [`tests/README.md`](tests/README.md) 的 `#[ignore]` 节

本地 E2E (需 redis-cli):

```bash
cargo build --release --features cluster
bash e2e/test_basic.sh
bash e2e/test_cluster_formation.sh   # 集群
```

## 模块验证

```bash
cargo test --test resp -- --test-threads=1
cargo test --test server -- --test-threads=1
cargo test --test commands -- --test-threads=1
cargo test --features cluster -- --test-threads=1
# 慢测 (CI: test-server-stress / test-commands-slow)
cargo test --test server --features cluster -- --ignored --test-threads=1
cargo test --test commands --features cluster -- --ignored --test-threads=1
```

## 编码约束

1. 生产代码禁止 `unwrap()` / `expect()` (测试除外)
2. 单文件 ≤ 800 行, 单函数 ≤ 50 行, 嵌套 ≤ 4 层
3. TDD: RED → GREEN → REFACTOR
4. BulkString 二进制安全, 编码不做 UTF-8 假设
5. E2E 脚本在 `e2e/`, 发版前跑相关 `test_*.sh`

## 已知限制

### 集群

- `CLUSTER FORGET`: 默认 graceful; `FORCE` 需节点不在唯一副本 group
- `CLUSTER DEL_REPLICA`: 先从 data group 移除副本再 FORGET
- **数据面 gRPC**: 默认 `rpc_port + 10000`, `--cluster-data-port-offset` 可配; 启动校验 `rpc_port ≤ 65535 - offset`

### 持久化 (延后)

- memory 引擎 AOF; 标准 RDB; `CONFIG REWRITE` — 生产推荐 `--engine aidb`

### 不做

- 非 RESP 数组的内联命令 (telnet 式 `PING\r\n`)

## 文档索引

| 文档 | 用途 |
|------|------|
| [README.md](README.md) | 快速开始与 CLI |
| [CONTRIBUTING.md](CONTRIBUTING.md) | 贡献与 CI job 表 |
| [ARCHITECTURE.md](ARCHITECTURE.md) | 架构 |
| [DEPLOYMENT.md](DEPLOYMENT.md) | 部署与集群参数 |
| [docs/superpowers/](docs/superpowers/) | 功能设计与计划 |
| [tests/README.md](tests/README.md) | 测试套件 |
| [e2e/README.md](e2e/README.md) | E2E 脚本 |
| [.github/README.md](.github/README.md) | CI / hook 流程 |
