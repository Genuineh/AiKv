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
  pytest e2e/function/ -v
```

在 **test-ui** 中执行时, 顶部环境条会注入 `AIKV_HOST`/`AIKV_PORT`, 无需手设.

### 功能测试划分 (`e2e/function/`)

按 4 个核心维度结构化划分子目录：

| 维度目录 | 内容与文件 |
|---|---|
| `01_single_node/` | **单机 RESP 基础命令与服务诊断** (8 个套件: String, Hash, List, Set, ZSet, Lua, JSON, Protocol/INFO parity) |
| `02_persistence_crash/` | **持久化与崩溃恢复** (3 个套件: RDB 快照, 优雅重启, SIGKILL 强杀 WAL 重放) |
| `03_cluster_migration/` | **集群拓扑与动态槽位迁移** (3 个套件: 集群拓扑不变量, MEET/REPLICATE 扩缩容, SETSLOT/ASK 在线切槽) |
| `04_ha_failover/` | **高可用故障转移** (3 个套件: Smart Client 自动重路由, 主节点故障转移, 故障节点恢复同步) |

```bash
AIKV_HOST=127.0.0.1 AIKV_PORT=6379 \
  pytest e2e/function/ -v
```

### Fixture

| Fixture | 含义 |
|---------|------|
| `svc` | 唯一入口: 连 `AIKV_HOST`/`AIKV_PORT` |

### 显示样板与 test-ui 规范

所有测试文件均遵循 test-ui 元数据标准规范 (`# @component` / `# @title` / docstring)：

| 用途 | 写法 |
|------|------|
| 树-文件中文名 | 文件头 `# @title …` |
| 详情-文件说明 | 模块 docstring (Markdown 渲染) |
| 树/详情-用例标题 | 函数上方 `# @title …` |
| Map 组件关联 | `# @component aikv-server` |

## Layout

```text
e2e/
├── harness/              # 客户端 / 外部连接 / (本机 start_node 仅调试用)
├── conftest.py           # 仅 svc
├── function/             # 4 维度端到端功能测试目录
│   ├── 01_single_node/       # 单机 RESP 基础命令与诊断
│   ├── 02_persistence_crash/ # 持久化落盘与崩溃恢复
│   ├── 03_cluster_migration/ # 集群拓扑与动态槽位迁移
│   └── 04_ha_failover/       # 高可用故障转移
├── pytest.ini
├── requirements.txt
└── old/                  # 旧资产 (不收集)
```

## Notes

- pytest **不收集** `old/`
- 完整覆盖矩阵 / Redis Tcl 对齐
