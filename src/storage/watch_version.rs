//! WATCH 版本计数器 — 存储层 meta key, 随底层 WriteBatch/Raft 复制.

/// 保留前缀, 避免与用户 key 冲突; KEYS/SCAN 需过滤.
pub const WATCH_META_PREFIX: &[u8] = b"\xff\xff/aikv/watch/";

#[inline]
pub fn meta_user_key(user_key: &[u8]) -> Vec<u8> {
    let mut out = WATCH_META_PREFIX.to_vec();
    out.extend_from_slice(user_key);
    out
}

#[inline]
pub fn is_watch_meta_user_key(key: &[u8]) -> bool {
    key.starts_with(WATCH_META_PREFIX)
}

#[inline]
pub fn encode_version(version: u64) -> [u8; 8] {
    version.to_be_bytes()
}

#[inline]
pub fn decode_version(bytes: &[u8]) -> u64 {
    if bytes.len() >= 8 {
        u64::from_be_bytes(bytes[..8].try_into().expect("slice length checked"))
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_version_bytes() {
        assert_eq!(decode_version(&encode_version(42)), 42);
        assert_eq!(decode_version(&encode_version(0)), 0);
    }

    #[test]
    fn meta_prefix_detected() {
        let meta = meta_user_key(b"mykey");
        assert!(is_watch_meta_user_key(&meta));
        assert!(!is_watch_meta_user_key(b"mykey"));
    }
}
