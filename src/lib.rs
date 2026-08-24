//! aikv: Redis RESP 协议兼容层
//!
//! 协议层与存储层完全解耦; Phase 9 交付 MemoryEngine + CommandRouter.
#![recursion_limit = "256"]

pub mod command;
pub mod config;
pub mod error;
pub mod protocol;
pub mod server;
pub mod storage;

#[cfg(feature = "cluster")]
pub mod cluster;

pub use error::{Error, Result};

#[cfg(feature = "cluster")]
pub use cluster::*;
