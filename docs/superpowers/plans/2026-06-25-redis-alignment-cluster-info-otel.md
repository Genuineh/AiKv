# Redis 8.8 对齐 (集群 + INFO + OTel) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 使 AiKv 集群 MOVED/ASK、INFO commandstats、OTel 导出与 **Redis Open Source 8.8** 官方模型一致; 移除透明转发; INFO 为监控真源.

**Architecture:** P0 改 `cluster_route` 仅返回 MOVED/ASK 且不计 commandstats; P1 扩展 `CommandTotals` + `InfoRenderer` 输出 8.8 commandstats 行; P2 新增 `info_catalog` 在 `refresh_runtime_metrics` 同步 OTLP `aikv_*`. 命名保持 `aikv_*`, reference 文档对照 redis_exporter.

**Tech Stack:** Rust, Tokio, OpenTelemetry SDK, 现有 `ServerMetrics` / `InfoRenderer` / `tests/modules/*`

**Spec:** [../specs/2026-06-25-redis-alignment-cluster-info-otel-design.md](../specs/2026-06-25-redis-alignment-cluster-info-otel-design.md)

**Status:** P0–P2 **Implemented** (2026-06-25); P3 sync-only hot path optional, 未做.

---

## File map

| File | Responsibility |
|------|----------------|
| `src/cluster/routing_key.rs` | **Create** — `cluster_routing_key` (从 forward.rs 迁出) |
| `src/cluster/forward.rs` | **Delete** — 透明 TCP 转发 |
| `src/cluster/mod.rs` | `forward` → `routing_key` |
| `src/command/router.rs` | MOVED/ASK 直接 Error; 去掉 cluster 路径 `record_command_outcome` |
| `src/server/connection.rs` | MOVED/ASK 跳过 `record_command_observability` |
| `src/server/metrics.rs` | `CommandTotals` 8.8 字段; slowlog 聚合 hook |
| `src/server/info.rs` | `REDIS_COMPAT_VERSION=8.8`; commandstats 8 字段行 |
| `src/server/info_catalog.rs` | **Create** — P0 INFO↔OTel 条目 + sync |
| `src/server/config.rs` | `refresh_runtime_metrics` 调用 otel sync |
| `src/server/mod.rs` | 导出 `info_catalog` (monitoring 下) |
| `tests/fixtures/redis88_info_p0_fields.txt` | **Create** — 自 redis7 复制改注释 |
| `tests/modules/command/info_golden.rs` | 引用 redis88 fixture |
| `tests/modules/command/info_alignment.rs` | 8.8 commandstats 断言 |
| `tests/modules/cluster/redirect_metrics.rs` | **Create** — MOVED 不计 cmdstat |
| `docs/modules/cluster.md` | 删透明转发描述 |
| `docs/modules/observability-reference.md` | INFO ↔ aikv_* ↔ redis_exporter 表 |
| `CHANGELOG.md` | breaking: 去透明转发 |
| `DEPLOYMENT.md` | 集群客户端需 `redis-cli -c` |

---

## Task 1: 迁移 `cluster_routing_key`, 删除 `forward.rs`

**Files:**
- Create: `src/cluster/routing_key.rs`
- Modify: `src/cluster/mod.rs`
- Delete: `src/cluster/forward.rs`
- Modify: `src/command/router.rs` (import 路径)

- [ ] **Step 1: 创建 routing_key.rs**

将 `src/cluster/forward.rs` 中 `cluster_routing_key` 及 `#[cfg(test)] mod tests` 原样迁入:

```rust
//! 集群 slot 路由 key 提取 (EVAL/EVALSHA 首 key 等).

pub fn cluster_routing_key<'a>(cmd: &str, args: &'a [bytes::Bytes]) -> Option<&'a [u8]> {
    // 与现 forward.rs:77-92 相同
}
```

- [ ] **Step 2: 更新 mod.rs**

```rust
// 删: pub mod forward;
pub mod routing_key;
```

- [ ] **Step 3: 更新 router.rs import**

```rust
// 改: crate::cluster::forward::cluster_routing_key
// 为: crate::cluster::routing_key::cluster_routing_key
```

- [ ] **Step 4: 删除 forward.rs**

- [ ] **Step 5: 编译验证**

