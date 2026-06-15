//! MIGRATE localhost TCP client (RESTORE to target aikv)

use bytes::Bytes;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::{timeout, Duration};

use crate::error::{Error, Result};
use crate::protocol::{RespParser, RespValue};

fn encode_command(parts: &[&[u8]]) -> Vec<u8> {
  let items: Vec<RespValue> = parts
    .iter()
    .map(|p| RespValue::BulkString(Some(Bytes::copy_from_slice(p))))
    .collect();
  RespValue::Array(Some(items)).serialize().to_vec()
}

async fn read_one_response(stream: &mut TcpStream, dur: Duration) -> Result<RespValue> {
  let mut parser = RespParser::new();
  let mut tmp = [0u8; 4096];
  loop {
    if let Some(resp) = parser.parse()? {
      return Ok(resp);
    }
    let n = timeout(dur, stream.read(&mut tmp))
      .await
      .map_err(|_| Error::Command("ERR timeout".into()))?
      .map_err(Error::Io)?;
    if n == 0 {
      return Err(Error::Command("ERR connection closed".into()));
    }
    parser.feed(&tmp[..n]);
  }
}

fn ensure_ok(resp: RespValue) -> Result<()> {
  match resp {
    RespValue::SimpleString(s) if s == "OK" => Ok(()),
    RespValue::Error(e) => Err(Error::Command(e)),
    other => Err(Error::Command(format!(
      "ERR unexpected response: {other:?}"
    ))),
  }
}

/// RESTORE 目标连接参数
pub struct RestoreTarget<'a> {
  pub host: &'a str,
  pub port: u16,
  pub timeout_ms: u64,
  pub dest_db: usize,
  pub key: &'a [u8],
  pub ttl_ms: i64,
  pub payload: &'a [u8],
  pub replace: bool,
  pub auth_password: Option<&'a [u8]>,
}

/// 向目标 aikv 发送 SELECT (如需) 与 RESTORE.
pub async fn send_restore(target: RestoreTarget<'_>) -> Result<()> {
  let RestoreTarget {
    host,
    port,
    timeout_ms,
    dest_db,
    key,
    ttl_ms,
    payload,
    replace,
    auth_password,
  } = target;
  let addr = if host.contains(':') && !host.starts_with('[') {
    format!("[{host}]:{port}")
  } else {
    format!("{host}:{port}")
  };
  let dur = Duration::from_millis(timeout_ms.max(1));
  let mut stream = timeout(dur, TcpStream::connect(&addr))
    .await
    .map_err(|_| Error::Command("ERR timeout".into()))?
    .map_err(|e| Error::Command(format!("ERR {e}")))?;

  if let Some(password) = auth_password {
    let auth_frame = encode_command(&[b"AUTH", password]);
    stream.write_all(&auth_frame).await?;
    ensure_ok(read_one_response(&mut stream, dur).await?)?;
  }

  if dest_db != 0 {
    let db_arg = dest_db.to_string();
    let select_frame = encode_command(&[b"SELECT", db_arg.as_bytes()]);
    stream.write_all(&select_frame).await?;
    ensure_ok(read_one_response(&mut stream, dur).await?)?;
  }

  // Slot migration: destination may be IMPORTING; ASKING allows RESTORE on migrating slots.
  #[cfg(feature = "cluster")]
  {
    let asking_frame = encode_command(&[b"ASKING"]);
    stream.write_all(&asking_frame).await?;
    ensure_ok(read_one_response(&mut stream, dur).await?)?;
  }

  let ttl_arg = ttl_ms.to_string();
  let mut parts: Vec<&[u8]> = vec![b"RESTORE", key, ttl_arg.as_bytes(), payload];
  let replace_flag = b"REPLACE";
  if replace {
    parts.push(replace_flag);
  }
  let restore_frame = encode_command(&parts);
  stream.write_all(&restore_frame).await?;
  ensure_ok(read_one_response(&mut stream, dur).await?)
}
