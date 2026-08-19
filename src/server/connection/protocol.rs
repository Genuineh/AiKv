//! 连接响应编码: 错误包装、TCP 写出、RESP3 null 适配.

use bytes::Bytes;
use tokio::io::AsyncWriteExt;
use tracing::instrument;

use crate::error::{Error, Result};
use crate::protocol::{ProtocolVersion, RespValue};

use super::Connection;

impl Connection {
    pub(super) async fn write_error(&mut self, err: &Error) -> Result<()> {
        let msg = match err {
            Error::Protocol(s) => format!("Protocol error: {s}"),
            Error::Command(s) => s.clone(),
            Error::Io(e) => return Err(Error::Io(std::io::Error::new(e.kind(), e.to_string()))),
            Error::Storage(s) => format!("Internal storage error: {s}"),
            Error::Config(s) => format!("CONFIG error: {s}"),
            #[cfg(feature = "cluster")]
            Error::Cluster(s) => format!("CLUSTER error: {s}"),
        };
        self.write_response(RespValue::Error(msg)).await
    }

    #[instrument(
        level = "debug",
        name = "kv_write",
        skip(self, value),
        fields(response_size)
    )]
    pub(super) async fn write_response(&mut self, value: RespValue) -> Result<()> {
        let bytes = self.encode(&value);
        tracing::Span::current().record("response_size", bytes.len());
        self.stream.write_all(&bytes).await?;
        self.state.metrics().on_net_output_bytes(bytes.len() as u64);
        tracing::debug!(response_size = bytes.len(), "kv.write.complete");
        Ok(())
    }

    #[instrument(
        level = "debug",
        name = "kv_encode",
        skip(self, value),
        fields(value_type)
    )]
    fn encode(&self, value: &RespValue) -> Bytes {
        // 仅 RESP3 协商后才做 null 适配, 否则直接序列化 (免克隆, F-034)
        let bytes = if self.protocol_negotiated && self.protocol_version == ProtocolVersion::Resp3 {
            self.adapt_null_to_resp3(value).serialize()
        } else {
            value.serialize()
        };
        tracing::debug!(encoded_size = bytes.len(), "kv.encode.complete");
        bytes
    }

    /// 递归检查 RespValue 树中是否包含 RESP2 风格的 null
    /// (BulkString(None) 或 Array(None)),用于免克隆短路 (F-034).
    fn contains_resp_null(value: &RespValue) -> bool {
        match value {
            RespValue::BulkString(None) | RespValue::Array(None) => true,
            RespValue::Array(Some(items)) => items.iter().any(Self::contains_resp_null),
            RespValue::Map(pairs) => pairs
                .iter()
                .any(|(k, v)| Self::contains_resp_null(k) || Self::contains_resp_null(v)),
            RespValue::Set(items) => items.iter().any(Self::contains_resp_null),
            RespValue::Push(items) => items.iter().any(Self::contains_resp_null),
            RespValue::Attribute { attributes, data } => {
                attributes
                    .iter()
                    .any(|(k, v)| Self::contains_resp_null(k) || Self::contains_resp_null(v))
                    || Self::contains_resp_null(data)
            }
            _ => false,
        }
    }

    /// RESP3 模式下将 RESP2 风格的 null 表示转为 RESP3 原生 Null.
    /// redis-py 8.0 的 RESP3 解析器对 `$-1\r\n` / `*-1\r\n` 处理有兼容性问题,
    /// 需使用 RESP3 原生 `_\r\n` (Null) 替代.
    /// RESP3 模式下将 RESP2 风格的 null 表示转为 RESP3 原生 Null.
    /// redis-py 8.0 的 RESP3 解析器对 `$-1\r\n` / `*-1\r\n` 处理有兼容性问题,
    /// 需使用 RESP3 原生 `_\r\n` (Null) 替代.
    fn adapt_null_to_resp3(&self, value: &RespValue) -> RespValue {
        match value {
            RespValue::BulkString(None) | RespValue::Array(None) => RespValue::Null,
            RespValue::Array(Some(items)) => {
                if !items.iter().any(Self::contains_resp_null) {
                    return value.clone();
                }
                RespValue::Array(Some(
                    items.iter().map(|v| self.adapt_null_to_resp3(v)).collect(),
                ))
            }
            RespValue::Map(pairs) => {
                if !pairs
                    .iter()
                    .any(|(k, v)| Self::contains_resp_null(k) || Self::contains_resp_null(v))
                {
                    return value.clone();
                }
                RespValue::Map(
                    pairs
                        .iter()
                        .map(|(k, v)| (self.adapt_null_to_resp3(k), self.adapt_null_to_resp3(v)))
                        .collect(),
                )
            }
            RespValue::Set(items) => {
                if !items.iter().any(Self::contains_resp_null) {
                    return value.clone();
                }
                RespValue::Set(items.iter().map(|v| self.adapt_null_to_resp3(v)).collect())
            }
            RespValue::Push(items) => {
                if !items.iter().any(Self::contains_resp_null) {
                    return value.clone();
                }
                RespValue::Push(items.iter().map(|v| self.adapt_null_to_resp3(v)).collect())
            }
            RespValue::Attribute { attributes, data } => {
                if !attributes
                    .iter()
                    .any(|(k, v)| Self::contains_resp_null(k) || Self::contains_resp_null(v))
                    && !Self::contains_resp_null(data)
                {
                    return value.clone();
                }
                RespValue::Attribute {
                    attributes: attributes
                        .iter()
                        .map(|(k, v)| (self.adapt_null_to_resp3(k), self.adapt_null_to_resp3(v)))
                        .collect(),
                    data: Box::new(self.adapt_null_to_resp3(data)),
                }
            }
            other => other.clone(),
        }
    }
}
