# AiKv 设计决策

## 协议

### 为什么同时支持 RESP2 和 RESP3?

- RESP2: redis-cli 默认协议, 兼容性最广
- RESP3: Redis 6.0+ 新协议, 支持 Map/Set/Push 类型
- HELLO 命令协商, 连接级别切换
- 渐进式支持: 先保证 RESP2 完整, RESP3 仅关键命令使用 Map/Set

### 为什么解析器支持 Pipeline?

- Redis 性能关键路径: Pipeline 减少 RTT
- tokio async 读写循环, 一次 read 可解析多个命令
- 不完整数据返回 NeedMoreData, 等待更多数据到达

## 存储

### 为什么使用 `KvStorage` trait 抽象?

- 支持双引擎: 内存 (`MemoryEngine`) 和持久化 (`AiDbEngine`)
- 命令层不关心存储实现, 只调用 trait 方法
- 测试时可以用 MemoryEngine 注入, 不依赖真实文件系统

### 为什么 MemoryEngine 使用 16 个独立 db?

- 兼容 Redis SELECT 0-15 语义
- 每个 db 独立 HashMap + 过期队列
- `FLUSHDB`/`SWAPDB` 等操作隔离

### 为什么 AiDbEngine 使用 spawn_blocking?

- AiDb 引擎是同步代码 (LSM-Tree 操作)
- tokio async 服务不能直接调用同步阻塞操作
- `spawn_blocking` 将同步操作交给 blocking thread pool, 不阻塞 async runtime

## 命令

### 为什么 Lua 使用 mlua (vendored lua) 而非 rhai/自助解析?

- Lua 是 Redis 官方脚本语言, 用户习惯 EVAL/EVALSHA
- mlua 提供零成本 FFI, vendored lua54 无需系统依赖
- 沙箱实现: 裁剪 StdLib (禁用 OS/IO), 封印危险全局变量

### 为什么 redis.call 用 ScriptTransaction?

- Lua 内多个 redis.call 需要原子性 (同 WriteBatch)
- KeyLock 按字典序排序加锁, 避免死锁
- async KvStorage 调用需 spawn_blocking, ScriptTransaction 统一管理

### 为什么 JSON 存为 ValueType::String 而非专用路径?

- AiKv 不要求 JSON 部分更新的高效性 (无 partial update)
- `serde_json` 反序列化后再写回, 简单可靠
- 专用 JSON 存储增加复杂度, 且数据库层需支持 (AiDb 无此抽象)

## 集群

### 为什么集群路由使用 `#[cfg]` feature gate?

- 单机模式零额外依赖
- 一条命令即可在单机和集群间切换
- 与 AiDb cluster feature 对齐

### 为什么 CLUSTER 命令用 replica read 提升读吞吐?

- CLUSTER REPLICAS 列举副本, READONLY 标记连接
- ClusterRouter::decide 在 readonly 时优先选副本
- Redis Cluster 官方行为一致

## 可观测性

### 为什么慢查询日志使用 tokio::time::Instant?

- 命令执行在 async context, Instant 比 clock_gettime 开销更低
- 阈值可配置 (`slowlog-log-slower-than`, 默认 100ms)
- SLOWLOG GET/LEN/RESET 命令兼容 Redis

### 为什么 Prometheus 指标在 `monitoring` feature 下编译?

- 避免生产环境不必要的依赖
- 默认无额外内存/CPU 开销
- 按需启用, 容器化部署典型
