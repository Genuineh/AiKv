# AiKv E2E Tests (Python / pytest)

黑盒客户端验收: **用例只连已部署 被测服务**, 不 spawn 进程、不选引擎.  
部署由 test-ui 环境条 / [`aifactory/scripts`](../aifactory/scripts/) (`up-single.sh`, `up-cluster.sh`) / [`aifactory/benchmark/aikv`](../aifactory/benchmark/aikv/) 对照压测 compose / 手工 `cargo run` 完成 (实验室约定 aidb).

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

先部署 被测服务, 再:

```bash
AIKV_HOST=127.0.0.1 AIKV_PORT=6379 \
  .venv-e2e/bin/pytest e2e/ -v
```

在 **test-ui** 中执行时, 顶部环境条会注入 `AIKV_HOST`/`AIKV_PORT`, 无需手设.

### 功能测试 (`e2e/function/`)

树: `aikv → 端到端 → 功能测试 → 文件 → 用例`.

| 文件 | 内容 |
|------|------|
| `test_commands.py` | Smoke (PING); String CRUD; INFO / CLUSTER INFO (Redis 8.8 字段); CLUSTER NODES L1 拓扑不变量 (非集群 skip) |
| `test_types.py` | Hash / List / Set / ZSet 浅 CRUD |
| `test_correctness.py` | 覆盖写 / 缺键 / 双连接 |
| `test_wrongtype.py` | WRONGTYPE 后仍可用 |
| `test_concurrency.py` | 双客户端并发 |
| `test_persist.py` | 写探针 + 重启后再读 |

```bash
AIKV_HOST=127.0.0.1 AIKV_PORT=6379 \
  .venv-e2e/bin/pytest e2e/function/ -v
```

持久 / 重启编排 (客户端不负责起停):

1. 跑 seed: `test_persist_seed_graceful`
2. **部署侧**优雅停再起同一 data-dir
3. `AIKV_E2E_AFTER_RESTART=1` 再跑 `test_persist_after_graceful_restart`
4. 跑 `test_persist_seed_kill` → **部署侧** `kill -9` 再起
5. `AIKV_E2E_AFTER_KILL=1` 再跑 `test_persist_after_kill`

未设上述 env 时, 对应「再读」用例会 `skip` (不算失败).

### Fixture

| Fixture | 含义 |
|---------|------|
| `svc` | 唯一入口: 连 `AIKV_HOST`/`AIKV_PORT` |

### 显示样板

[`test_set.py`](test_set.py) 仅作 test-ui 元数据写法样例 (`# @title` / docstring), 挂在「端到端」根下.

| 用途 | 写法 |
|------|------|
| 树-文件中文名 | 文件头 `# @title …` |
| 详情-文件说明 | 模块 docstring (无编号步骤时 Markdown 渲染) |
| 树/详情-用例标题 | 函数上方 `# @title …` |
| 详情-用例说明 | 编号步骤剧本: `N. 主谓宾动作 \| 期望` (含主语如 key/连接/被测服务; 也兼容 `→ 期望`); test-ui 渲染为步骤条 |
| Map | `# @component aikv-…` |

用例 docstring 示例:

```python
# @title String
def test_string_crud(svc):
    """String 的增删改查.

    1. 将 key SET 为 "hello" | 成功
    2. GET 该 key | 返回 "hello"
    3. 将同一 key SET 为 "world" | 成功
    4. GET 该 key | 返回 "world"
    5. DEL 该 key | 删除数 1
    6. GET 该 key | 缺键 (None)
    """
```


## Layout

```text
e2e/
├── harness/              # 客户端 / 外部连接 / (本机 start_node 仅调试用)
├── conftest.py           # 仅 svc
├── function/             # 功能测试 (UI: 功能测试)
├── test_set.py           # 显示样板
├── pytest.ini
├── requirements.txt
└── old/                  # 旧资产 (不收集)
```

## Notes

- pytest **不收集** `old/`
- 完整覆盖矩阵 / Redis Tcl 对齐 **后置**
