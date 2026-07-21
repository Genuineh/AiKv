# AiKv E2E Tests (Python / pytest)

黑盒客户端验收: **用例只连已部署 DUT**, 不 spawn 进程、不选引擎.  
部署由 test-ui 环境条 / `aifactory/scripts` / 手工完成 (实验室约定 aidb).

旧 shell / 旧 pytest 在 [`old/`](old/) (**仅参考, 不维护**).

## Prerequisites

- 已部署的单机 (或集群) aikv, 地址可知
- `redis-cli` on PATH (逃生口 / 会话检查)
- [uv](https://github.com/astral-sh/uv) 管理 Python 环境

## Setup (uv)

```bash
cd aikv
uv venv .venv-e2e
uv pip install -r e2e/requirements.txt --python .venv-e2e
```

## Run

先部署 DUT, 再:

```bash
WIKV_HOST=127.0.0.1 WIKV_PORT=6379 \
  .venv-e2e/bin/pytest e2e/ -v
```

在 **test-ui** 中执行时, 顶部环境条会注入 `WIKV_HOST`/`WIKV_PORT`, 无需手设.

### 功能测试 (`e2e/function/`)

树: `aikv → 端到端 → 功能测试 → 文件 → 用例`.

| 文件 | 内容 |
|------|------|
| `test_smoke.py` | PING |
| `test_commands.py` | STRING / HASH |
| `test_correctness.py` | 覆盖写 / 缺键 / 双连接 |
| `test_wrongtype.py` | WRONGTYPE 后仍可用 |
| `test_concurrency.py` | 双客户端并发 |
| `test_persist.py` | 写探针 + 重启后再读 |

```bash
WIKV_HOST=127.0.0.1 WIKV_PORT=6379 \
  .venv-e2e/bin/pytest e2e/function/ -v
```

持久 / 重启编排 (客户端不负责起停):

1. 跑 seed: `test_persist_seed_graceful`
2. **部署侧**优雅停再起同一 data-dir
3. `WIKV_E2E_AFTER_RESTART=1` 再跑 `test_persist_after_graceful_restart`
4. 跑 `test_persist_seed_kill` → **部署侧** `kill -9` 再起
5. `WIKV_E2E_AFTER_KILL=1` 再跑 `test_persist_after_kill`

未设上述 env 时, 对应「再读」用例会 `skip` (不算失败).

### Fixture

| Fixture | 含义 |
|---------|------|
| `dut` | 唯一入口: 连 `WIKV_HOST`/`WIKV_PORT` |

### 显示样板

[`test_set.py`](test_set.py) 仅作 test-ui 元数据写法样例 (`# @title` / docstring), 挂在「端到端」根下.

| 用途 | 写法 |
|------|------|
| 树-文件中文名 | `# @title …` |
| 详情-文件说明 | 模块 docstring |
| 树/详情-用例 | 函数 docstring (首行) |
| Map | `# @component aikv-…` |

## Layout

```text
e2e/
├── harness/              # 客户端 / 外部连接 / (本机 start_node 仅调试用)
├── conftest.py           # 仅 dut
├── function/             # 功能测试 (UI: 功能测试)
├── test_set.py           # 显示样板
├── pytest.ini
├── requirements.txt
└── old/                  # 旧资产 (不收集)
```

## Notes

- pytest **不收集** `old/`
- 完整覆盖矩阵 / Redis Tcl 对齐 **后置**
