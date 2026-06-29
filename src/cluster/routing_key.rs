//! 集群 slot 路由 key 提取 (EVAL/EVALSHA 首 key 等).

use bytes::Bytes;

/// EVAL/EVALSHA 的 slot 路由 key: `EVAL script numkeys key ...`
pub fn cluster_routing_key<'a>(cmd: &str, args: &'a [Bytes]) -> Option<&'a [u8]> {
    match cmd.to_ascii_lowercase().as_str() {
        "eval" | "evalsha" => {
            if args.len() < 3 {
                return None;
            }
            let numkeys = std::str::from_utf8(&args[1])
                .ok()
                .and_then(|s| s.parse::<usize>().ok())?;
            if args.len() < 2 + numkeys {
                return None;
            }
            Some(args[2].as_ref())
        }
        _ => args.first().map(|b| b.as_ref()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    #[test]
    fn eval_routing_key_uses_first_declared_key() {
        let args = [
            Bytes::from_static(b"return 1"),
            Bytes::from_static(b"1"),
            Bytes::from_static(b"mykey:{0}"),
            Bytes::from_static(b"0"),
        ];
        assert_eq!(cluster_routing_key("EVAL", &args), Some(&b"mykey:{0}"[..]));
    }

    #[test]
    fn get_routing_key_uses_first_arg() {
        let args = [Bytes::from_static(b"userkey")];
        assert_eq!(cluster_routing_key("GET", &args), Some(&b"userkey"[..]));
    }
}
