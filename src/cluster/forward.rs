//! 跨分片命令透明转发 (单端点客户端无需处理 MOVED).

use bytes::{BufMut, Bytes, BytesMut};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::error::{Error, Result};
use crate::protocol::parser::RespParser;
use crate::protocol::RespValue;

const FORWARD_TIMEOUT_SECS: u64 = 30;

/// 将 RESP 命令转发到目标节点并返回响应.
pub async fn forward_command(
    addr: &str,
    send_asking: bool,
    cmd: &str,
    args: &[Bytes],
) -> Result<RespValue> {
    let mut stream = tokio::time::timeout(
        std::time::Duration::from_secs(FORWARD_TIMEOUT_SECS),
        TcpStream::connect(addr),
    )
    .await
    .map_err(|_| Error::Command(format!("ERR cluster forward timeout connecting to {addr}")))?
    .map_err(|e| Error::Command(format!("ERR cluster forward connect {addr}: {e}")))?;

    if send_asking {
        write_resp_array(&mut stream, "ASKING", &[]).await?;
        read_one_response(&mut stream).await?;
    }

    write_resp_array(&mut stream, cmd, args).await?;
    read_one_response(&mut stream).await
}

async fn write_resp_array(stream: &mut TcpStream, cmd: &str, args: &[Bytes]) -> Result<()> {
    let mut buf = BytesMut::new();
    let argc = 1 + args.len();
    buf.put_slice(format!("*{argc}\r\n").as_bytes());
    buf.put_slice(format!("${}\r\n", cmd.len()).as_bytes());
    buf.put_slice(cmd.as_bytes());
    buf.put_slice(b"\r\n");
    for arg in args {
        buf.put_slice(format!("${}\r\n", arg.len()).as_bytes());
        buf.put_slice(arg);
        buf.put_slice(b"\r\n");
    }
    stream
        .write_all(&buf)
        .await
        .map_err(|e| Error::Command(format!("ERR cluster forward write: {e}")))?;
    Ok(())
}

async fn read_one_response(stream: &mut TcpStream) -> Result<RespValue> {
    let mut parser = RespParser::new();
    let mut tmp = [0u8; 4096];
    loop {
        if let Some(value) = parser.parse()? {
            return Ok(value);
        }
        let n = stream
            .read(&mut tmp)
            .await
            .map_err(|e| Error::Command(format!("ERR cluster forward read: {e}")))?;
        if n == 0 {
            return Err(Error::Command(
                "ERR cluster forward: connection closed".into(),
            ));
        }
        parser.feed(&tmp[..n]);
    }
}

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
