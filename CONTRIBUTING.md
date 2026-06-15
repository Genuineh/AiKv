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

# Phase 9 验收
python3 ../WiQunTools/scripts/acceptance.py ../WiQunTools/scripts/storage-acceptance.json

# Phase 8 回归
python3 ../WiQunTools/scripts/acceptance.py ../WiQunTools/scripts/resp-acceptance.json

# Phase 10 回归
python3 ../WiQunTools/scripts/acceptance.py ../WiQunTools/scripts/extended-acceptance.json

# Phase 11 JSON / Lua 验收
python3 ../WiQunTools/scripts/acceptance.py ../WiQunTools/scripts/json-acceptance.json
python3 ../WiQunTools/scripts/acceptance.py ../WiQunTools/scripts/lua-acceptance.json

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
4. **PR**: CI 必须通过
5. **E2E**: 发版前运行 `e2e/test_*.sh`

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