```bash
cd /root/code/workspace/aikv
export RUSTFLAGS='-D warnings'
cargo check --features cluster
```

Expected: PASS (router 仍引用 forward_command, 下一步修)

- [ ] **Step 6: Commit**

```bash
git add src/cluster/routing_key.rs src/cluster/mod.rs src/command/router.rs
git rm src/cluster/forward.rs
git commit -m "refactor(cluster): move cluster_routing_key out of forward.rs"
```

---

## Task 2: MOVED/ASK 直接返回, 移除透明转发 (P0)

**Files:**
- Modify: `src/command/router.rs:349-405`

- [ ] **Step 1: 替换 Moved/Ask 分支**

```rust
RouteDecision::Moved { slot, addr, .. } => {
    if let Some(m) = self.metrics.as_ref() {
        m.on_cluster_redirect("moved");
    }
    Some(Ok(RespValue::Error(format!("MOVED {slot} {addr}"))))
}
RouteDecision::Ask { slot, addr, .. } => {
    if let Some(m) = self.metrics.as_ref() {
        m.on_cluster_redirect("ask");
    }
    Some(Ok(RespValue::Error(format!("ASK {slot} {addr}"))))
}
```

删除 `connect_addr` / `forward_command` 整块.

- [ ] **Step 2: cluster 早返回不再 record_command_outcome**

```rust
if let Some(result) = self.cluster_route(cmd, args, state).await {
    // 删: record_command_outcome(&self.metrics, cmd, &result);
    return result;
}
```

- [ ] **Step 3: 编译**

```bash
cargo check --features cluster
```

Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src/command/router.rs
git commit -m "fix(cluster): return MOVED/ASK instead of transparent forward (Redis 8.8)"
```

---

## Task 3: Connection 跳过 MOVED/ASK observability (P0)

**Files:**
- Modify: `src/server/connection.rs`

- [ ] **Step 1: 添加 helper (connection.rs 内 private fn)**

```rust
fn is_cluster_redirect_response(resp: &RespValue) -> bool {
    matches!(
        resp,
        RespValue::Error(msg) if msg.starts_with("MOVED ") || msg.starts_with("ASK ")
    )
}
```

- [ ] **Step 2: 在 `Ok(resp)` 分支跳过 redirect**

找到 `record_command_observability` 调用处, 改为:

```rust
if let Some(start) = started {
    if !is_cluster_redirect_response(&resp) {
        self.record_command_observability(cmd, &arg_strings, start, true);
    }
}
```

- [ ] **Step 3: 跑 server 测试**

```bash
cargo test -p aikv --test server --features cluster -- --test-threads=1 2>&1 | tail -20
```

Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src/server/connection.rs
git commit -m "fix(observability): skip latency/cmd duration on MOVED/ASK responses"
```

---

## Task 4: MOVED 不计 commandstats — 回归测试 (P0)

**Files:**
- Create: `tests/modules/cluster/redirect_metrics.rs`
- Modify: `tests/modules/cluster/mod.rs`

- [ ] **Step 1: 写失败测试**

`tests/modules/cluster/redirect_metrics.rs`:

```rust
//! MOVED/ASK 不 increment commandstats (Redis 8.8).

#![cfg(feature = "cluster")]

use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use bytes::Bytes;
use parking_lot::RwLock;

use aidb::cluster::meta_types::{default_slot_table, SlotStatus};
use aidb::cluster::{MultiRaftNode, RaftServiceDispatcher, Router};

use aikv::cluster::announce::AnnounceResolver;
use aikv::cluster::connection::ClusterConnectionState;
use aikv::cluster::state::{
    ClusterStateManager, ReplicationRole, CLUSTER_STATE_MGR, DEFAULT_DATA_PORT_OFFSET,
};
use aikv::command::router::CommandRouter;
use aikv::protocol::RespValue;
use aikv::server::metrics::ServerMetrics;

async fn setup_router_on_node_2() -> (CommandRouter, Arc<ServerMetrics>) {
    // 复用 cluster_routing.rs / cluster_integration.rs 模式:
    // node_id=2, slot 0 assigned to group 1 (node 1 leader), local group NOT overridden
    // ... (完整 setup 见 cluster_routing.rs route_decision_all_branches 前半)
    todo!("wire ClusterStateManager + CommandRouter with metrics")
}

#[tokio::test]
async fn moved_response_does_not_increment_get_commandstats() {
    let (router, metrics) = setup_router_on_node_2().await;
    let mut db = 0;
    let state = ClusterConnectionState::default();
    let key = Bytes::from_static(b"\x00\x00"); // slot 0, not local on node 2

    let before = metrics.command_ok_count("GET");
    let resp = router
        .execute_with_client(
            "GET",
            &[key],
            &mut db,
            None,
            None,
            aikv::protocol::ProtocolVersion::Resp2,
            Some(&state),
        )
        .await
        .unwrap();

    match resp {
        RespValue::Error(e) => assert!(e.starts_with("MOVED "), "got {e}"),
        other => panic!("expected MOVED error, got {other:?}"),
    }
    assert_eq!(metrics.command_ok_count("GET"), before);
    assert_eq!(metrics.total_commands_processed(), 0);
}
```

