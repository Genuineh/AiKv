# 测试说明

## 套件

| 命令 | 说明 |
|------|------|
| `cargo test --test resp` | L1: RESP 编解码 golden + parser roundtrip/边界 |
| `cargo test --test server` | L2: TCP listener + 内联命令 + Phase 9 SET/GET/SELECT/EXPIRE |
| `cargo test --test storage` | L1: MemoryEngine 读写/过期/scan/glob/并发 |
| `cargo test --test commands` | L1: CommandRouter String/Hash/Database/Key 命令 |
| `cargo test -- --test-threads=1` | 全量串行 (CI 推荐) |

## L1 RESP (`tests/modules/resp/`)

- `types.rs` — golden encode
- `parser.rs` — roundtrip + 安全边界

## L1 Storage (`tests/modules/storage/`)

- `types.rs` — StoredValue 过期/type_name
- `memory.rs` — MemoryEngine get/set/expire/scan/WRONGTYPE/并发

## L1 Commands (`tests/modules/command/`)

- `router.rs` — 未知命令, SELECT 更新 db
- `string.rs` — SET EX/NX/XX, MGET/MSET, INCR/DECR, APPEND/STRLEN, WRONGTYPE
- `hash.rs` — HSET/HGET/HDEL/HSETNX/HINCRBY, 全命令 WRONGTYPE 矩阵
- `database.rs` — SELECT/DBSIZE/FLUSHDB/FLUSHALL/SWAPDB/MOVE
- `key.rs` — EXPIRE/TTL/PTTL/PERSIST/EXPIREAT, TTL 覆盖

## L2 Server (`tests/modules/server/`)

- `listen.rs` — accept smoke
- `tcp.rs` — PING/ECHO/HELLO + Phase 9 SET/GET/MGET/EXPIRE/SELECT/HSET
- `observability.rs` — 连接 metrics

## 可选慢测 (#[ignore])

| 测试 | test target | CI job |
|------|-------------|--------|
| `test_tcp_malicious_slow_send` | `server` | `test-server-stress` |
| `test_tcp_pipeline_large_buffer` | `server` | `test-server-stress` |
| `test_px_expiry_real_wait` | `commands` | `test-commands-slow` |

`test-cluster` 默认跳过上述用例. 本地:

```bash
cargo test --test server --features cluster -- --ignored --test-threads=1
cargo test --test commands --features cluster -- --ignored --test-threads=1
```
