# MultiRaft Failover 修复方案

## 问题综述

kill 分片 primary 后，副本没有接管 slot，CLUSTER NODES 仍指向已死节点。

## 根因

N1 通过 `add_learner_to_group(1, 2, "127.0.0.1:16380")` 把 N2 加入 group 1 时，地址用的是 **MetaRaft RPC 端口** (`rpc_port`)。N2 的 **MultiRaft gRPC server** 在 `rpc_port + 10000` (`26380`)。消息发到 `16380` 时，该端口的 dispatcher 只注册了 group 0（MetaRaft），`get_raft(1)` 返回 None，`add_learner` 静默失败。

**三个受影响路径**（共享 dispatcher 修复自动覆盖全部）:

| 路径 | 文件:行 | 地址来源 |
|------|---------|---------|
| A. `change_group_membership` | `membership_coordinator.rs:325-332` | `node.rpc_addr` |
| B. `apply_membership_change` (drift 对账) | `multi_raft_node.rs:377-381` | `router.get_node_addr()` → `rpc_addr` |
| C. `add_node` TakeoverGroups | `membership_coordinator.rs:154-165` | `node.rpc_addr` |

连锁效应:

```
add_learner 失败
  → N2 从未收到 membership change {1,2}
    → N2 的 group 1 始终是 blank Learner（init_as_voter=false）
      → LeaderChangeWatcher 检测不到 leader
        → MetaRaft is_leader 不更新
          → CLUSTER NODES / MOVED 重定向指向死 N1
```

## 修复方案: 共享 dispatcher (方向 A)

**核心思路:** 让 MetaRaft gRPC server 使用全局共享的 `RaftServiceDispatcher`（与 MultiRaft gRPC server 共用），这样任意端口都能路由任意 group 的 Raft 消息。

### 改动点

#### 1. `AiDb/src/cluster/node.rs` — `OpenRaftNode::start_server`

新增 `start_server_with_dispatcher` 方法，接受外部 dispatcher:

```rust
pub async fn start_server_with_dispatcher(
    &self,
    addr: SocketAddr,
    max_message_size: u64,
    dispatcher: Arc<RaftServiceDispatcher>,
) -> Result<()> {
    dispatcher.register_group(self.group_id, self.raft.clone());
    let service = RaftServiceImpl::new(dispatcher);
    // ... 启动 gRPC server (与原 start_server 相同) ...
}
```

原 `start_server` 改为调用新方法（内部创建独立 dispatcher，保持向后兼容）。

#### 2. `AiDb/src/cluster/meta_raft_node.rs` — `MetaRaftNode::start_server`

透出 `start_server_with_dispatcher`:

```rust
pub async fn start_server_with_dispatcher(
    &self, addr: SocketAddr, max_message_size: u64,
    dispatcher: Arc<RaftServiceDispatcher>,
) -> Result<()> {
    self.inner.start_server_with_dispatcher(addr, max_message_size, dispatcher).await
}
```

#### 3. `AiKv/src/main.rs` — `init_cluster`

将 MetaRaft gRPC 改为使用共享 dispatcher:

```rust
// 之前: meta_raft_clone.start_server(rpc_socket, max_msg_size)
// 之后: meta_raft_clone.start_server_with_dispatcher(
//     rpc_socket, max_msg_size, dispatcher.clone()
// )
```

其中 `dispatcher` 来自 `Arc::new(RaftServiceDispatcher::new())`（即传给 `MultiRaftNode::new_with_lifecycle` 的同一个）。

#### 4. `AiDb/src/cluster/membership_coordinator.rs` — 无需改动

`add_node` 中新增的 `promote_learner_to_voter`（MetaRaft voter 晋升）已经存在且正常工作。

#### 5. `AiDb/src/cluster/multi_raft_node.rs` — 无需改动

`create_group_inner` 的 `init_as_voter` 逻辑保持不变（仅 leader 初始化 single voter）。初始化地址 `format!("127.0.0.1:{}", 10000 + node_id)` 是占位符 — 单节点 group 不需要真实地址，后续 drift 对账会用 `router.get_node_addr()` 获取正确地址覆盖。

### 修复后的数据流

```
N1 (leader of group 1)
  │
  │ add_learner(2, "127.0.0.1:16380")
  │
  ▼
N2:16380 (MetaRaft gRPC, 现在用共享 dispatcher)
  │ dispatcher.get_raft(1) → OpenRaftNode for group 1 ✓
  │ forward AppendEntries → N2's group 1 收到 membership change
  │ N2's group 1 apply ReplaceAllVoters({1,2}) → N2 becomes Voter ✓
  ▼

--- N1 宕机后 ---

N2's group 1 (now Voter)
  │ Raft election: N2 wins (sole surviving Voter) ✓
  │ LeaderChangeWatcher detects leader change (None→2 or 1→2) ✓
  │ propose ChangeGroupMembership(is_leader=true for N2) → MetaRaft ✓
  │ LifecycleManager refreshes Router → group_leader[1] = 2 ✓
  │ MOVED redirect → 127.0.0.1:6380 (alive N2) ✓
```

### e2e 测试验证

`test_cluster_failover.sh` 流程（重写为真实 failover 场景）:

```
1. 3 节点: N1(主), N2(从), N3(从), slots 全部在 N1
2. CLUSTER ADD_REPLICA 将 N2, N3 添加为 group 1 的 replica
3. 写入 30 条数据
4. kill N1（主节点）
5. wait 10s（Raft election + LeaderChangeWatcher）
6. CLUSTER NODES — 验证 is_leader 已变更到 N2 或 N3
7. GET 数据 — 验证 MOVED 指向新 primary
8. SET 新数据 — 验证可写入
9. restart N1 — 验证集群恢复
```

### 测试计划

| 测试类型 | 内容 |
|---------|------|
| 单元测试 | `start_server_with_dispatcher` 验证 dispatcher 正确注册 group |
| 集成测试 | 共享 dispatcher 路由多个 group 的 RPC |
| E2E 测试 | 3 节点 + 副本 → kill primary → 验证 failover 完整链路 |

### 影响范围

| 层面 | 影响 |
|------|------|
| 数据面 write path | 不变 |
| MetaRaft 消息 | 不变 |
| MultiRaft 消息 | 修复 (消息现在可达) |
| API | 不变 |
| 安全性 | MetaRaft 端口现在可路由数据 group RPC，仅在集群内网通信，无实际风险 |

### 预计改动量

~40 行 (新增 `start_server_with_dispatcher` + `main.rs` 调用 + 单元测试 + e2e 重写)