实现 `setup_router_on_node_2` 时: `node_id: 2`, `multi_raft.override_group_local` **不要** 设为 true for group 1, slot 0 → group 1 → MOVED.

- [ ] **Step 2: mod.rs 注册**

```rust
mod redirect_metrics;
```

- [ ] **Step 3: 跑测试 (应 FAIL 若 before 逻辑已 fix 则 PASS)**

```bash
cargo test -p aikv redirect_metrics --features cluster -- --test-threads=1
```

- [ ] **Step 4: 若 FAIL, Task 2/3 已合入后应 PASS; Commit**

```bash
git add tests/modules/cluster/redirect_metrics.rs tests/modules/cluster/mod.rs
git commit -m "test(cluster): MOVED must not increment commandstats"
```

---

## Task 5: P0 文档 (cluster.md, CHANGELOG, DEPLOYMENT)

**Files:**
- Modify: `docs/modules/cluster.md`
- Modify: `CHANGELOG.md`
- Modify: `DEPLOYMENT.md`

- [ ] **Step 1: cluster.md** — 删 `forward_command` 图/段; 写「MOVED/ASK 仅字符串, 客户端 `-c`」

- [ ] **Step 2: CHANGELOG** — `### Changed` breaking: 移除透明转发; 集群客户端须 cluster-aware

- [ ] **Step 3: DEPLOYMENT** — 集群一节加: `redis-cli -c` / smart client 必须

- [ ] **Step 4: Commit**

```bash
git add docs/modules/cluster.md CHANGELOG.md DEPLOYMENT.md
git commit -m "docs: Redis 8.8 cluster MOVED-only, remove transparent forward"
```

---

## Task 6: `CommandTotals` + commandstats 8.8 行 (P1)

**Files:**
- Modify: `src/server/metrics.rs` (`CommandTotals`)
- Modify: `src/server/info.rs` (`render_commandstats`, `REDIS_COMPAT_VERSION`)
- Modify: `src/server/connection.rs` (slowlog → metrics)
- Test: `tests/modules/command/info_alignment.rs`

- [ ] **Step 1: 扩展 CommandTotals**

```rust
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct CommandTotals {
    pub(crate) ok: u64,
    pub(crate) err: u64,
    pub(crate) usec: u64,
    pub(crate) rejected: u64,
    pub(crate) slowlog_count: u64,
    pub(crate) slowlog_time_ms_sum: u64,
    pub(crate) slowlog_time_ms_max: u64,
}
```

`failed_calls` = `err` (INFO 输出时用 `totals.err`).

- [ ] **Step 2: slowlog 钩子**

在 `record_command_observability` 当 `duration_us >= threshold` 时:

```rust
self.state.metrics().on_slowlog_command(cmd, duration_us);
```

`ServerMetrics::on_slowlog_command`:

```rust
pub fn on_slowlog_command(&self, command: &str, duration_us: u64) {
    let ms = duration_us / 1000;
    let mut map = self.commands_total.lock().unwrap();
    let entry = map.entry(command.to_ascii_uppercase()).or_default();
    entry.slowlog_count += 1;
    entry.slowlog_time_ms_sum = entry.slowlog_time_ms_sum.saturating_add(ms);
    entry.slowlog_time_ms_max = entry.slowlog_time_ms_max.max(ms);
}
```

- [ ] **Step 3: render_commandstats 8.8 格式**

