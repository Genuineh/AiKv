//! AiKv 内部 DUMP 编解码 (非 Redis 兼容)

use crate::error::{Error, Result};
use crate::storage::StoredValue;

pub const DUMP_VERSION: u8 = 0;

pub fn encode(value: &StoredValue) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(1 + 64);
    out.push(DUMP_VERSION);
    bincode::serialize_into(&mut out, value)
        .map_err(|e| Error::Storage(format!("DUMP encode failed: {e}")))?;
    Ok(out)
}

pub fn decode(payload: &[u8]) -> Result<StoredValue> {
    if payload.is_empty() {
        return Err(Error::Command(
            "ERR DUMP payload version or checksum error".into(),
        ));
    }
    let version = payload[0];
    if version != DUMP_VERSION {
        return Err(Error::Command(
            "ERR DUMP payload version or checksum error".into(),
        ));
    }
    bincode::deserialize(&payload[1..])
        .map_err(|_| Error::Command("ERR DUMP payload version or checksum error".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::ValueType;

    #[test]
    fn test_dump_roundtrip() {
        let value = StoredValue {
            value: ValueType::String(b"hello".to_vec()),
            expires_at: Some(1_700_000_000_000),
        };
        let encoded = encode(&value).unwrap();
        assert_eq!(encoded[0], DUMP_VERSION);
        let decoded = decode(&encoded).unwrap();
        assert_eq!(decoded, value);
    }

    #[test]
    fn test_dump_version_check() {
        let err = decode(&[1, 0, 1, 2]).unwrap_err();
        assert!(matches!(err, Error::Command(_)));
        let err = decode(&[]).unwrap_err();
        assert!(matches!(err, Error::Command(_)));
    }
}
