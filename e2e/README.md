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
| `dut` | **黑盒 DUT** (须 `WIKV_HOST`/`WIKV_PORT`; test-ui 环境条自动注入) |
| `memory_node` | 本机 `--engine memory`; EXTERNAL 时改连外部 (兼容旧冒烟) |
| `aidb_node` | 本机 `--engine aidb` + 临时 data-dir; EXTERNAL 时 skip |
| `aikv_binary` | `target/release/aikv` (缺失则构建; EXTERNAL 时不强制) |

黑盒 (推荐, SET 等命令族):

```bash
# 先用 test-ui / aifactory/scripts 部署单机或集群, 再:
WIKV_HOST=127.0.0.1 WIKV_PORT=6379 \
  .venv-e2e/bin/pytest e2e/test_set.py -v
```

在 **test-ui** 中执行时, 会自动把顶部环境条的地址/端口注入为 `WIKV_HOST`/`WIKV_PORT`, 无需手设.

同一套 `test_set.py` 可对单机与集群各跑一遍 (拓扑由部署决定; 客户端自动跟 MOVED).

冒烟仍可用本机 spawn:

```bash
.venv-e2e/bin/pytest e2e/test_smoke_ping.py -v
```

或连外部:

```bash
WIKV_EXTERNAL_DUT=1 WIKV_HOST=127.0.0.1 WIKV_PORT=6379 \
  .venv-e2e/bin/pytest e2e/test_smoke_ping.py::test_ping_memory -v
```

## Layout

```text
e2e/
├── harness/           # 日志 / 二进制 / 进程 / 客户端 / 单机节点
├── conftest.py        # dut / memory_node / aidb_node
├── test_smoke_ping.py # 本机或 EXTERNAL 冒烟
├── test_set.py        # 黑盒 SET/GET (仅 dut)
├── pytest.ini         # 忽略 old/; pythonpath=.
├── requirements.txt
└── old/               # 旧资产 (不收集)
```

## test-ui 显示约定 (pytest)

样板: [`test_set.py`](test_set.py).

| 用途 | 写法 |
|------|------|
| 树-文件中文名 | 文件头 `# @title SET 命令` |
| 详情-文件说明 | 模块 docstring |
| 树/详情-用例 | 函数 docstring (首行作树标题) |
| Map | `# @component aikv-…` (不变) |

详情「路径」形如 `aikv/e2e/test_set.py`. 目录级说明暂不扫描 (留空).

## Notes

- pytest **不收集** `old/`
- 产物默认 `target/release/aikv`; 若设置了 `CARGO_TARGET_DIR`, 请确保二进制仍出现在该路径或先本地安装到 `target/release/aikv`
- **推荐路径**: 先用 test-ui / `aifactory/scripts` 部署 DUT, 再 `WIKV_EXTERNAL_DUT=1` 跑黑盒用例; `memory_node` 仅适合轻量冒烟
- 集群编排与命令族用例将分批用 pytest 重写; 参考 `old/`
