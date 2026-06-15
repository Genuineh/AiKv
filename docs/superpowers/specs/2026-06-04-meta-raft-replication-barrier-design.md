# MetaRaft Learner→Voter 复制屏障修复

## 1. 问题建模

### 当前调用链

```
MembershipCoordinator::add_node()
  ├─ meta_raft.add_learner_nonblocking(n)   ← 返回, n 可能一条 log 都没收到
  ├─ meta_raft.promote_learner_to_voter(n)  ← 立即调 change_membership
  │    └─ change_membership({1,...,n})
  │         ├─ joint consensus 需要 quorum of {1..n} 的 ACK
  │         ├─ 得到足够 ACK → N1 上 commit ✓
  │         └─ 返回成功
  │
  │    ⚠️ 此时 follower 的 Raft log 中有 membership entry (已持久化)
  │       但 commit_index 未推进 → StateMachine 未 apply
  │       N1 需要在下一轮 heartbeat 告诉 follower 新的 commit_index
  │
  └─ 如果 N1 在 heartbeat 之前宕机:
       follower 重启 → 从磁盘读 Raft log → entry 存在但 uncommitted
       → 不 apply → 仍是 Learner → 不能参与选举 → 集群不可恢复
```

### 核心矛盾

`change_membership` 成功只保证 entries **已复制到 follower log**, 不保证 follower **已 apply**. 只有 leader 的下一轮 heartbeat 才会推进 follower 的 `commit_index`. 如果 leader 在这个间隙宕机, follower 永远不知道那些 entries 已经 committed.

### 安全保证

确认至少 1 个其他 voter 持有 membership change entry 即足够:

- `change_membership` 返回的前提是 entry 已在多数派上持久化
- 如果 1 个 follower 的 `matched >= last_log_index`, 该 follower 的 log 包含 membership change entry
- Raft 选举限制: 只有 log 更新的节点能当选 → 该 follower 一定能赢得选举
- 新 leader 当选后 propose no-op entry → commit no-op → commit_index 推进 → membership change 被 apply

## 2. 修复方案

### 概览

改 `AiDb/src/cluster/meta_raft_node.rs` 的 `promote_learner_to_voter` 一个函数, 在 `change_membership` 前后各加一道屏障.

### 屏障 1 — 前置: Catch-up Gate

`change_membership` 之前, 确保目标 learner 已追平 leader 的日志. 用 `raft.metrics()` 检查目标节点的 `matched` log index 与 leader 的 `last_log_index` 差距不超过阈值.

- 轮询间隔: 50ms
- 超时: 30s
- 阈值: 5 条 entry
- 超时返回 `ClusterError::Timeout`, `add_node` 的 `tracing::warn` 捕获, 下次 MEET 重试

### 屏障 2 — 后置: Replication Confirmation

`change_membership` 返回后, 分两步确认:

1. **等 entry 复制到 follower log**: 确认至少一个其他 voter 的 `matched >= leader.last_log_index`. 轮询 50ms, 超时 5s.
2. **等 commit_index 传播**: sleep `3 * heartbeat_interval`, 让 follower 收到 leader_commit 并 apply.

### 为什么不在 `add_node` 中把 `add_learner_nonblocking` 换成 `add_learner` (blocking)

保留 `add_learner_nonblocking` — 避免在 learner 不可达时阻塞整个 `add_node`. 屏障 1 放在 `promote_learner_to_voter` 内部已足够, 且超时后 `add_node` 的现有 `tracing::warn` 逻辑自然捕获.

## 3. 详细实现

### 修改文件: `AiDb/src/cluster/meta_raft_node.rs`

#### 3.1 `MetaRaftNode` 新增字段

```rust
pub struct MetaRaftNode {
    inner: OpenRaftNode,
    state_machine: Arc<MetaStateMachine>,
    heartbeat_interval_ms: u64,  // 新增, 从 RaftNodeConfig 读取
}
```

在 `new()` 中提取:

```rust
let heartbeat_interval_ms = config.heartbeat_interval;
```

#### 3.2 常量

```rust
const CATCH_UP_TIMEOUT: Duration = Duration::from_secs(30);
const CATCH_UP_POLL: Duration = Duration::from_millis(50);
const CATCH_UP_THRESHOLD: u64 = 5;
const REPLICATION_CONFIRM_TIMEOUT: Duration = Duration::from_secs(5);
const REPLICATION_POLL: Duration = Duration::from_millis(50);
const REPLICATION_HEARTBEAT_MULTIPLIER: u32 = 3;
```

