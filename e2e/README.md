# AiKv E2E Tests

End-to-end smoke tests against a running `aikv` **release** binary via `redis-cli`.

**新用例优先 pytest**; 存量 **shell** 脚本保留维护, CI 集群场景仍跑 `test_cluster_*.sh`.

## Prerequisites

- `redis-cli` on PATH (Redis tools package)
- Rust toolchain for `cargo build --release`
- Python 3.10+ and pytest (pytest 用例):

  ```bash
  python3 -m venv .venv-e2e && .venv-e2e/bin/pip install -r e2e/requirements.txt
  # Debian/Ubuntu PEP 668 环境需 venv; CI 直接 pip install
  ```

## Run

### pytest (推荐, 新 E2E)

从仓库根目录:

```bash
cd aikv
python3 -m venv .venv-e2e && .venv-e2e/bin/pip install -r e2e/requirements.txt
.venv-e2e/bin/pytest e2e/ -v
```

单进程即可 (fixture 使用随机端口). 慢测/压测 marker 与 Rust 集成测语义一致, 默认全跑 (当前示例无 slow/stress).

### shell (存量)

```bash
cd aikv
chmod +x e2e/*.sh
./e2e/test_basic.sh
./e2e/test_datatypes.sh
./e2e/test_ext.sh
./e2e/test_json.sh
```

Environment overrides:

- `WIKV_HOST` (default `127.0.0.1`)
- `WIKV_PORT` — 仅 shell `utils.sh` 单节点脚本; pytest fixture 内随机端口

## Layout

```shell
e2e/
├── conftest.py          # pytest fixtures (build, memory_server)
├── lib/                 # redis-cli / server helpers (非 testviz 索引)
├── test_*.py            # 新 E2E (testviz 扫描 # @component)
├── requirements.txt     # pytest
├── utils.sh             # shell 共享 helper
└── test_*.sh            # 存量 shell E2E (21 个)
```

## Notes

- pytest 与 shell 均构建 `target/release/aikv` 并启动 ephemeral **memory** 引擎 (`--engine memory`).
- **AiDb 重启持久化** 由 L1 覆盖: `cargo test --test storage test_aidb` (roundtrip + restart + adapter list/flushdb). 可选手动:

  ```bash
  DATA=/tmp/aikv-e2e
  cargo run --release -- --bind 127.0.0.1:6380 --engine aidb --data-dir "$DATA"
  redis-cli -p 6380 SET k v
  # 重启同命令后 GET k
  ```

- 集群复杂场景暂用 shell (`test_cluster_*.sh`); CI `e2e` job 跑 cluster shell + pytest smoke.
- CI images without `redis-cli` should install `redis-tools` or skip these tests.
