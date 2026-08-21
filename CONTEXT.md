# AiKv 领域词汇

## 协议层

**RESP**: Redis Serialization Protocol, AiKv 实现的 Redis 线缆协议, 同时支持 RESP2 和 RESP3.

**BulkString**: RESP 中的二进制安全字符串编码, 不做 UTF-8 假设, 可传输任意字节序列.

## 路由层

**CommandRouter**: 根据命令类型 (读/写/管理等) 将请求分派到对应处理器.

**ClusterRouter**: 在集群模式下, 根据 key 的 CRC16 哈希计算 slot, 将请求路由到正确的节点或返回 MOVED 错误.

## 存储层

**KvStorage**: 存储抽象接口, 统一内存和持久化两种后端.

**MemoryEngine**: 纯内存的 KV 存储实现, 用于开发测试或不需要持久化的场景.

**AiDbEngine**: 通过 aidb crate 实现的持久化存储, 生产推荐.

**StoredValue**: AiKv 内部对 Redis 值的编码表示, 包含类型标记和序列化后的字节.

**ValueType**: Redis 数据类型的枚举 (String, List, Set, ZSet, Hash 等).

**DbKeyCounters**: 内存原子键计数器 (`[AtomicU64; 16]`), 跟踪各逻辑 DB 的顶级用户 key 总数, 提供 $O(1)$ 的 `DBSIZE` 与 `db_key_counts()` 统计.

**键计数与过期语义** (三层一致):
1. **热路径**: 计数含尚未惰性删除的过期 key (对齐 Redis `DBSIZE`); 主动删 / 覆盖写 / 读触发惰性删成功时精确 `decr`.
2. **冷启动 Rebuild**: `open` 时按「当前未过期存活逻辑 key」扫描填充, 并可能顺带 lazy 清理过期 key.
3. **Compaction GC**: listener 持 `Weak<DB>`; Version 安装后仅在 `get==None` 且 `ExpireDecrGate` 单飞成功时 `decr`; 惰性删/`DEL` 同样需赢得门闩并持有至 `set_typed` / `write_batch` Put 重生, 避免 tombstone 被再次 notify.

**Rebuild Counters**: 在冷启动 `open` 时对各 DB 执行一次性前缀扫描恢复键计数器, 杜绝生产重启后指标从 0 开始失真.

## 集群层

**Slot**: 16384 个槽位之一, 通过 CRC16(key) % 16384 计算; 与 Redis Cluster 完全兼容.

**MOVED/ASK**: 集群重定向错误; 由客户端处理 (`redis-cli -c`), 服务端不做透明转发.

**Slot 迁移**: 将 slot 从源节点迁移到目标节点的在线过程, 不中断服务.

**Gossip**: 每节点定期发送的轻量拓扑信息, 用于刷新 leader 路由缓存和集群指标; 故障判定走 MetaRaft, 非 SWIM 式 gossip.

**MetaRaft**: 管理集群元数据的 Raft 组 (控制面, 低频).

**MultiRaft**: 按 slot Group 划分的多个数据面 Raft 组 (数据面, 高吞吐).

## 写入批处理

**Batcher**: 将多个写入操作合并为一批, 减少 Raft 共识次数.

**EagerFlush**: 批处理中的阈值, 当累计操作数达到此值时不等 timeout 立即 propose.

## 可观测性

**OTLP**: OpenTelemetry Protocol, AiKv 的生产指标出口; 指标前缀为 `aikv_*`.

**INFO**: Redis 兼容的 INFO 命令输出; 目标是与 Redis 8.8 的 INFO 字段对齐, 部分字段为 stub 值.
