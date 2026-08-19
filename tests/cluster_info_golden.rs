//! @component aikv-cluster
//! CLUSTER INFO 输出 Redis 8.8 字段存在性 golden 测试 (独立 binary).
//!
//! 独立成 binary 的原因: 本测试需要 `CLUSTER_STATE_MGR.set(...)` (OnceLock,
//! 一次性不可逆). 若留在 `tests/commands.rs` binary 内, 会污染同进程
//! 所有后续命令测试 — 临时服务器会被误判为集群模式, 报
//! `slot N is not allocated to any group` (MIGRATE/JSON 8 测曾因此失败).
#[cfg(feature = "cluster")]
#[path = "modules/cluster/info_golden.rs"]
mod tests;
