# AiKv 架构

## 目录结构

```
src/
├── main.rs              # 入口 (CLI 参数解析 + Server::run)
├── lib.rs               # 库入口
├── server/              # TCP 服务层
│   ├── mod.rs           # Server 启动 / 连接管理
│   ├── connection.rs    # 单连接读写循环
│   ├── monitor.rs       # MONITOR 广播
│   └── metrics_server.rs # Prometheus HTTP 端点 (feature-gated)
├── protocol/            # RESP 协议层
│   ├── mod.rs
│   ├── parser.rs        # RESP2 + RESP3 解析器
│   └── value.rs         # RespValue 编码/解码
├── command/             # 命令处理器
│   ├── mod.rs
│   ├── router.rs        # CommandRouter (路由表 + 执行)
│   ├── key_lock.rs      # 多 key 字典序加锁
│   ├── string.rs        # GET/SET/INCR/DECR 等
│   ├── hash.rs          # HSET/HGET/HDEL 等
│   ├── list.rs          # LPUSH/LPOP/LRANGE 等
│   ├── set.rs           # SADD/SREM/SMEMBERS 等
│   ├── zset.rs          # ZADD/ZRANGE/ZSCORE 等
│   ├── keys.rs          # KEYS/SCAN/RENAME/TYPE 等
│   ├── server_cmd.rs    # INFO/TIME/CONFIG/CLIENT 等
│   ├── slowlog.rs       # 慢查询日志
│   ├── latency.rs       # LATENCY 直方图
│   ├── command_registry.rs # 命令元数据
│   ├── json_exec.rs     # JSON 命令 (Lua 路径)
│   └── cluster_commands.rs # CLUSTER 命令
├── storage/             # 存储适配层
│   ├── mod.rs           # KvStorage trait
│   ├── value.rs         # StoredValue / ValueType
│   ├── memory_engine.rs # MemoryEngine (HashMap + 过期)
│   └── adapter.rs       # AiDbEngine (spawn_blocking)
├── persistence/         # 持久化
│   ├── mod.rs
│   └── persistence_cmd.rs # SAVE/BGSAVE/LASTSAVE
├── script/              # Lua 脚本引擎
│   ├── mod.rs           # EVAL/EVALSHA/SCRIPT 处理
│   ├── sandbox.lua      # 沙箱 (StdLib 裁剪)
│   └── json_exec.rs     # JSON 桥接
├── cluster/             # 集群协议层
│   ├── mod.rs           # ClusterStateManager
│   ├── router.rs        # ClusterRouter (MOVED/ASK/CLUSTERDOWN)
│   ├── connection_state.rs # per-connection asking/readonly
│   └── gossip.rs        # GossipState 拓扑缓存
└── metrics.rs           # ServerMetrics + Prometheus 指标
```

## 数据流

### 命令执行路径

```
TCP 数据到达
  → Connection::read_loop (tokio async)
  → RespParser::parse (RESP2/RESP3, pipeline)
  → 内联命令? (PING/ECHO/HELLO/QUIT/READONLY/ASKING)
    → 直接处理, 回复
  → CommandRouter::execute/execute_with_client
    → 参数解析 → 命令 handler
      → KvStorage::get/set/delete...
    → RESP 编码回复
  → 记录 metrics (commands_total, duration)
  → 慢查询检查 (>100ms → SLOWLOG)
```

### 集群初始化路径

```
init_cluster (main.rs)
  → DB::open (WAL 必须启用)
  → MetaRaftNode::new + initialize/bootstrap (控制平面, group_id=0)
  → tokio::spawn(meta_raft.start_server) (MetaRaft gRPC, rpc_addr 端口)
  → MultiRaftNode::new_with_lifecycle + Arc::new
  → multi_raft.start_lifecycle_with_data (后台 task 自动创建/销毁数据 Group)
  → tokio::spawn(multi_raft.start) (数据面 gRPC, rpc_port + 10000)
  → MembershipCoordinator + ClusterStateManager + GossipState 初始化
```

### 集群命令执行路径

```
CLUSTER 命令 / 跨槽命令
  → ClusterRouter::decide (无锁判断)
    → 本地 Leader: 正常执行
    → 非本地: -MOVED <slot> <host:port>
    → 迁移中: -ASK <slot> <host:port> (+ ASKING flag)
    → 未分配: -CLUSTERDOWN
    → CROSSSLOT: -CROSSSLOT

CLUSTER ADDSLOTS slot [...]
  → CLUSTER_STATE_MGR.get()
  → 查找本节点所属 Group
    → 如无 Group: MetaRequest::CreateGroup (自动创建)
  → MetaRequest::AssignSlots (控制平面共识)
  → LifecycleManager::tick 后续创建数据 Group
```

### Lua 脚本执行路径

```
EVAL script numkeys key ...
  → mlua Lua 5.4 沙箱
  → redis.call/redis.pcall → ScriptTransaction
    → async KvStorage 操作 (KeyLock 加锁 + spawn_blocking)
  → 超时检查 (5s) / 内存检查 (128MB)
  → 结果 RESP 编码
```

## 关键设计决策

| 决策 | 选择 | 理由 |
|------|------|------|
| 异步框架 | tokio | Rust 生态最成熟 async runtime |
| RESP 协议 | RESP2 + RESP3 双兼容 | 客户端兼容性最大化 |
| 存储抽象 | `KvStorage` trait | 内存/AiDb 引擎可切换 |
| Lua 沙箱 | mlua + StdLib 裁剪 | 防止恶意脚本破坏 |
| 集群路由 | `#[cfg(feature = "cluster")]` | 单机集群同一二进制 |
| JSON 存储 | ValueType::String + serde_json | 无需专用存储路径 |
| Key 锁 | 字典序排序锁 | 避免死锁, 简单可靠 |

## 设计原则

详见 [DESIGN.md](DESIGN.md).
