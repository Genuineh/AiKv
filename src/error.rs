//! 错误类型

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O 错误: {0}")]
    Io(#[from] std::io::Error),

    #[error("协议错误: {0}")]
    Protocol(String),

    #[error("命令错误: {0}")]
    Command(String),

    #[error("存储错误: {0}")]
    Storage(String),

    #[error("配置错误: {0}")]
    Config(String),

    /// 集群错误 (仅在 cluster feature 下使用)
    #[cfg(feature = "cluster")]
    #[error("集群错误: {0}")]
    Cluster(ClusterError),
}

/// 集群错误子类型 — 封装 AiDb 的 ClusterError, 消除字符串 catch-all
#[cfg(feature = "cluster")]
#[derive(Debug, thiserror::Error)]
pub enum ClusterError {
    #[error("{0}")]
    Aidb(#[from] aidb::error::ClusterError),
}

#[cfg(feature = "cluster")]
impl From<ClusterError> for Error {
    fn from(e: ClusterError) -> Self {
        Error::Cluster(e)
    }
}

#[cfg(feature = "cluster")]
impl From<aidb::error::ClusterError> for Error {
    fn from(e: aidb::error::ClusterError) -> Self {
        Error::Cluster(ClusterError::Aidb(e))
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display() {
        let e = Error::Protocol("invalid frame".to_string());
        assert!(e.to_string().contains("invalid frame"));
    }
}
