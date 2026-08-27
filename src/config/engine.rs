use clap::ValueEnum;
use serde::{Deserialize, Serialize};

/// 存储引擎种类 (memory / aidb).
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
pub enum EngineKind {
    #[serde(rename = "memory")]
    Memory,
    #[serde(rename = "aidb")]
    #[value(name = "aidb")]
    AiDb,
}
