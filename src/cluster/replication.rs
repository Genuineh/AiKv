//! INFO replication 段与集群节点角色 (Redis 语义).

use aidb::cluster::meta_types::ClusterMeta;

use crate::cluster::commands::cluster_node_role_label;

/// 本节点在 CLUSTER NODES / INFO replication 中的 role 标签.
pub fn node_replication_role(meta: &ClusterMeta, node_id: u64) -> &'static str {
  cluster_node_role_label(meta, node_id)
}

/// 本节点作为 group leader 所关联的 replica 数量 (不含自身).
pub fn connected_slaves_count(meta: &ClusterMeta, node_id: u64) -> u64 {
  meta
    .groups
    .values()
    .filter(|group| {
      group
        .replicas
        .iter()
        .any(|r| r.node_id == node_id && r.is_leader)
    })
    .map(|group| {
      group
        .replicas
        .iter()
        .filter(|r| r.node_id != node_id)
        .count()
    })
    .sum::<usize>() as u64
}

#[cfg(test)]
mod tests {
  use std::collections::HashMap;

  use aidb::cluster::meta_types::{ClusterMeta, GroupMeta, ReplicaInfo};

  use super::*;

  fn sample_meta() -> ClusterMeta {
    ClusterMeta {
      cluster_id: "test".into(),
      nodes: HashMap::new(),
      groups: HashMap::from([(
        1,
        GroupMeta {
          group_id: 1,
          replicas: vec![
            ReplicaInfo {
              node_id: 1,
              is_leader: true,
            },
            ReplicaInfo {
              node_id: 2,
              is_leader: false,
            },
            ReplicaInfo {
              node_id: 3,
              is_leader: false,
            },
          ],
          slot_ranges: vec![(0, 8191)],
          config_version: 1,
        },
      )]),
      version: 1,
      format_version: 1,
    }
  }

  #[test]
  fn leader_node_is_master_with_two_slaves() {
    let meta = sample_meta();
    assert_eq!(node_replication_role(&meta, 1), "master");
    assert_eq!(connected_slaves_count(&meta, 1), 2);
  }

  #[test]
  fn replica_node_is_slave_with_zero_slaves() {
    let meta = sample_meta();
    assert_eq!(node_replication_role(&meta, 2), "slave");
    assert_eq!(connected_slaves_count(&meta, 2), 0);
  }
}
