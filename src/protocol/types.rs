//! RESP 值类型

use bytes::Bytes;

/// RESP2 + RESP3 协议值
#[derive(Debug, Clone, PartialEq)]
pub enum RespValue {
    // RESP2
    SimpleString(String),
    Error(String),
    Integer(i64),
    BulkString(Option<Bytes>),
    Array(Option<Vec<RespValue>>),

    // RESP3
    Null,
    Boolean(bool),
    Double(f64),
    BigNumber(String),
    BulkError(String),
    VerbatimString {
        format: String,
        data: Bytes,
    },
    Map(Vec<(RespValue, RespValue)>),
    Set(Vec<RespValue>),
    Push(Vec<RespValue>),
    Attribute {
        attributes: Vec<(RespValue, RespValue)>,
        data: Box<RespValue>,
    },
    StreamedString(Vec<Bytes>),
}

/// 协议版本, 默认 RESP3。
/// 客户端可通过 `HELLO 2` 回退到 RESP2 兼容模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProtocolVersion {
    Resp2,
    #[default]
    Resp3,
}

impl ProtocolVersion {
    pub fn as_u8(self) -> u8 {
        match self {
            Self::Resp2 => 2,
            Self::Resp3 => 3,
        }
    }
}