#### 3.3 `promote_learner_to_voter` 修改后结构

```rust
#[instrument(skip(self), fields(node_id, heartbeat_ms = self.heartbeat_interval_ms))]
pub async fn promote_learner_to_voter(&self, node_id: NodeId) -> Result<()> {
    // ── 屏障 1: 等 learner 追平 ──
    self.wait_learner_catch_up(node_id).await?;

    // 1. 从 ClusterMeta 读当前 voter 列表
    let meta = self.get_cluster_meta();
    let mut voter_ids: Vec<NodeId> = meta
        .nodes
        .iter()
        .filter(|(_, info)| matches!(info.role, crate::cluster::meta_types::NodeRole::Voter))
        .map(|(id, _)| *id)
        .collect();
    if voter_ids.is_empty() {
        voter_ids.push(self.node_id());
    }
    if !voter_ids.contains(&node_id) {
        voter_ids.push(node_id);
    }

    // 2. 调用 change_membership
    self.inner.change_membership(voter_ids).await?;

    // ── 屏障 2: 确认 entry 已传播 ──
    self.confirm_replication(&voter_ids).await?;

    // 3. 更新 ClusterMeta 中的 NodeRole
    self.propose(MetaRequest::ChangeNodeRole {
        node_id,
        role: crate::cluster::meta_types::NodeRole::Voter,
    })
    .await?;
    Ok(())
}
```

#### 3.4 `wait_learner_catch_up`

```rust
async fn wait_learner_catch_up(&self, node_id: NodeId) -> Result<()> {
    let start = tokio::time::Instant::now();
    let deadline = start + CATCH_UP_TIMEOUT;
    loop {
        let metrics = self.inner.metrics().await;
        let leader_last = metrics.last_log_index;
        let learner_matched = metrics
            .replication
            .get(&node_id)
            .map(|r| r.matched_log_index)
            .unwrap_or(0);

        tracing::debug!(
            node_id,
            leader_last,
            learner_matched,
            behind = leader_last.saturating_sub(learner_matched),
            "waiting for learner to catch up"
        );

        if leader_last.saturating_sub(learner_matched) <= CATCH_UP_THRESHOLD {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            tracing::warn!(
                node_id,
                leader_last,
                learner_matched,
                elapsed_ms = start.elapsed().as_millis(),
                "learner catch-up timed out"
            );
            return Err(Error::Cluster(ClusterError::Timeout(
                format!("learner {node_id} failed to catch up within 30s")
            )));
        }
        tokio::time::sleep(CATCH_UP_POLL).await;
    }
}
```

#### 3.5 `confirm_replication`

```rust
async fn confirm_replication(&self, voter_ids: &[NodeId]) -> Result<()> {
    // 快速路径: 只有本节点一个 voter, entry 已本地 committed, 无需等别人
    if voter_ids.len() <= 1 {
        tracing::debug!("single voter, replication confirmation skipped");
        return Ok(());
    }

    // Step 1: 确认至少一个其他 voter 收到 entry
    let last_log = self.inner.metrics().await.last_log_index;
    let deadline = tokio::time::Instant::now() + REPLICATION_CONFIRM_TIMEOUT;

    loop {
        let metrics = self.inner.metrics().await;
        let confirmed_voter = voter_ids.iter().find(|id| {
            *id != self.node_id()
                && metrics
                    .replication
                    .get(id)
                    .map(|r| r.matched_log_index >= last_log)
                    .unwrap_or(false)
        });
        if let Some(voter_id) = confirmed_voter {
            tracing::debug!(
                confirmed_voter = %voter_id,
                last_log_index = last_log,
                "replication confirmed, waiting for commit propagation"
            );
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(Error::Cluster(ClusterError::Timeout(
                "no voter confirmed replication within 5s".into()
            )));
        }
        tokio::time::sleep(REPLICATION_POLL).await;
    }

    // Step 2: 等 3 个 heartbeat 让 commit_index 传播
    tokio::time::sleep(Duration::from_millis(
        REPLICATION_HEARTBEAT_MULTIPLIER as u64 * self.heartbeat_interval_ms,
    ))
    .await;
    Ok(())
}
```

## 4. 错误处理 & 可观测性

