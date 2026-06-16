# AiKv — AI 助手指南

## 项目是什么

**AiKv** 是用 Rust 实现的 **Redis RESP 兼容 KV 服务** (bin + lib).

- **对外**: RESP2/RESP3、Redis 命令、Redis Cluster (MOVED/ASK、CLUSTER 子命令、slot 迁移等).
- **对内**: 不实现 LSM/Raft; 持久化与共识委托 **AiDb** (`Cargo.toml`: `aidb = { path = "../aidb" }`, 集群时 `cluster = ["aidb/cluster"]`).
- **存储**: `MemoryEngine` (内存) 或 `AiDbEngine` (WAL + LSM); 生产集群推荐 `--engine aidb` (当前只开发 AiDbEngine, 内存只是留作未来拓展选项).

**与 AiDb**: Redis 协议与命令在 AiKv; 数据平面与 Multi-Raft 在 AiDb. 本地需 sibling 布局 `../aidb`; CI checkout 同名分支 AiDb 并 link.

## 架构要点

- **客户端兼容**: 标准 RESP 数组; BulkString 二进制安全 (不做 UTF-8 假设).
- **Cluster**: 16384 槽、CRC16、`{hash tag}`、MOVED/ASK、在线 slot 迁移; 与 Redis Cluster 客户端协作.
- **分层**: `protocol` → `server` → `command` → `storage` (memory | AiDb) → `cluster` 协议层 → AiDb MetaRaft / MultiRaft.
- **数据流**:

```text
Redis Client → TCP/RESP/CommandRouter/ClusterRouter
            → KvStorage (MemoryEngine | AiDbEngine)
            → aidb (DB | MetaRaft | MultiRaft | Router)
```

- 存在性 / 删除等语义须走 AiDb `DB::get` 与 tombstone 规则, 不在 storage adapter 绕过.
- 同一 codebase; 开发与 CI 以 `--features cluster` 为主.

## 开发与 CI

流程见 `[.github/README.md](.github/README.md)`.

```bash
# 确保 ../aidb 存在
./install-hooks.sh
export RUSTFLAGS='-D warnings'
cargo fmt --check
cargo clippy --all-targets --features cluster
cargo test --workspace --features cluster
```

慢测 (CI: `test-server-stress`, `test-commands-slow`):

```bash
cargo test --test server --features cluster -- --ignored --test-threads=1
cargo test --test commands --features cluster -- --ignored --test-threads=1
```

## 已知限制

**集群**

- `CLUSTER FORGET` / `DEL_REPLICA` 等行为见 CHANGELOG; FORCE 路径有 guard.
- 数据面 gRPC: `rpc_port + --cluster-data-port-offset` (默认 10000).

**持久化 (未做或延后)**

- memory 引擎 AOF; 标准 `dump.rdb` 暂不实现持久化; `CONFIG REWRITE` — 生产推荐 `--engine aidb`.

**协议**

- 不支持 telnet 式非数组内联命令 (`PING\r\n`).

## 进一步阅读

- [README.md](README.md)
- [ARCHITECTURE.md](ARCHITECTURE.md)
- [.github/README.md](.github/README.md)