```rust
out.push_str(&format!(
    "cmdstat_{}:calls={calls},usec={},usec_per_call={usec_per_call:.2},\
     rejected_calls={},failed_calls={},\
     slowlog_count={},slowlog_time_ms_sum={},slowlog_time_ms_max={}\r\n",
    cmd.to_ascii_lowercase(),
    totals.usec,
    totals.rejected,
    totals.err,
    totals.slowlog_count,
    totals.slowlog_time_ms_sum,
    totals.slowlog_time_ms_max,
));
```

- [ ] **Step 4: REDIS_COMPAT_VERSION**

```rust
const REDIS_COMPAT_VERSION: &str = "8.8";
```

- [ ] **Step 5: 更新 info_alignment 测试**

```rust
assert!(text.contains("rejected_calls=0,failed_calls=0"));
assert!(text.contains("slowlog_count=0,slowlog_time_ms_sum=0,slowlog_time_ms_max=0"));
```

慢查询触发后 assert slowlog_count >= 1 (可选同文件新 test).

- [ ] **Step 6: 跑测试**

```bash
cargo test -p aikv info_commandstats info_alignment -- --test-threads=1
```

- [ ] **Step 7: Commit**

```bash
git add src/server/metrics.rs src/server/info.rs src/server/connection.rs tests/modules/command/info_alignment.rs
git commit -m "feat(info): Redis 8.8 commandstats format and compatible version 8.8"
```

---

## Task 7: Fixture redis88 + info_golden (P1)

**Files:**
- Create: `tests/fixtures/redis88_info_p0_fields.txt`
- Modify: `tests/modules/command/info_golden.rs`
- Modify: `tests/modules/command/info_alignment.rs` (compatible version 断言)

- [ ] **Step 1: 复制 fixture**

```bash
cp tests/fixtures/redis7_info_p0_fields.txt tests/fixtures/redis88_info_p0_fields.txt
```

首行注释改为 `# Redis 8.8 INFO P0 字段清单`.

- [ ] **Step 2: info_golden 改 include**

```rust
const INFO_P0_FIELDS: &str = include_str!("../../fixtures/redis88_info_p0_fields.txt");
```

测试名 `info_redis8_all_p0_fields_present`; 断言 `redis_compatible_version:8.8`.

- [ ] **Step 3: info_alignment default section**

```rust
assert!(text.contains("redis_compatible_version:8.8"));
```

- [ ] **Step 4: 跑 golden**

```bash
cargo test -p aikv info_golden info_alignment -- --test-threads=1
```

- [ ] **Step 5: Commit**

```bash
git add tests/fixtures/redis88_info_p0_fields.txt tests/modules/command/info_golden.rs tests/modules/command/info_alignment.rs
git commit -m "test(info): golden fixture for Redis 8.8 P0 fields"
```

---

## Task 8: `info_catalog` + OTel sync (P2)

**Files:**
- Create: `src/server/info_catalog.rs`
- Modify: `src/server/config.rs`
- Modify: `src/server/otel_metrics.rs`
- Modify: `src/server/mod.rs`
- Test: `tests/modules/server/observability.rs`

- [x] **Step 1: info_catalog.rs 骨架**

```rust
//! INFO P0 字段 ↔ OTel 同步 catalog (Redis 8.8 基线).

use crate::server::metrics::ServerMetrics;

#[cfg(feature = "monitoring")]
use std::sync::Arc;
#[cfg(feature = "monitoring")]
use crate::server::otel_metrics::OtelMetrics;

pub fn sync_otel_from_server_metrics(
    metrics: &ServerMetrics,
    #[cfg(feature = "monitoring")] otel: Option<&Arc<OtelMetrics>>,
) {
    #[cfg(feature = "monitoring")]
    if let Some(otel) = otel {
        otel.sync_stats_gauges(metrics);
        otel.sync_commandstats(metrics);
    }
    let _ = metrics;
}
```

- [x] **Step 2: OtelMetrics 新增 sync 方法**

`sync_stats_gauges`: 读 `keyspace_hits`, `used_memory`, `connected_clients` 等 — 与现有 observable 回调数值一致 (可复用 gauge snapshot 写入).

`sync_commandstats`: 遍历 `client_command_totals()`, 对每个 cmd 设置 cumulative counter 到当前 calls (若 OTel SDK 不支持 set, 用 delta-from-last-sync 内部状态 — 见现 `on_command` 路径, P2 可先 **assert 一致** 再 P3 删热路径).

