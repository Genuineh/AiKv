# AiKv E2E Tests (Python / pytest)

黑盒客户端验收: 大多数用例仅连接已部署的被测服务 (单机或集群拓扑均可);
`crash/test_crash.py` 与 `crash/test_restart.py` 会在本机自建临时进程并控制其生命周期.

## 环境前置

- 已部署并就绪的 AiKv 服务 (单机或集群), 地址端口可知
- `redis-cli` 工具已安装在系统 `PATH` 中 (自动门禁检查)
- 使用 [uv](https://github.com/astral-sh/uv) 管理 Python 虚拟环境

### 候选 binary

`crash/test_crash.py` 与 `crash/test_restart.py` 需要可执行的 AiKv binary.
`AIKV_BIN` 存在时优先使用
该路径, 且路径必须是普通文件并具有执行权限; 路径无效时直接失败, 不会回退到默认
binary. 例如使用尚未 publish 的本地候选 binary:

```bash
AIKV_BIN=/path/to/aikv pytest e2e/function/crash/ -v
```

未设置 `AIKV_BIN` 时, harness 依次确认 `target/release/aikv` 是否为可执行文件;
若不存在则执行 `cargo build --release --features cluster`, 构建后再次校验输出文件.

## 环境搭建 (uv)

```bash
cd aikv
uv venv .venv-e2e
uv pip install -r e2e/requirements.txt --python .venv-e2e
```

## 环境配置 (.env)

`aikv/e2e` 支持本地配置文件解耦. 可通过复制 `.env.example` 生成本地 `.env` 文件:

```bash
cp e2e/.env.example e2e/.env
```

`svc` 节点的被测服务地址解析优先级如下:
1. **进程环境变量** (如终端显示执行 `AIKV_PORT=6380 pytest` 或 console 自动注入)
2. **本地 `e2e/.env` 文件** (持久化记录常用测试节点地址)
3. **默认回退值** (`127.0.0.1:6379`)

## 测试运行

直接运行 pytest (自动读取 `.env` 或默认连 127.0.0.1:6379):

```bash
pytest e2e/function/ -v
```

或显式指定地址覆盖运行:

```bash
AIKV_HOST=127.0.0.1 AIKV_PORT=6379 \
  pytest e2e/function/ -v
```

## 功能测试划分 (`e2e/function/`)

测试用例按 4 个核心维度结构化划分子目录:

| 维度目录 | 说明 | 包含脚本 |
|---|---|---|
| `command/` | **命令与协议 (单机/集群通用)** | `test_proto.py`, `test_string.py`, `test_list.py`, `test_hash.py`, `test_set.py`, `test_zset.py`, `test_lua_tx.py`, `test_json.py` |
| `crash/` | **持久化与崩溃恢复** | `test_crash.py`, `test_restart.py`, `test_rdb.py` |
| `migration/` | **集群拓扑与动态槽位迁移** | `test_nodes.py`, `test_scale.py`, `test_migration.py` |
| `failover/` | **高可用故障转移** | `test_reconnect.py`, `test_node_fail.py`, `test_failover.py` |

执行目标说明:

- **连接外部拓扑**: `command/` 全部用例, `crash/test_rdb.py`, `migration/` 全部用例,
  以及 `failover/` 全部用例. 故障转移用例按现有六节点 Docker 端口约定运行.
- **自建本机进程**: `crash/test_crash.py` 与 `crash/test_restart.py` 使用 harness
  启动临时单机进程, 结束后清理数据目录. 这两个用例不接收 `svc` fixture 参数,
  数据校验针对自建节点.

### Fixture

| Fixture | 含义 |
|---------|------|
| `svc` | 唯一入口: 连接 `AIKV_HOST`/`AIKV_PORT`, 在交付用例前自动执行 `FLUSHALL` 前置清库 |

### console 编写规范与示例

所有测试脚本均遵循 console 元数据标准规范 (`# @component` / `# @title` / docstring):

| 用途 | 语法/位置 |
|------|------|
| 树-文件中文名 | 文件头部 `# @title …` |
| 详情-文件说明 | 模块 docstring (Markdown 渲染) |
| 树/详情-用例标题 | 函数上方 `# @title …` |
| Map 组件关联 | `# @component aikv-server` |

#### 示例代码

```python
# @component aikv-server
# @title SET 命令
"""黑盒 SET/GET: 仅连预部署外部被测服务 (单机或集群拓扑均可)."""

from __future__ import annotations


# @title SET/GET
def test_set_get(svc):
    """SET/GET 样板.

    1. 将 key k1 SET 为 "v1" | 成功
    2. GET key k1 | 返回 "v1"
    """
    c = svc.client()
    assert c.set("{test}k1", "v1") is True
    assert c.get("{test}k1") == "v1"
```

## 目录布局

```shell
e2e/
├── harness/        # 底层测试脚手架 (Node, RedisClient, ClusterNodes)
├── conftest.py     # 全局入口 (前置自动 FLUSHALL 清库)
├── function/       # 4 维度端到端功能测试目录
│   ├── command/    # 命令与协议 (单机/集群通用)
│   ├── crash/      # 持久化落盘与崩溃恢复
│   ├── migration/  # 集群拓扑与动态槽位迁移
│   └── failover/   # 高可用故障转移
├── pytest.ini
└── requirements.txt
```
