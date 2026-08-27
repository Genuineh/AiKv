//! @component aikv-cluster
//! FIX-0056-A1 aikv 侧合并读 / 迁移期写 集成测试.
//!
//! 使用真实的单节点 `MetaRaftNode` + 双 group (source=1, target=2) 的
//! `MultiRaftNode` (lifecycle 驱动本地 Raft group 创建), 通过
//! `ClusterDataAdapter` (`StorageAdapter` trait) 驱动 GET/SET/DEL, 验证:
//!
//! 1. Prepare/Migrating (Copying) 期 SET 未拷贝完的 key, GET 立即可见新值.
//! 2. target miss 的 DEL 仍一律 propose 并记录 tombstone, 之后 GET 不复活.
//! 3. DEL 后模拟 bulk-copy `PutConditional` 不复活; ReadyToCommit 后仍 miss.
//! 9. Frozen 合并读回归: 不再纯 source, 能看到只落在 target 的新写.
#![cfg(feature = "cluster")]
#[path = "modules/cluster/migration_merge_read.rs"]
mod tests;
