//! RESP 编码器: `RespValue` → 字节帧 (`RespValue::serialize` / `encode_into`), 与
//! `parser.rs` 对称.
//!
//! # 编码约定
//!
//! - 每个变体输出 marker + 载荷 + `\r\n`; 容器变体 (Array / Map / Set / Push /
//!   Attribute / StreamedString) 递归 `encode_into`; bulk 载荷二进制安全
//!   (`bytes::Bytes` 原样拷贝).
//! - RESP2 null 线格式: `BulkString(None)` → `$-1\r\n`, `Array(None)` → `*-1\r\n`.
//! - 特判: `Double` 的 nan / inf / -inf / -0; `VerbatimString` 要求 3 字符 format + `:`.
//!
//! # Invariant
//!
//! - 与解析器对称: `serialize()` 输出可被 `RespParser::parse` 完整恢复 (roundtrip).
//! - `ProtocolVersion` 不影响 `serialize()` 输出; 版本相关 null 线格式适配
//!   (`$-1`/`*-1` → `_`) 在 `server/connection/protocol.rs` 的 `encode` / `adapt_null_to_resp3`.

use bytes::{BufMut, Bytes, BytesMut};

use crate::protocol::types::RespValue;

impl RespValue {
    /// 编码为 RESP 帧
    pub fn serialize(&self) -> Bytes {
        let mut buf = BytesMut::new();
        self.encode_into(&mut buf);
        buf.freeze()
    }

    fn encode_into(&self, buf: &mut BytesMut) {
        match self {
            RespValue::SimpleString(s) => {
                buf.put_slice(b"+");
                buf.put_slice(s.as_bytes());
                buf.put_slice(b"\r\n");
            }
            RespValue::Error(e) => {
                buf.put_slice(b"-");
                buf.put_slice(e.as_bytes());
                buf.put_slice(b"\r\n");
            }
            RespValue::Integer(i) => {
                buf.put_slice(b":");
                buf.put_slice(itoa::Buffer::new().format(*i).as_bytes());
                buf.put_slice(b"\r\n");
            }
            RespValue::BulkString(None) => buf.put_slice(b"$-1\r\n"),
            RespValue::BulkString(Some(b)) => {
                buf.put_slice(b"$");
                buf.put_slice(itoa::Buffer::new().format(b.len()).as_bytes());
                buf.put_slice(b"\r\n");
                buf.put_slice(b);
                buf.put_slice(b"\r\n");
            }
            RespValue::Array(None) => buf.put_slice(b"*-1\r\n"),
            RespValue::Array(Some(items)) => {
                buf.put_slice(b"*");
                buf.put_slice(itoa::Buffer::new().format(items.len()).as_bytes());
                buf.put_slice(b"\r\n");
                for item in items {
                    item.encode_into(buf);
                }
            }
            RespValue::Null => buf.put_slice(b"_\r\n"),
            RespValue::Boolean(b) => {
                if *b {
                    buf.put_slice(b"#t\r\n");
                } else {
                    buf.put_slice(b"#f\r\n");
                }
            }
            RespValue::Double(d) => {
                buf.put_slice(b",");
                if d.is_nan() {
                    buf.put_slice(b"nan");
                } else if *d == f64::INFINITY {
                    buf.put_slice(b"inf");
                } else if *d == f64::NEG_INFINITY {
                    buf.put_slice(b"-inf");
                } else if *d == 0.0 && d.is_sign_negative() {
                    buf.put_slice(b"-0");
                } else {
                    buf.put_slice(ryu::Buffer::new().format(*d).as_bytes());
                }
                buf.put_slice(b"\r\n");
            }
            RespValue::BigNumber(s) => {
                buf.put_slice(b"(");
                buf.put_slice(s.as_bytes());
                buf.put_slice(b"\r\n");
            }
            RespValue::BulkError(e) => {
                buf.put_slice(b"!");
                buf.put_slice(itoa::Buffer::new().format(e.len()).as_bytes());
                buf.put_slice(b"\r\n");
                buf.put_slice(e.as_bytes());
                buf.put_slice(b"\r\n");
            }
            RespValue::VerbatimString { format, data } => {
                let total = format.len() + 1 + data.len();
                buf.put_slice(b"=");
                buf.put_slice(itoa::Buffer::new().format(total).as_bytes());
                buf.put_slice(b"\r\n");
                buf.put_slice(format.as_bytes());
                buf.put_slice(b":");
                buf.put_slice(data);
                buf.put_slice(b"\r\n");
            }
            RespValue::Map(pairs) => {
                buf.put_slice(b"%");
                buf.put_slice(itoa::Buffer::new().format(pairs.len()).as_bytes());
                buf.put_slice(b"\r\n");
                for (k, v) in pairs {
                    k.encode_into(buf);
                    v.encode_into(buf);
                }
            }
            RespValue::Set(items) => {
                buf.put_slice(b"~");
                buf.put_slice(itoa::Buffer::new().format(items.len()).as_bytes());
                buf.put_slice(b"\r\n");
                for item in items {
                    item.encode_into(buf);
                }
            }
            RespValue::Push(items) => {
                buf.put_slice(b">");
                buf.put_slice(itoa::Buffer::new().format(items.len()).as_bytes());
                buf.put_slice(b"\r\n");
                for item in items {
                    item.encode_into(buf);
                }
            }
            RespValue::Attribute { attributes, data } => {
                buf.put_slice(b"|");
                buf.put_slice(itoa::Buffer::new().format(attributes.len()).as_bytes());
                buf.put_slice(b"\r\n");
                for (k, v) in attributes {
                    k.encode_into(buf);
                    v.encode_into(buf);
                }
                data.encode_into(buf);
            }
            RespValue::StreamedString(chunks) => {
                buf.put_slice(b"$?\r\n");
                for chunk in chunks {
                    buf.put_slice(b";");
                    buf.put_slice(itoa::Buffer::new().format(chunk.len()).as_bytes());
                    buf.put_slice(b"\r\n");
                    buf.put_slice(chunk);
                    buf.put_slice(b"\r\n");
                }
                buf.put_slice(b";0\r\n");
            }
        }
    }
}
