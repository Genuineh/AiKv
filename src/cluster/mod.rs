//! Cluster module for AiKv Redis Cluster protocol support.
//!
//! This module provides the Redis Cluster protocol adaptation layer,
//! wrapping AiDb's MultiRaft API to provide Redis Cluster compatibility.
//!
//! # Feature Flag
//!
//! This module is only available when the `cluster` feature is enabled:
//!
//! ```toml
//! [features]
//! cluster = ["aidb/raft-cluster"]
//! ```
//!
//! # Architecture
//!
//! The cluster module acts as a thin glue layer between Redis Cluster protocol
//! and AiDb's MultiRaft implementation:
//!
//! ```text
//! ┌─────────────────────────────────────────────┐
//! │         Redis Cluster Commands              │
//! │  (CLUSTER KEYSLOT, INFO, NODES, etc.)       │
//! └─────────────────────────────────────────────┘
//!                      │
//!                      ▼
//! ┌─────────────────────────────────────────────┐
//! │         AiKv Cluster Glue Layer             │
//! │  (ClusterNode, SlotRouter, Commands)        │
//! └─────────────────────────────────────────────┘
//!                      │
//!                      ▼
//! ┌─────────────────────────────────────────────┐
//! │         AiDb MultiRaft API                  │
//! │  (Router, MultiRaftNode, MetaRaftNode)      │
//! └─────────────────────────────────────────────┘
//! ```

mod commands;
mod node;
mod router;

pub use commands::ClusterCommands;
pub use node::ClusterNode;
pub use router::SlotRouter;

// Re-export constants from AiDb
#[cfg(feature = "cluster")]
pub use aidb::cluster::SLOT_COUNT;

/// Default slot count for Redis Cluster (16384 slots)
#[cfg(not(feature = "cluster"))]
pub const SLOT_COUNT: u16 = 16384;
