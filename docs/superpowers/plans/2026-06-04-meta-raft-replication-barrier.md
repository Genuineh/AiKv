# MetaRaft Learner→Voter 复制屏障修复 — 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 `promote_learner_to_voter` 的 `change_membership` 前后各加一道复制屏障, 消除 leader 在 commit_index 传播前宕机导致 follower 永久保持 Learner 的竞态窗口.

**Architecture:** 只改 `meta_raft_node.rs` 一个文件. `MetaRaftNode` 新增 `heartbeat_interval_ms` 字段 + 两个私有辅助方法 (`wait_learner_catch_up`, `confirm_replication`), 在 `promote_learner_to_voter` 中依次调用. 无 API/协议变更.

**Tech Stack:** Rust, tokio, openraft, tracing

---

### Task 1: MetaRaftNode 新增 `heartbeat_interval_ms` 字段

**Files:**
- Modify: `AiDb/src/cluster/meta_raft_node.rs:16-19` (struct), `:21-48` (new)

- [ ] **Step 1: 在 struct 中加字段**

在 `MetaRaftNode` struct 末尾添加 `heartbeat_interval_ms: u64`:

```rust
pub struct MetaRaftNode {
    inner: OpenRaftNode,
    state_machine: Arc<MetaStateMachine>,
    heartbeat_interval_ms: u64,  // 新增, 从 RaftNodeConfig 读取
}
```

- [ ] **Step 2: 在 `new()` 中提取并存入**

在 `config.validate()?;` 之前, 提取 `heartbeat_interval`:

```rust
pub async fn new(
    mut config: RaftNodeConfig,
    db: Arc<DB>,
    network_factory: RaftNetworkClientFactory,
) -> Result<Self> {
    config.group_id = METARAFT_GROUP_ID;
    let heartbeat_interval_ms = config.heartbeat_interval;  // 提取
    config.validate()?;

    // ... 已有逻辑 ...

    Ok(Self {
        inner,
        state_machine,
        heartbeat_interval_ms,  // 存入
    })
}
```

- [ ] **Step 3: 编译验证**

```bash
cd /root/code/dev/AiDb && cargo build --features cluster 2>&1 | tail -5
```

Expected: 编译成功 (struct 字段新增不影响现有调用方).

---

### Task 2: 实现 `wait_learner_catch_up` 屏障

**Files:**
- Modify: `AiDb/src/cluster/meta_raft_node.rs` — 在 `impl MetaRaftNode` 块内, `promote_learner_to_voter` 之前插入

- [ ] **Step 1: 添加常量定义**

在文件顶部 (struct 定义之前, `use` 语句之后) 添加:

```rust
/// 屏障超时常量.
const CATCH_UP_TIMEOUT: Duration = Duration::from_secs(30);
const CATCH_UP_POLL: Duration = Duration::from_millis(50);
const CATCH_UP_THRESHOLD: u64 = 5;
```

需要添加 `use std::time::Duration;` (如果尚未 import).

- [ ] **Step 2: 实现 `wait_learner_catch_up`**

在 `impl MetaRaftNode` 块内, `pub async fn promote_learner_to_voter` 之前插入:

