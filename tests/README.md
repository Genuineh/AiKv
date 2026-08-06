# 测试说明

## 套件

| 命令 | 说明 |
|------|------|
| `cargo test --test resp` | L1: RESP 编解码 golden + parser roundtrip/边界 |
| `cargo test --test server` | L2: TCP listener + 内联命令 + Phase 9 SET/GET/SELECT/EXPIRE |
| `cargo test --test storage` | L1: MemoryEngine 读写/过期/scan/glob/并发 |
| `cargo test --test commands` | L1: CommandRouter 全命令族 |
| `cargo test --test cluster_info_golden` | L1: CLUSTER INFO Redis 8.8 字段 golden (独立 binary, 进程隔离) |
| `cargo test -- --test-threads=1` | 全量串行 (CI 推荐) |

## 测试写法与范围 (硬性)

对新测 / 改测强制执行. 旧测不要求本次回填.
test-ui 中文一级目录由路径虚拟映射生成 (见 aifactory `test-ui/README`); 分类不靠 `@suite`.
L0 `src/**` → UI「单元测试」; 其余落点见该映射表与下文「落点」.

### 写法

| 位置 | 要求 |
|------|------|
| `tests/` 下新建/改动的集成测文件 | `//! @component aikv-{domain}` + 中文摘要 |
| L0 `src/**` 新建 `#[test]` | 中文 `///`; 不要求 `@component` |
| 每个新增/改动的 `#[test]` | 中文 `///`; 禁止用 `//` 顶替 |
| bug 回归 | `///` 含现象、期望、ISSUE (若有) |
| e2e pytest | `# @component` + 文件/用例 `# @title`; docstring 为 `N. 主谓宾 \| 期望` 剧本; 详见 [e2e/README.md](../e2e/README.md) |
| e2e shell | `# @component aikv-{domain}`; 首条说明性 `#` 作摘要 (仅索引, 不在 test-ui 执行) |

- `#[ignore]`: `slow:` / `stress:`; 禁止裸 ignore
- 除 `@component` 外不加分类标签

### 跨仓边界

| 测什么 | 放哪 |
|--------|------|
| 引擎 / LSM / Raft 存储 | **aidb** |
| RESP / 命令 / TCP / 集群对外 | **本仓 aikv** |
| 多进程 / redis-cli / failover | **e2e/** |

### 落点

| 场景 | 落点 |
|------|------|
| RESP / storage / commands | L1 `tests/modules/...` |
| TCP / listener | L2 `server` |
| 集群协议/路由 | `tests/cluster_*.rs` |
| 回归 | 无独立 L4; 放对应模块或 cluster 测 |
| 单节点协议 smoke | `e2e/function/command/test_*.py` (黑盒被测服务) |
| 多节点 / failover | `e2e/function/failover/`、`e2e/function/migration/` (pytest) |

默认 `--features cluster`; 集成测推荐 `--test-threads=1`.

## L1 RESP (`tests/modules/resp/`)

- `types.rs` — golden encode
- `parser.rs` — roundtrip + 安全边界

## L1 Storage (`tests/modules/storage/`)

- `types.rs` — StoredValue 过期/type_name
- `memory.rs` — MemoryEngine get/set/expire/scan/WRONGTYPE/并发

## L1 Commands (`tests/modules/command/`)

命令族测试见本目录, 按域拆分. 关键入口:

| 文件 | 覆盖 |
|------|------|
| `router.rs` | 未知命令, SELECT 更新 db |
| `string.rs` | SET EX/NX/XX, MGET/MSET, INCR/DECR, APPEND/STRLEN |
| `hash.rs` | HSET/HGET/HDEL/HSETNX/HINCRBY, WRONGTYPE 矩阵 |
| `list.rs` | LPUSH/RPOP/LRANGE 等 |
| `set.rs` | SADD/SMEMBERS 等 |
| `zset.rs` | ZADD/ZRANGE 等 |
| `json.rs` / `atom.rs` | JSON / Atom 命令 |
| `database.rs` | SELECT/DBSIZE/FLUSHDB/FLUSHALL/SWAPDB/MOVE |
| `key.rs` | EXPIRE/TTL/PTTL/PERSIST/EXPIREAT |

完整文件列表以 `tests/modules/command/` 目录为准.

## L2 Server (`tests/modules/server/`)

- `listen.rs` — accept smoke
- `tcp.rs` — PING/ECHO/HELLO + Phase 9 SET/GET/MGET/EXPIRE/SELECT/HSET
- `observability.rs` — 连接 metrics

## 回归测放置

bugfix **必带** 回归测; 详见 [CONTRIBUTING.md §回归测试](../CONTRIBUTING.md#回归测试-必带).

| 场景 | 落点 |
|------|------|
| 命令/路由 | `tests/modules/command/` |
| TCP/server | `tests/modules/server/` |
| storage | `tests/modules/storage/` |
| 集群 | `tests/cluster_*.rs` |

示例: `prod_options.rs` (ISSUE-002). entry 文件加 `//! @component aikv-{domain}`.

## 慢测与压测 (`#[ignore]`)

前缀: `slow:` (真实等待) / `stress:` (大数据集或恶意输入). 详见 [CONTRIBUTING.md](../CONTRIBUTING.md#ignore-慢测与压测).

| 测试 | 标签 | test target | CI job |
|------|------|-------------|--------|
| `test_tcp_malicious_slow_send` | stress | `server` | `test-server-stress` |
| `test_tcp_pipeline_large_buffer` | stress | `server` | `test-server-stress` |
| `test_px_expiry_real_wait` | slow | `commands` | `test-commands-slow` |
| `test_concurrent_write_with_ttl_filter` | stress | `stress_ttl` | 本地 `--ignored` |

`test-cluster` 默认跳过上述用例. 本地:

```bash
cargo test --test server --features cluster -- --ignored --test-threads=1
cargo test --test commands --features cluster -- --ignored --test-threads=1
cargo test --test stress_ttl --features cluster -- --ignored --test-threads=1
```

## 测试隔离约束: CLUSTER_STATE_MGR 全局污染 (2026-08-06)

`CLUSTER_STATE_MGR` 是 OnceLock 一次性全局单例. `tests/commands.rs` binary 内
任何测试若调用 `CLUSTER_STATE_MGR.set(...)`, 会永久污染同进程后续所有命令测试:
临时服务器被误判为集群模式, 命令报 `slot N is not allocated to any group`,
曾导致 MIGRATE 3 测 + atom_json_batch 5 测失败.

约束:
1. `tests/commands.rs` binary (`tests/modules/command/*`) 禁止 set 全局集群状态.
2. 需要集群状态的 golden 测试放独立 test binary (如 `tests/cluster_info_golden.rs`),
   依赖 cargo auto-discovery 的进程隔离.
3. 新增 set 全局状态的测试前, 先确认目标 binary 内没有依赖「未初始化全局」的测试.
