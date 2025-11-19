# AiKv 集群和高可用适配方案

## 📋 任务概述

根据 TODO.md "优先级 9 - 集群和高可用" 的要求，本方案将 AiKv 的集群和高可用功能适配到 AiDb v0.2.0 的集群架构。

## 🎯 核心目标

1. **升级 AiDb 依赖**：从 v0.1.0 升级到 v0.2.0
2. **参考 AiDb 集群方案**：利用 AiDb v0.2.0 已有的分布式集群能力
3. **适配 Redis 协议**：确保 Redis 客户端能够透明访问 AiKv 集群
4. **最小化改动**：利用 AiDb 现有能力，避免重复造轮子

## 📊 当前状态分析

### AiKv v0.1.0 现状
- ✅ 基于 AiDb v0.1.0（单机版）
- ✅ 完整的 Redis 协议支持（RESP2/RESP3）
- ✅ 支持 String、List、Hash、Set、ZSet 数据类型
- ✅ 支持 JSON、Lua 脚本
- ✅ 支持 TTL 过期机制
- ✅ 双存储引擎：Memory 和 AiDb
- ❌ 无集群支持
- ❌ 无主从复制
- ❌ 无故障转移

### AiDb v0.2.0 新增能力
- ✅ **完整的分布式集群架构**
  - Primary-Replica 架构（Replica 作为缓存层）
  - gRPC 远程过程调用
  - Coordinator 集群协调器（一致性哈希路由）
  - 多 Shard 分片，支持水平扩展
  - 健康检查和故障自动检测
- ✅ **备份恢复系统**
  - 完整的备份恢复机制（本地和云存储）
  - WAL 归档和回放
  - 快照管理
- ✅ **弹性伸缩**
  - 手动和自动扩缩容
  - 节点动态添加/移除
- ✅ **监控和运维**
  - Prometheus 监控
  - Grafana 仪表盘
  - aidb-admin CLI 工具

## 🏗️ 集群架构设计

### 方案一：代理模式（推荐）

```
┌─────────────────────────────────────────────────────────┐
│                    Redis Clients                         │
│         (redis-cli, redis-py, node-redis, etc.)         │
└───────────────────────┬─────────────────────────────────┘
                        │ Redis Protocol (RESP2/RESP3)
                        ▼
┌─────────────────────────────────────────────────────────┐
│                   AiKv Cluster Layer                     │
│  ┌─────────────────────────────────────────────────┐   │
│  │          AiKv Proxy / Coordinator               │   │
│  │  • Redis 协议解析                                │   │
│  │  • 命令路由（基于键的一致性哈希）                  │   │
│  │  • 连接管理                                       │   │
│  │  • 客户端重定向（MOVED/ASK）                      │   │
│  └─────────────────┬───────────────────────────────┘   │
└────────────────────┼───────────────────────────────────┘
                     │
     ┌───────────────┼───────────────┐
     │               │               │
┌────▼────┐     ┌───▼────┐     ┌───▼────┐
│ AiKv    │     │ AiKv   │     │ AiKv   │
│ Node 1  │     │ Node 2 │     │ Node N │
│┌───────┐│     │┌──────┐│     │┌──────┐│
││ Redis ││     ││Redis ││     ││Redis ││
││Handler││     ││Handler│     ││Handler│
│└───┬───┘│     │└──┬───┘│     │└──┬───┘│
│    │    │     │   │    │     │   │    │
│┌───▼───┐│     │┌──▼───┐│     │┌──▼───┐│
││ AiDb  ││     ││ AiDb ││     ││ AiDb ││
││Primary││     ││Primary│     ││Primary│
│└───┬───┘│     │└──────┘│     │└──────┘│
│    │    │     
│┌───▼────┐│    (Each node can have Replicas)
││Replicas││
│└────────┘│
└──────────┘
```

**特点**：
- AiKv 作为 Redis 协议层，每个节点独立处理 Redis 命令
- 底层使用 AiDb v0.2.0 的 Shard Group 管理数据分片
- Coordinator 负责键路由和负载均衡
- 支持 Redis Cluster 的 MOVED/ASK 重定向

### 方案二：智能客户端模式

```
┌──────────────────────────────────────┐
│       Redis Clients                   │
│  (with AiKv cluster awareness)       │
└───────┬──────────────────────────────┘
        │ Direct Connection
        │ (after route discovery)
        ▼
┌───────────────────────────────────────┐
│    AiKv Cluster (multiple nodes)      │
│  Each node:                           │
│  • Redis Protocol Handler             │
│  • Local routing logic                │
│  • Returns MOVED if key not local    │
│  • Uses AiDb for storage              │
└───────────────────────────────────────┘
```

