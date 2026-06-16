# 贡献指南

## 仓库结构

```
src/
├── main.rs          # 入口
├── lib.rs           # 库入口
├── server/          # TCP 服务
├── protocol/        # RESP 编解码
├── command/         # 命令处理器
├── storage/         # 存储适配层
├── persistence/     # AOF + RDB
├── script/          # Lua 脚本引擎
└── cluster/         # Redis 集群协议
```

## 工具链与依赖布局

- [`rust-toolchain.toml`](rust-toolchain.toml): stable + clippy/rustfmt, 与 CI 一致.
- **path 依赖**: `Cargo.toml` 中 `aidb = { path = "../aidb" }`, 本地目录需为 sibling 布局:

```text
parent/
├── aidb/    # wiqun/AiDb
└── aikv/    # wiqun/AiKv
```

CI 会 checkout 同名分支的 `wiqun/AiDb` 并 `ln -sf` 到 `../aidb`; 只改 aikv 时, 远程也应有对应分支的 AiDb.

推送前安装 pre-commit (fmt + cluster clippy; 不含 test):

```bash
./install-hooks.sh
```

## 构建与测试

```bash
cargo build
cargo test -- --test-threads=1
cargo test --test storage --test commands -- --test-threads=1
cargo test --test resp --test server -- --test-threads=1
cargo clippy -- -D warnings
cargo fmt --check
cargo run -- --bind 127.0.0.1:6379

# 集群测试
cargo build --features cluster
cargo test --features cluster -- --test-threads=1

# E2E 测试 (需 redis-cli)
cargo build --release
./e2e/test_basic.sh
./e2e/test_datatypes.sh
./e2e/test_json.sh
./e2e/test_lua.sh
./e2e/test_persistence.sh
```

## 开发流程

1. **TDD**: 先写测试 (RED) → 实现 (GREEN) → 重构 (IMPROVE)
2. **覆盖率**: 保持 80%+
3. **提交格式**: `type: description` — feat, fix, refactor, test, docs, chore, perf
4. **PR**: CI 必须通过 (`ci.yml` + `security.yml`)
5. **E2E**: 发版前运行 `e2e/test_*.sh`

### CI 任务说明

| Job | 说明 |
|-----|------|
| `test-cluster` | fmt → clippy (cluster) → 常规测试 |
| `test-server-stress` | `--test server -- --ignored` — TCP 慢发送 / 大 pipeline 压测 |
| `test-commands-slow` | `--test commands -- --ignored` — 真实等待的 TTL 慢测 |
| `e2e` | release 构建 + `e2e/test_cluster_*.sh` (依赖 `test-cluster` 通过) |

带 `#[ignore]` 的用例默认不进 `test-cluster`; 上表两个 job 按 test target 补跑, 命名与 aidb 的 `test-default` / `test-cluster` 同一思路.

## 命令参考

| 参数 | 默认值 | 说明 |
|------|--------|------|
| `--bind` | 127.0.0.1 | 监听地址 |
| `--port` | 6379 | 监听端口 |
| `--engine` | memory | 存储引擎: `memory` / `aidb` |
| `--data-dir` | - | AiDb 数据目录 |
| `--metrics-port` | 9191 | Prometheus 端口 |
| `--cluster-mode` | - | 启用集群模式 |

详见 [DEPLOYMENT.md](DEPLOYMENT.md).
