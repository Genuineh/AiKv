//! RESP 编码器

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
        buf.put_slice(format!("{i}").as_bytes());
        buf.put_slice(b"\r\n");
      }
      RespValue::BulkString(None) => buf.put_slice(b"$-1\r\n"),
      RespValue::BulkString(Some(b)) => {
        buf.put_slice(b"$");
        buf.put_slice(format!("{}", b.len()).as_bytes());
        buf.put_slice(b"\r\n");
        buf.put_slice(b);
        buf.put_slice(b"\r\n");
      }
      RespValue::Array(None) => buf.put_slice(b"*-1\r\n"),
      RespValue::Array(Some(items)) => {
        buf.put_slice(b"*");
        buf.put_slice(format!("{}", items.len()).as_bytes());
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
          buf.put_slice(d.to_string().as_bytes());
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
        buf.put_slice(format!("{}", e.len()).as_bytes());
        buf.put_slice(b"\r\n");
        buf.put_slice(e.as_bytes());
        buf.put_slice(b"\r\n");
      }
      RespValue::VerbatimString { format, data } => {
        let total = format.len() + 1 + data.len();
        buf.put_slice(b"=");
        buf.put_slice(format!("{total}").as_bytes());
        buf.put_slice(b"\r\n");
        buf.put_slice(format.as_bytes());
        buf.put_slice(b":");
        buf.put_slice(data);
        buf.put_slice(b"\r\n");
      }
      RespValue::Map(pairs) => {
        buf.put_slice(b"%");
        buf.put_slice(format!("{}", pairs.len()).as_bytes());
        buf.put_slice(b"\r\n");
        for (k, v) in pairs {
          k.encode_into(buf);
          v.encode_into(buf);
        }
      }
      RespValue::Set(items) => {
        buf.put_slice(b"~");
        buf.put_slice(format!("{}", items.len()).as_bytes());
        buf.put_slice(b"\r\n");
        for item in items {
          item.encode_into(buf);
        }
      }
      RespValue::Push(items) => {
        buf.put_slice(b">");
        buf.put_slice(format!("{}", items.len()).as_bytes());
        buf.put_slice(b"\r\n");
        for item in items {
          item.encode_into(buf);
        }
      }
      RespValue::Attribute { attributes, data } => {
        buf.put_slice(b"|");
        buf.put_slice(format!("{}", attributes.len()).as_bytes());
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
          buf.put_slice(format!("{}", chunk.len()).as_bytes());
          buf.put_slice(b"\r\n");
          buf.put_slice(chunk);
          buf.put_slice(b"\r\n");
        }
        buf.put_slice(b";0\r\n");
      }
    }
  }
}
