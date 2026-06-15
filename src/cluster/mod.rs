//! 集群协议适配层: 桥接 AiDb 集群基础设施与 RESP 协议处理.

pub mod announce;
pub mod config_auto_save;
pub mod connection;
pub mod forward;
pub mod gossip;
pub mod replication;
pub mod router;
pub mod state;

pub use announce::{AnnounceMode, AnnounceResolver};

mod commands;
pub use commands::*;

// Re-export AiDb cluster utilities used by examples and external code.
pub use config_auto_save::ConfigAutoSave;
pub use aidb::cluster::{extract_hash_tag, key_to_slot};