**特点**：
- 客户端直连各个 AiKv 节点
- 节点返回 MOVED/ASK 响应引导客户端
- 需要客户端支持 Redis Cluster 协议

## 📐 详细设计

### 1. AiDb 依赖升级

**文件**：`Cargo.toml`

```toml
[dependencies]
# 从 v0.1.0 升级到 v0.2.0
aidb = { git = "https://github.com/Genuineh/AiDb", tag = "v0.2.0" }
```

**影响分析**：
- ✅ API 兼容性：AiDb v0.2.0 保持单机 API 向后兼容
- ✅ 新增功能：可选使用集群功能
- ⚠️ 需要验证：确保现有的 `AiDbStorageAdapter` 正常工作

### 2. 集群配置结构

**新增文件**：`src/config/cluster.rs`

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterConfig {
    /// 是否启用集群模式
    pub enabled: bool,
    
    /// 当前节点 ID
    pub node_id: String,
    
    /// 当前节点绑定地址
    pub bind_addr: String,
    
    /// 集群节点列表
    pub nodes: Vec<ClusterNode>,
    
    /// Coordinator 地址（如果使用代理模式）
    pub coordinator_addr: Option<String>,
    
    /// 集群模式：proxy 或 smart_client
    pub mode: ClusterMode,
    
    /// 数据分片数量
    pub num_shards: usize,
    
    /// 副本数量
    pub num_replicas: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterNode {
    pub id: String,
    pub addr: String,
    pub role: NodeRole,  // Primary or Replica
    pub shard_id: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClusterMode {
    Proxy,        // 使用 Coordinator 代理
    SmartClient,  // 智能客户端模式
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NodeRole {
    Primary,
    Replica,
}
```

### 3. 集群路由层

**新增文件**：`src/cluster/router.rs`

```rust
use std::sync::Arc;
use std::collections::HashMap;

/// 集群路由器 - 负责将 Redis 命令路由到正确的 Shard
pub struct ClusterRouter {
    /// AiDb Coordinator（使用 v0.2.0 的 Coordinator）
    coordinator: Arc<aidb::cluster::Coordinator>,
    
    /// 节点映射（node_id -> connection）
    nodes: HashMap<String, Arc<NodeConnection>>,
    
    /// 一致性哈希环
    hash_ring: ConsistentHashRing,
}

impl ClusterRouter {
    /// 根据 key 路由到正确的节点
    pub async fn route(&self, key: &[u8]) -> Result<String> {
        // 使用 AiDb 的一致性哈希算法
        let shard_id = self.coordinator.route_key(key)?;
        
        // 获取 shard 的 primary 节点
        let node_id = self.get_primary_for_shard(shard_id)?;
        
        Ok(node_id)
    }
    
    /// 执行命令（自动路由）
    pub async fn execute_command(&self, cmd: Command) -> Result<Response> {
        // 提取键
        let key = cmd.get_key()?;
        
        // 路由到目标节点
        let node_id = self.route(key).await?;
        
        // 获取连接
        let conn = self.nodes.get(&node_id)
            .ok_or_else(|| Error::NodeNotFound)?;
        
        // 执行命令
        conn.execute(cmd).await
    }
}
```

### 4. Redis Cluster 协议支持

**扩展文件**：`src/command/cluster.rs`

```rust
/// Redis Cluster 相关命令
pub struct ClusterCommands {
    router: Arc<ClusterRouter>,
    config: Arc<ClusterConfig>,
}

impl ClusterCommands {
    /// CLUSTER SLOTS - 返回槽位分配信息
    pub async fn cluster_slots(&self) -> Result<Response> {
        // 返回每个 shard 的槽位范围和节点信息
        let slots_info = self.router.get_slots_info().await?;
        
        // 转换为 Redis 协议格式
        Ok(Response::Array(slots_info))
    }
    
    /// CLUSTER NODES - 返回集群节点信息
    pub async fn cluster_nodes(&self) -> Result<Response> {
        let nodes_info = self.router.get_nodes_info().await?;
        Ok(Response::BulkString(nodes_info.into()))
    }
    
    /// CLUSTER INFO - 返回集群状态信息
    pub async fn cluster_info(&self) -> Result<Response> {
        let info = format!(
            "cluster_state:ok\n\
             cluster_slots_assigned:{}\n\
             cluster_slots_ok:{}\n\
             cluster_known_nodes:{}\n",
            self.config.num_shards * 16384 / self.config.num_shards,
            self.config.num_shards * 16384 / self.config.num_shards,
            self.config.nodes.len()
        );
        
        Ok(Response::BulkString(info.into()))
    }
}
```

### 5. 命令路由处理

**修改文件**：`src/server/handler.rs`

```rust
impl Handler {
    pub async fn handle_command(&mut self, cmd: Command) -> Result<Response> {
        // 如果启用集群模式
        if self.cluster_enabled {
            return self.handle_cluster_command(cmd).await;
        }
        
        // 单机模式（现有逻辑）
        self.handle_standalone_command(cmd).await
    }
    
    async fn handle_cluster_command(&mut self, cmd: Command) -> Result<Response> {
        // 特殊处理集群命令
        match cmd.name.to_uppercase().as_str() {
            "CLUSTER" => return self.cluster_commands.execute(&cmd).await,
            _ => {}
        }
        
        // 检查键是否属于本节点
        if let Some(key) = cmd.get_key() {
            let target_node = self.router.route(key).await?;
            
            if target_node != self.config.node_id {
                // 返回 MOVED 重定向
                let target_addr = self.get_node_addr(&target_node)?;
                let slot = self.router.get_slot_for_key(key);
                
                return Ok(Response::Error(
                    format!("MOVED {} {}", slot, target_addr)
                ));
            }
        }
        
        // 键属于本节点，正常处理
        self.handle_standalone_command(cmd).await
    }
}
```

### 6. 存储层集成

**修改文件**：`src/storage/aidb_adapter.rs`

```rust
pub struct AiDbStorageAdapter {
    // 单机模式：直接使用 DB
    db: Option<Arc<aidb::DB>>,
    
    // 集群模式：使用 ShardGroup
    shard_group: Option<Arc<aidb::cluster::ShardGroup>>,
    
    // 配置
    cluster_config: Option<ClusterConfig>,
}

impl AiDbStorageAdapter {
    /// 创建单机实例
    pub fn new_standalone(db: Arc<aidb::DB>) -> Self {
        Self {
            db: Some(db),
            shard_group: None,
            cluster_config: None,
        }
    }
    
    /// 创建集群实例
    pub fn new_cluster(
        shard_group: Arc<aidb::cluster::ShardGroup>,
        config: ClusterConfig,
    ) -> Self {
        Self {
            db: None,
            shard_group: Some(shard_group),
            cluster_config: Some(config),
        }
    }
    
    /// 获取值（自动路由）
    pub fn get_value(&self, db: usize, key: &str) -> Result<Option<StoredValue>> {
        if let Some(db) = &self.db {
            // 单机模式
            self.get_from_standalone(db, key)
        } else if let Some(shard_group) = &self.shard_group {
            // 集群模式 - 使用 ShardGroup
            self.get_from_cluster(shard_group, db, key)
        } else {
            Err(Error::InvalidState)
        }
    }
}
```

### 7. 配置文件示例

**新增文件**：`config/cluster.toml`

```toml
[server]
host = "0.0.0.0"
port = 6379

[cluster]
enabled = true
mode = "proxy"  # 或 "smart_client"
node_id = "node-1"
bind_addr = "192.168.1.10:6379"
num_shards = 3
num_replicas = 2

# Coordinator 地址（proxy 模式必需）
coordinator_addr = "192.168.1.100:7379"

# 集群节点列表
[[cluster.nodes]]
id = "node-1"
addr = "192.168.1.10:6379"
role = "Primary"
shard_id = 0

[[cluster.nodes]]
id = "node-2"
addr = "192.168.1.11:6379"
role = "Primary"
shard_id = 1

[[cluster.nodes]]
id = "node-3"
addr = "192.168.1.12:6379"
role = "Primary"
shard_id = 2

[[cluster.nodes]]
id = "replica-1"
addr = "192.168.1.13:6379"
role = "Replica"
shard_id = 0

[storage]
engine = "aidb"
data_dir = "./data"

[logging]
level = "info"
```

## 🔄 实施步骤

### 阶段 1：依赖升级和验证（1-2天）
1. ✅ 升级 `Cargo.toml` 中的 AiDb 依赖到 v0.2.0
2. ✅ 验证现有单机功能正常工作
3. ✅ 运行所有现有测试，确保通过
4. ✅ 更新文档说明 AiDb 版本升级

### 阶段 2：集群配置和基础结构（2-3天）
1. 创建 `src/config/cluster.rs` - 集群配置结构
2. 创建 `src/cluster/` 模块目录
3. 实现基础的集群配置加载
4. 添加集群配置的单元测试

### 阶段 3：集群路由层（3-4天）
1. 实现 `ClusterRouter` - 集成 AiDb Coordinator
2. 实现一致性哈希路由
3. 实现节点连接管理
4. 添加路由逻辑的单元测试

### 阶段 4：Redis Cluster 协议（2-3天）
1. 实现 `CLUSTER SLOTS` 命令
2. 实现 `CLUSTER NODES` 命令
3. 实现 `CLUSTER INFO` 命令
4. 实现 MOVED/ASK 重定向
5. 添加集群命令的测试

### 阶段 5：命令路由集成（3-4天）
1. 修改 `Handler` 支持集群模式
2. 实现命令路由逻辑
3. 实现键所属检查
4. 实现自动重定向
5. 添加端到端测试

### 阶段 6：存储层集成（2-3天）
1. 修改 `AiDbStorageAdapter` 支持集群模式
2. 集成 AiDb ShardGroup
3. 实现跨节点操作（如 MGET）
4. 添加集成测试

### 阶段 7：测试和文档（2-3天）
1. 编写完整的集成测试套件
2. 性能测试和优化
3. 更新 README 和用户文档
4. 编写集群部署指南
5. 更新 TODO.md

## 📝 测试计划

### 单元测试
- [ ] 集群配置解析和验证
- [ ] 一致性哈希路由算法
- [ ] 节点连接管理
- [ ] MOVED/ASK 响应生成

### 集成测试
- [ ] 多节点集群启动
- [ ] 跨节点数据读写
- [ ] 节点故障转移
- [ ] 数据重新分片

### 性能测试
- [ ] 集群模式下的 QPS
- [ ] 跨节点延迟
- [ ] 负载均衡效果

## 📚 文档更新

1. **README.md**
   - 添加集群模式使用说明
   - 更新架构图
   - 添加集群配置示例

2. **新增文档**
   - `docs/CLUSTER_GUIDE.md` - 集群部署和使用指南
   - `docs/CLUSTER_ARCHITECTURE.md` - 集群架构详解
   - `examples/cluster_example.rs` - 集群使用示例

3. **TODO.md**
   - 更新 "优先级 9" 状态
   - 标记已完成的任务

## ⚠️ 风险和缓解

### 风险 1：AiDb API 变化
- **缓解**：仔细阅读 AiDb v0.2.0 文档，使用 feature flags 隔离集群功能

### 风险 2：性能影响
- **缓解**：在集群模式下增加性能测试，优化热点路径

### 风险 3：Redis 协议兼容性
- **缓解**：使用 redis-cli 和 redis-py 进行兼容性测试

### 风险 4：数据一致性
- **缓解**：依赖 AiDb 的一致性保证，添加数据校验测试

## 🎯 验收标准

### 必须满足
1. ✅ AiDb 依赖成功升级到 v0.2.0
2. ✅ 所有现有测试通过
3. ✅ 集群模式可配置开关（默认关闭）
4. ✅ 支持多节点集群部署
5. ✅ 支持 Redis Cluster 基本命令
6. ✅ 文档更新完整

### 可选目标
- ⭐ 支持主从复制（利用 AiDb Primary-Replica）
- ⭐ 支持自动故障转移
- ⭐ 支持动态扩缩容
- ⭐ 集成 Prometheus 监控

## 📊 时间估算

| 阶段 | 预计时间 | 依赖 |
|------|---------|------|
| 阶段 1：依赖升级 | 1-2天 | - |
| 阶段 2：基础结构 | 2-3天 | 阶段 1 |
| 阶段 3：路由层 | 3-4天 | 阶段 2 |
| 阶段 4：Redis 协议 | 2-3天 | 阶段 3 |
| 阶段 5：命令路由 | 3-4天 | 阶段 4 |
| 阶段 6：存储集成 | 2-3天 | 阶段 5 |
| 阶段 7：测试文档 | 2-3天 | 阶段 6 |
| **总计** | **15-22天** | - |

## 🔍 后续优化方向

1. **主从复制**：完整利用 AiDb Primary-Replica 架构
2. **哨兵模式**：自动故障检测和转移
3. **Pub/Sub 集群化**：支持跨节点发布订阅
4. **事务支持**：分布式事务处理
5. **Stream 支持**：集群模式下的 Stream 数据类型

## 📌 总结

本方案充分利用 AiDb v0.2.0 的分布式集群能力，通过在 AiKv 上添加一层 Redis 协议适配层，实现 Redis 客户端对 AiKv 集群的透明访问。方案具有以下优势：

1. ✅ **最小化改动**：利用 AiDb 现有能力，避免重复开发
2. ✅ **渐进式升级**：集群功能可选，不影响单机模式
3. ✅ **协议兼容**：完整支持 Redis Cluster 协议
4. ✅ **生产可用**：基于成熟的 AiDb 集群架构

---

**文档版本**：v1.0  
**创建日期**：2025-11-19  
**作者**：GitHub Copilot  
**审核状态**：待审核