**P2 最小可行:** 在 `sync_commandstats` 内 debug_assert INFO calls == otel counter; 文档 catalog 表; 热路径保留.

- [x] **Step 3: refresh_runtime_metrics 末尾**

```rust
#[cfg(feature = "monitoring")]
crate::server::info_catalog::sync_otel_from_server_metrics(
    &self.metrics,
    self.metrics.otel_handle(), // 若无 accessor, 在 ServerMetrics 加 otel Option 只读 getter
);
```

若 `ServerMetrics` 无 otel getter, 在 `ServerSharedState` 传 otel Arc.

- [x] **Step 4: 扩展 info_metrics_consistency 测试**

`tests/modules/server/observability.rs` — refresh 后比对 INFO stats 字段与 exporter gauge.

- [ ] **Step 5: Commit**

```bash
git add src/server/info_catalog.rs src/server/config.rs src/server/otel_metrics.rs src/server/mod.rs tests/modules/server/observability.rs
git commit -m "feat(observability): INFO catalog sync hook for OTLP mirrors"
```

---

## Task 9: observability-reference 三列 mapping (P2)

**Files:**
- Modify: `docs/modules/observability-reference.md`
- Modify: `docs/modules/observability.md` (一句交叉引用)

- [x] **Step 1: 增加章节 「INFO field ↔ aikv_* ↔ redis_exporter」**

覆盖 P0: stats (hits/misses/commands/net), memory, clients, commandstats 动态行, cluster redirects.

标注: redis_exporter 8.8 `slowlog_*` 字段 **可能尚未解析** — AiKV INFO 先对齐.

- [ ] **Step 2: Commit**

```bash
git add docs/modules/observability-reference.md docs/modules/observability.md
git commit -m "docs: INFO to aikv_* to redis_exporter mapping (Redis 8.8)"
```

---

## Task 10: 全量验证 + spec 状态

**Files:**
- Modify: `docs/superpowers/specs/2026-06-25-redis-alignment-cluster-info-otel-design.md` (status → Implemented 各阶段)

- [x] **Step 1: CI 本地等价**

```bash
cd /root/code/workspace/aikv
export RUSTFLAGS='-D warnings'
cargo fmt --check
cargo clippy --all-targets --features cluster
cargo test --features cluster -- --test-threads=1
```

- [x] **Step 2: 更新 spec status**

```markdown
**状态:** P0–P2 Implemented (P3 sync-only hot path optional, 未做)
```

- [ ] **Step 3: Commit (仅 spec, 若用户要求)**

---

## Task 11 (Optional P3): 热路径 OTel 收敛

**仅当 P2 sync 稳定后.**

- [ ] 将 `OtelMetrics::on_command` / `on_command_duration` 改为 debug-only 或移除, 仅 `sync_otel_from_server_metrics` 写入
- [ ] 跑全量 observability + cluster 测试
- [ ] Commit: `refactor(observability): OTLP commandstats from INFO sync only`

---

## Spec coverage self-review

| Spec 要求 | Task |
|-----------|------|
| G1 去透明转发 | Task 2 |
| G2 MOVED 不计 commandstats | Task 2, 3, 4 |
| G3 commandstats 8.8 八字段 | Task 6 |
| G4 compatible 8.8 | Task 6, 7 |
| G5 INFO P0 fixture | Task 7 |
| G6 OTel 镜像 INFO | Task 8 (P2 最小 + P3 完整) |
| G7 文档 mapping | Task 5, 9 |
| DESIGN.md | 已在 spec 前序提交; Task 5 补 cluster.md |

## Placeholder scan

无 TBD / 实现时补全 — Task 4 `setup_router_on_node_2` 需从 `cluster_routing.rs` 复制完整 setup (计划内已指明参照文件).

---

## Execution handoff

Plan complete and saved to `docs/superpowers/plans/2026-06-25-redis-alignment-cluster-info-otel.md`.

**Two execution options:**

1. **Subagent-Driven (recommended)** — 每 Task 派生子 agent, Task 间 review
2. **Inline Execution** — 本会话按 Task 1→11 连续实现, checkpoint 在 P0/P1/P2 末

**Which approach?**
