# AiKv E2E Tests (Python / pytest)

终态目标: **全部使用 pytest**. 旧 shell / 旧 pytest 已迁入 [`old/`](old/) (**仅参考, 不维护**).

## Prerequisites

- `redis-cli` on PATH
- Rust toolchain (`cargo build --release --features cluster`)
- [uv](https://github.com/astral-sh/uv) for the Python env

## Setup (uv)

```bash
cd aikv
uv venv .venv-e2e
uv pip install -r e2e/requirements.txt --python .venv-e2e
```

## Run

从仓库根:

```bash
.venv-e2e/bin/pytest e2e/ -v
# 或
uv run --python .venv-e2e pytest e2e/ -v
```

日志: `WIKV_E2E_LOG=DEBUG` 可见起停细节.

### Fixtures

| Fixture | 含义 |
|---------|------|
| `memory_node` | 本机 `--engine memory` (默认) |
| `aidb_node` | 本机 `--engine aidb` + 临时 data-dir |
| `aikv_binary` | `target/release/aikv` (缺失则构建) |

外部 DUT (已手起服务):

```bash
WIKV_EXTERNAL_DUT=1 WIKV_HOST=127.0.0.1 WIKV_PORT=6379 \
  .venv-e2e/bin/pytest e2e/test_smoke_ping.py::test_ping_memory -v
```

`aidb_node` 在 EXTERNAL 模式下 skip.

## Layout

```text
e2e/
├── harness/           # 日志 / 二进制 / 进程 / 客户端 / 单机节点
├── conftest.py
├── test_smoke_ping.py
├── pytest.ini         # 忽略 old/; pythonpath=.
├── requirements.txt
└── old/               # 旧资产 (不收集)
```

## Notes

- pytest **不收集** `old/`
- 产物默认 `target/release/aikv`; 若设置了 `CARGO_TARGET_DIR`, 请确保二进制仍出现在该路径或先本地安装到 `target/release/aikv`
- **推荐路径**: 先用 testviz / `aifactory/scripts` 部署 DUT, 再 `WIKV_EXTERNAL_DUT=1` 跑黑盒用例; `memory_node` 仅适合轻量冒烟
- 集群编排与命令族用例将分批用 pytest 重写; 参考 `old/`