### 错误分支

| 场景 | 行为 |
|------|------|
| 屏障 1 超时 (learner 30s 未追平) | `Err(ClusterError::Timeout(...))` → `add_node` `tracing::warn` 捕获, 下次 MEET 重试 |
| 屏障 2 step 1 超时 (5s 无 follower 确认) | `Err(ClusterError::Timeout(...))` → 同上 |
| `change_membership` 本身失败 | 已有 NotLeader/Raft 错误处理, `propose` 3 次重试 |
| 屏障期间 N1 不再是 leader | `client_write` → `ForwardToLeader` → `propose` 重试 |
| 目标节点已经离线 | 屏障 1 超时 → warn → 返回, 不阻塞集群 |

### Tracing

- `#[instrument(skip(self), fields(node_id, heartbeat_ms = self.heartbeat_interval_ms))]` on `promote_learner_to_voter`
- 屏障 1: `debug!` 含 `leader_last`, `learner_matched`, `behind`; 超时时 `warn!` 含 `elapsed_ms`
- 屏障 2: `debug!` 含 `confirmed_voter`, `last_log_index`
- 超时错误使用 `ClusterError::Timeout` 而非 `ClusterError::Raft`, 语义更精确

## 5. 测试计划

### 5.1 单元测试 (追加到 `meta_raft_node.rs` 现有 `#[cfg(test)] mod tests`, `tokio::time::pause()` 加速)

| 测试 | 描述 | 方法 |
|------|------|------|
| `learner_catch_up_timeout` | learner 永远不追平 → 模拟 30s 后超时 | `tokio::time::pause()` + single node, 手动构造无 learner 场景 |
| `replication_confirm_timeout` | change_membership 后无 follower 确认 → 5s 后超时 | `tokio::time::pause()` + single node |
| `barrier_fast_path` | learner 已追平 (`matched ≈ last_log`) + voter_ids 含其他节点 → 两道屏障在 <500ms 内通过 | 构造 learner 已追平的 metrics 场景 |

### 5.2 集成测试 (`tests/` — 需要双节点 + gRPC)

| 测试 | 描述 |
|------|------|
| `promote_single_learner_success` | 双节点: N1 leader + N2 learner → `promote_learner_to_voter(2)` → N2 `NodeRole::Voter` |

### 5.3 E2E 测试 (`AiKv/e2e/test_cluster_failover.sh`)

3 节点 failover 流程:

```
1. 3 节点启动 (N1/N2/N3)
2. CLUSTER ADDSLOTS 0 16383 N1
3. CLUSTER ADD_REPLICA <n1_hex> <n2_hex>  ← 补充
4. CLUSTER ADD_REPLICA <n1_hex> <n3_hex>  ← 补充
5. SET 30 条数据
6. kill N1
7. sleep 5 (election timeout 500-1000ms + barrier ×3 + LeaderChangeWatcher)
8. CLUSTER NODES 验证 is_leader 已变更到 N2 或 N3
9. GET 数据 — 验证 MOVED 指向新 primary
10. SET 新数据 — 验证可写入
```

注意:
- `ADD_REPLICA` 的 hex node ID 从 `CLUSTER NODES` 输出提取: `grep "myself,master" | awk '{print $1}'`
- 等待时间用 5 秒 (election 1s + barrier 0.15s + watcher 0.25s + 余量)

## 6. 改动总结

| 文件 | 新增 | 修改 | 说明 |
|------|------|------|------|
| `AiDb/src/cluster/meta_raft_node.rs` | ~60 行 | ~3 行 | 两道屏障 + `heartbeat_interval_ms` 字段 |
| `AiDb/src/cluster/meta_raft_node.rs` (test) | ~50 行 | 0 | 3 个纯逻辑单元测试 (追加到现有 `#[cfg(test)] mod tests`) |
| `AiDb/tests/` | ~60 行 | 0 | 1 个双节点集成测试 |
| `AiKv/e2e/test_cluster_failover.sh` | ~10 行 | 5 行 | 补充 replica 配置步骤 |

**无 API 变更, 无协议变更, 无新增文件.**

## 7. 后续工作

- [ ] **Bootstrap 模式**: 集群首次启动时, 所有 `--cluster-peers` 节点一起初始化 MetaRaft 为 multi-voter, 消除 single-voter 退化状态. 作为独立一期 feature 设计.