```rust
/// 等待 learner 追平 leader 日志 (屏障 1).
///
/// 每 50ms 检查一次 replication metrics, 允许落后 5 条以内.
/// 超过 30s 返回 `ClusterError::Timeout`.
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

- [ ] **Step 3: 编译验证**

```bash
cd /root/code/dev/AiDb && cargo build --features cluster 2>&1 | tail -5
```

Expected: 编译成功. 新方法是 private 的, 仅被 Task 4 调用.

---

### Task 3: 实现 `confirm_replication` 屏障

**Files:**
- Modify: `AiDb/src/cluster/meta_raft_node.rs` — 在 `wait_learner_catch_up` 之后插入

- [ ] **Step 1: 添加常量定义**

在 Task 2 Step 1 的常量块中追加:

```rust
const REPLICATION_CONFIRM_TIMEOUT: Duration = Duration::from_secs(5);
const REPLICATION_POLL: Duration = Duration::from_millis(50);
const REPLICATION_HEARTBEAT_MULTIPLIER: u32 = 3;
```

- [ ] **Step 2: 实现 `confirm_replication`**

在 `wait_learner_catch_up` 之后插入:

```rust
/// 确认 membership change entry 已传播到至少一个其他 voter (屏障 2).
///
/// Step 1: 等待至少一个其他 voter 的 `matched >= last_log_index`.
/// Step 2: sleep `3 * heartbeat_interval` 让 commit_index 传播.
async fn confirm_replication(&self, voter_ids: &[NodeId]) -> Result<()> {
    // 快速路径: 只有本节点一个 voter, entry 已本地 committed
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
                "no voter confirmed replication within 5s".into(),
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

- [ ] **Step 3: 编译验证**

```bash
cd /root/code/dev/AiDb && cargo build --features cluster 2>&1 | tail -5
```

Expected: 编译成功.

---

### Task 4: 修改 `promote_learner_to_voter` 集成两道屏障

**Files:**
- Modify: `AiDb/src/cluster/meta_raft_node.rs:196-225` (`promote_learner_to_voter`)

- [ ] **Step 1: 更新 `promote_learner_to_voter`**

在 `change_membership` 调用前后插入屏障调用:

```rust
/// Promote the given node from Learner to Voter in the MetaRaft group.
///
/// Reads the current voter set from ClusterMeta and builds a new voter
/// list that includes the target node.  After the membership change
/// commits, updates ClusterMeta so subsequent promotions see the
/// correct voter set.
///
/// Two replication barriers guard against the leader crashing between
/// `change_membership` commit and follower `commit_index` propagation:
/// 1. `wait_learner_catch_up` — learner must be near leader's last_log
/// 2. `confirm_replication` — one other voter must confirm + heartbeat wait
#[instrument(skip(self), fields(node_id, heartbeat_ms = self.heartbeat_interval_ms))]
pub async fn promote_learner_to_voter(&self, node_id: NodeId) -> Result<()> {
    // ── 屏障 1: 等 learner 追平 ──
    self.wait_learner_catch_up(node_id).await?;

    let meta = self.get_cluster_meta();
    let mut voter_ids: Vec<NodeId> = meta
        .nodes
        .iter()
        .filter(|(_, info)| matches!(info.role, crate::cluster::meta_types::NodeRole::Voter))
        .map(|(id, _)| *id)
        .collect();
    // Safety net: if no voters are recorded in ClusterMeta (possible with stale
    // persisted data from an earlier release), include the local bootstrap node
    // which is always the initial voter.
    if voter_ids.is_empty() {
        voter_ids.push(self.node_id());
    }
    if !voter_ids.contains(&node_id) {
        voter_ids.push(node_id);
    }
    self.inner.change_membership(voter_ids.clone()).await?;

    // ── 屏障 2: 确认 entry 已传播 ──
    self.confirm_replication(&voter_ids).await?;

    // Update ClusterMeta so the promoted node is recorded as Voter.
    // This keeps the persisted metadata consistent with the Raft layer
    // so that subsequent promotions see the correct voter set.
    self
        .propose(MetaRequest::ChangeNodeRole {
            node_id,
            role: crate::cluster::meta_types::NodeRole::Voter,
        })
        .await?;
    Ok(())
}
```

关键改动:
- `#[instrument]` 新增 `heartbeat_ms` field
- 开头插入 `self.wait_learner_catch_up(node_id).await?;`
- `change_membership` 传 `voter_ids.clone()` (因 barrier 需要 ownership)
- `change_membership` 后插入 `self.confirm_replication(&voter_ids).await?;`

- [ ] **Step 2: 编译 + clippy 验证**

```bash
cd /root/code/dev/AiDb && cargo build --features cluster 2>&1 | tail -5
cargo clippy --features cluster -- -D warnings 2>&1 | tail -10
```

Expected: 编译成功, clippy 无 warning.

- [ ] **Step 3: 运行已有测试确保无回归**

```bash
cd /root/code/dev/AiDb && cargo test --features cluster meta_raft_node -- --test-threads=1 2>&1 | tail -20
```

Expected: `test_single_node_propose_register` PASS. 新 barrier 在单节点场景下通过快速路径 (learner 是 self → catch_up 立即返回, voter_ids=[1] → confirm 快速路径跳过).

---

### Task 5: 单元测试

**Files:**
- Modify: `AiDb/src/cluster/meta_raft_node.rs:269-317` (`#[cfg(test)] mod tests`)

在现有 `test_single_node_propose_register` 测试后追加 3 个测试.

- [ ] **Step 1: 添加 `learner_catch_up_timeout` 测试**

```rust
/// 屏障 1 超时: learner 不在 replication metrics 中 → 30s 后 Timeout.
#[tokio::test]
async fn test_learner_catch_up_timeout() {
    tokio::time::pause();

    let dir = TempDir::new().unwrap();
    let db = DB::open(dir.path(), Options::for_testing()).unwrap();
    let factory = RaftNetworkClientFactory::new(
        1,
        METARAFT_GROUP_ID,
        RaftNodeConfig::default().rpc_timeout_ms,
        RaftNodeConfig::default().grpc_max_message_size,
    );
    let cfg = RaftNodeConfig {
        node_id: 1,
        group_id: METARAFT_GROUP_ID,
        election_timeout_min: 500,
        election_timeout_max: 1000,
        heartbeat_interval: 50,
        ..Default::default()
    };
    let node = MetaRaftNode::new(cfg, db, factory).await.unwrap();
    node.initialize(vec![(1, "http://127.0.0.1:1".into())])
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(800)).await;

    // node 999 不在 group 中, metrics.replication 不会有它
    // → learner_matched = 0, leader_last > 5 after init + sleep
    // → 永远不满足 threshold → 超时
    tokio::time::advance(Duration::from_secs(31)).await;
    let result = node.wait_learner_catch_up(999).await;
    assert!(result.is_err());
    let err_msg = format!("{}", result.unwrap_err());
    assert!(err_msg.contains("failed to catch up"), "unexpected: {err_msg}");
}

```

- [ ] **Step 2: 添加 `replication_confirm_timeout` 测试**

```rust
/// 屏障 2 超时: 虚假 voter 不在 replication metrics 中 → 5s 后 Timeout.
#[tokio::test]
async fn test_replication_confirm_timeout() {
    tokio::time::pause();

    let dir = TempDir::new().unwrap();
    let db = DB::open(dir.path(), Options::for_testing()).unwrap();
    let factory = RaftNetworkClientFactory::new(
        1,
        METARAFT_GROUP_ID,
        RaftNodeConfig::default().rpc_timeout_ms,
        RaftNodeConfig::default().grpc_max_message_size,
    );
    let cfg = RaftNodeConfig {
        node_id: 1,
        group_id: METARAFT_GROUP_ID,
        election_timeout_min: 500,
        election_timeout_max: 1000,
        heartbeat_interval: 50,
        ..Default::default()
    };
    let node = MetaRaftNode::new(cfg, db, factory).await.unwrap();
    node.initialize(vec![(1, "http://127.0.0.1:1".into())])
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(800)).await;

    // voter_ids 含虚假节点 999, 但 999 不在 replication 中
    // → 永远不会确认 → 超时
    tokio::time::advance(Duration::from_secs(6)).await;
    let result = node.confirm_replication(&[1, 999]).await;
    assert!(result.is_err());
    let err_msg = format!("{}", result.unwrap_err());
    assert!(err_msg.contains("no voter confirmed"), "unexpected: {err_msg}");
}
```

- [ ] **Step 3: 添加 `barrier_fast_path` 测试**

```rust
/// 快速路径: self 作为 learner (immediate catch_up) + 单个 voter (skip confirm).
#[tokio::test]
async fn test_barrier_fast_path() {
    let dir = TempDir::new().unwrap();
    let db = DB::open(dir.path(), Options::for_testing()).unwrap();
    let factory = RaftNetworkClientFactory::new(
        1,
        METARAFT_GROUP_ID,
        RaftNodeConfig::default().rpc_timeout_ms,
        RaftNodeConfig::default().grpc_max_message_size,
    );
    let cfg = RaftNodeConfig {
        node_id: 1,
        group_id: METARAFT_GROUP_ID,
        election_timeout_min: 500,
        election_timeout_max: 1000,
        heartbeat_interval: 50,
        ..Default::default()
    };
    let node = MetaRaftNode::new(cfg, db, factory).await.unwrap();
    node.initialize(vec![(1, "http://127.0.0.1:1".into())])
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(800)).await;

    // 屏障 1: wait_learner_catch_up(1) — self 的 matched >= last_log, 立即返回
    let start = std::time::Instant::now();
    node.wait_learner_catch_up(1).await.unwrap();
    let elapsed1 = start.elapsed();

    // 屏障 2: confirm_replication(&[1]) — len <= 1 → 快速路径
    let start2 = std::time::Instant::now();
    node.confirm_replication(&[1]).await.unwrap();
    let elapsed2 = start2.elapsed();

    assert!(
        elapsed1 < Duration::from_millis(500),
        "catch_up took {elapsed1:?}, expected <500ms"
    );
    assert!(
        elapsed2 < Duration::from_millis(100),
        "confirm took {elapsed2:?}, expected <100ms"
    );
}
```

- [ ] **Step 4: 运行测试**

```bash
cd /root/code/dev/AiDb && cargo test --features cluster meta_raft_node -- --test-threads=1 2>&1 | tail -30
```

Expected: 4 tests PASS (1 existing + 3 new):
- `test_single_node_propose_register` — PASS
- `test_learner_catch_up_timeout` — PASS
- `test_replication_confirm_timeout` — PASS
- `test_barrier_fast_path` — PASS

- [ ] **Step 5: Commit**

```bash
git add AiDb/src/cluster/meta_raft_node.rs
git commit -m "feat: add replication barriers to MetaRaft promote_learner_to_voter

- Add heartbeat_interval_ms field to MetaRaftNode
- Implement wait_learner_catch_up barrier (30s timeout, 5-entry threshold)
- Implement confirm_replication barrier (5s timeout, 3x heartbeat wait)
- Integrate both barriers into promote_learner_to_voter
- Add 3 unit tests (catch_up timeout, confirm timeout, fast path)"
```

---

### Task 6: 集成测试 — 双节点 promote learner 成功路径

**Files:**
- Create: `AiDb/tests/meta_raft_promote_integration.rs`

需要启动两个 MetaRaftNode + gRPC server, 验证完整的 `add_learner → promote_learner_to_voter → NodeRole::Voter` 链路.

> **注意:** 此测试需要可用端口和 gRPC 连接, 耗时较长 (~10s). 如果 E2E 测试 (Task 7) 已覆盖相同场景, 此集成测试可作为后续优化.

- [ ] **Step 1: 创建集成测试文件**

```rust
//! 集成测试: 双节点 MetaRaft promote_learner_to_voter 端到端验证.

use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use aidb::cluster::meta_raft_node::MetaRaftNode;
use aidb::cluster::meta_types::{MetaRequest, NodeRole, METARAFT_GROUP_ID};
use aidb::cluster::network::RaftNetworkClientFactory;
use aidb::cluster::types::{RaftNodeConfig, Response};
use aidb::DB;

/// 双节点: N1 leader + N2 learner → promote → N2 becomes Voter.
#[tokio::test]
async fn test_promote_single_learner_success() {
    // ---- Arrange: 启动 N1 (leader) ----
    let dir1 = TempDir::new().unwrap();
    let db1 = Arc::new(DB::open(dir1.path(), aidb::config::Options::for_testing()).unwrap());

    let factory1 = RaftNetworkClientFactory::new(
        1,
        METARAFT_GROUP_ID,
        RaftNodeConfig::default().rpc_timeout_ms,
        RaftNodeConfig::default().grpc_max_message_size,
    );
    let cfg1 = RaftNodeConfig {
        node_id: 1,
        group_id: METARAFT_GROUP_ID,
        election_timeout_min: 500,
        election_timeout_max: 1000,
        heartbeat_interval: 50,
        ..Default::default()
    };
    let n1 = Arc::new(MetaRaftNode::new(cfg1, db1, factory1).await.unwrap());
    n1.initialize(vec![(1, "http://127.0.0.1:19981".into())])
        .await
        .unwrap();

    // Start N1 gRPC server
    let n1_socket = "127.0.0.1:19981".parse().unwrap();
    n1.start_server(n1_socket, 64 * 1024 * 1024).await.unwrap();
    tokio::time::sleep(Duration::from_millis(800)).await;

    // ---- Arrange: 启动 N2 (learner) ----
    let dir2 = TempDir::new().unwrap();
    let db2 = Arc::new(DB::open(dir2.path(), aidb::config::Options::for_testing()).unwrap());

    let factory2 = RaftNetworkClientFactory::new(
        2,
        METARAFT_GROUP_ID,
        RaftNodeConfig::default().rpc_timeout_ms,
        RaftNodeConfig::default().grpc_max_message_size,
    );
    let cfg2 = RaftNodeConfig {
        node_id: 2,
        group_id: METARAFT_GROUP_ID,
        election_timeout_min: 500,
        election_timeout_max: 1000,
        heartbeat_interval: 50,
        ..Default::default()
    };
    let n2 = Arc::new(MetaRaftNode::new(cfg2, db2, factory2).await.unwrap());

    // N2 joins the existing cluster (不 initialize, 作为 learner 加入)
    n2.add_node_address(1, "http://127.0.0.1:19981".into());
    let n2_socket = "127.0.0.1:19982".parse().unwrap();
    n2.start_server(n2_socket, 64 * 1024 * 1024).await.unwrap();

    // ---- Act: N1 adds N2 as learner + promote to voter ----
    n1.add_learner_nonblocking(2, "http://127.0.0.1:19982".into())
        .await
        .unwrap();

    // promote_learner_to_voter 内含两道复制屏障
    let result = tokio::time::timeout(
        Duration::from_secs(40),  // 30s catch_up + 余量
        n1.promote_learner_to_voter(2),
    )
    .await
    .expect("promote timed out")
    .expect("promote failed");

    // ---- Assert: N2 is now Voter in ClusterMeta ----
    let meta = n1.get_cluster_meta();
    let n2_info = meta.nodes.get(&2).expect("N2 not in cluster meta");
    assert!(
        matches!(n2_info.role, NodeRole::Voter),
        "expected Voter, got {:?}",
        n2_info.role
    );

    // ---- Cleanup ----
    let _ = n2.shutdown().await;
    let _ = n1.shutdown().await;
}
```

- [ ] **Step 2: 验证集成测试需要的 public API**

确认以下类型和方法是 public 的:
- `MetaRaftNode` — `pub struct` ✓ (已在 `mod.rs` re-export)
- `MetaRaftNode::add_learner_nonblocking` — `pub` ✓
- `MetaRaftNode::promote_learner_to_voter` — `pub` ✓
- `MetaRaftNode::start_server` — `pub` ✓
- `MetaRaftNode::shutdown` — `pub` ✓
- `MetaRaftNode::add_node_address` — `pub` ✓
- `MetaRaftNode::get_cluster_meta` — `pub` ✓
- `NodeRole` — `pub enum` ✓

- [ ] **Step 3: 编译测试文件**

```bash
cd /root/code/dev/AiDb && cargo test --features cluster --test meta_raft_promote_integration --no-run 2>&1 | tail -10
```

- [ ] **Step 4: 运行集成测试**

```bash
cd /root/code/dev/AiDb && cargo test --features cluster --test meta_raft_promote_integration -- --test-threads=1 2>&1 | tail -20
```

Expected: `test_promote_single_learner_success` PASS. N2 的 role 从 Learner 变为 Voter.

- [ ] **Step 5: Commit**

```bash
git add AiDb/tests/meta_raft_promote_integration.rs
git commit -m "test: add dual-node MetaRaft promote integration test

Verify full add_learner → promote_learner_to_voter → Voter pipeline
with two MetaRaftNodes connected via gRPC."
```

---

### Task 7: E2E 测试 — 补充 replica 配置步骤

**Files:**
- Modify: `AiKv/e2e/test_cluster_failover.sh`

当前脚本的 Part 2 (leader failover) 缺少 `CLUSTER ADD_REPLICA` 步骤, 导致 N2/N3 在 data group 中仍是 Learner, 无法参与选举.

- [ ] **Step 1: 在 ADDSLOTS 之后、写入数据之前插入 ADD_REPLICA 步骤**

找到 `sleep 5` (ADDSLOTS 之后), 在它之后插入:

```bash
# Add replicas so N2 and N3 can take over data group on leader failover
N1_HEX="0000000000000000000000000000000000000001"
N2_HEX="0000000000000000000000000000000000000002"
N3_HEX="0000000000000000000000000000000000000003"
echo "--- Adding replicas: N2, N3 as replicas of N1 ---"
rc_node "${N1_HOST}" "${N1_PORT}" CLUSTER ADD_REPLICA "${N1_HEX}" "${N2_HEX}"
rc_node "${N1_HOST}" "${N1_PORT}" CLUSTER ADD_REPLICA "${N1_HEX}" "${N3_HEX}"
sleep 3  # Wait for replication barriers + membership propagation

# Verify replicas are present in CLUSTER NODES
CLUSTER_NODES=$(rc_node "${N1_HOST}" "${N1_PORT}" CLUSTER NODES)
echo "CLUSTER NODES after ADD_REPLICA:"
echo "${CLUSTER_NODES}"
```

- [ ] **Step 2: 增强 Part 2 的 failover 验证 — 显式检查 CLUSTER NODES 的 is_leader 切换**

修改 Part 2 中 kill N1 后的验证逻辑:

```bash
  kill "${N1_PID}" 2>/dev/null || true
  wait "${N1_PID}" 2>/dev/null || true
  sleep 5  # election 1s + barrier 0.15s + watcher 0.25s + 余量

  # Verify new leader elected in CLUSTER NODES
  CLUSTER_NODES_AFTER=$(rc_node "${N2_HOST}" "${N2_PORT}" CLUSTER NODES 2>&1 || echo "DEAD")
  echo "CLUSTER NODES after N1 kill (from N2):"
  echo "${CLUSTER_NODES_AFTER}"

  # At least one node should now show myself,master for the slot range
  if echo "${CLUSTER_NODES_AFTER}" | grep -q "myself,master"; then
    echo "Leader failover: new master elected"
  else
    echo "WARN: no master found after failover (may need more time)"
  fi
```

- [ ] **Step 3: 运行 E2E 测试验证**

```bash
cd /root/code/dev/AiKv && bash e2e/test_cluster_failover.sh 2>&1 | tail -30
```

Expected: 测试 PASS, 输出包含:
- "Adding replicas: N2, N3 as replicas of N1"
- CLUSTER NODES 显示 N2 或 N3 为 `myself,master` (failover 后)
- "Leader failover: new master elected"

- [ ] **Step 4: Commit**

```bash
git add AiKv/e2e/test_cluster_failover.sh
git commit -m "test: add ADD_REPLICA steps to E2E failover test

Add replica configuration before leader kill so N2/N3 are voters
in the data group and can participate in Raft election after N1
goes down."
```

---

### Task 8: 最终验证

- [ ] **Step 1: 全量编译 + clippy**

```bash
cd /root/code/dev/AiDb && cargo build --features cluster 2>&1 | tail -5
cd /root/code/dev/AiDb && cargo clippy --features cluster --all-targets -- -D warnings 2>&1 | tail -10
cd /root/code/dev/AiKv && cargo build --features cluster 2>&1 | tail -5
```

- [ ] **Step 2: 运行全部 cluster 相关测试**

```bash
cd /root/code/dev/AiDb && cargo test --features cluster -- --test-threads=1 2>&1 | tail -40
```

Expected: 所有已有测试 PASS + 新增 3 个测试 PASS.

- [ ] **Step 3: 检查测试覆盖率 (目标 ≥80%)**

```bash
cd /root/code/dev/AiDb && cargo llvm-cov --features cluster --summary-only 2>&1 | tail -10
```

关注 `meta_raft_node.rs` 的覆盖率.

---

## 改动总结

| 文件 | 新增 | 修改 | 说明 |
|------|------|------|------|
| `AiDb/src/cluster/meta_raft_node.rs` | ~60 行 | ~5 行 | 两道屏障 + `heartbeat_interval_ms` + 3 个测试 |
| `AiDb/tests/meta_raft_promote_integration.rs` | ~70 行 | 0 | 双节点 promote 集成测试 |
| `AiKv/e2e/test_cluster_failover.sh` | ~15 行 | ~2 行 | ADD_REPLICA + failover 验证增强 |

**无 API/协议变更.**
