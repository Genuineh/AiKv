#![recursion_limit = "512"]

/// AiKv 基础用法示例: 启动内嵌服务器, 通过 TCP 发送命令.
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use aikv::server::{ConnectionConfig, Server, ServerSharedState};
use aikv::storage::MemoryEngine;

fn encode_resp(parts: &[&str]) -> Vec<u8> {
  let mut out = format!("*{}\r\n", parts.len());
  for p in parts {
    out.push_str(&format!("${}\r\n", p.len()));
    out.push_str(p);
    out.push_str("\r\n");
  }
  out.into_bytes()
}

async fn read_response(stream: &mut TcpStream) -> String {
  let mut buf = vec![0u8; 4096];
  let n = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut buf))
    .await
    .expect("read timeout")
    .expect("read failed");
  buf.truncate(n);
  String::from_utf8_lossy(&buf).to_string()
}

#[tokio::main]
async fn main() {
  println!("=== AiKv 基础示例 ===\n");

  // 1. 启动内嵌服务器
  let storage: Arc<dyn aikv::storage::KvStorage> = MemoryEngine::new(16);
  let state = ServerSharedState::new(ConnectionConfig::default(), storage, 0);
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
    .await
    .expect("bind failed");
  let addr = listener.local_addr().unwrap();
  tokio::spawn(async move {
    let _ = Server::run_with_listener(listener, state).await;
  });
  tokio::time::sleep(Duration::from_millis(100)).await;
  println!("  服务器已启动: {}\n", addr);

  // 2. 连接服务器
  let mut stream = TcpStream::connect(addr).await.expect("connect failed");
  println!("  已连接服务器\n");

  // 3. PING
  stream.write_all(&encode_resp(&["PING"])).await.unwrap();
  let resp = read_response(&mut stream).await;
  println!("  PING -> {}", resp.trim());

  // 4. SET / GET
  stream
    .write_all(&encode_resp(&["SET", "hello", "world"]))
    .await
    .unwrap();
  let resp = read_response(&mut stream).await;
  println!("  SET hello world -> {}", resp.trim());

  stream
    .write_all(&encode_resp(&["GET", "hello"]))
    .await
    .unwrap();
  let resp = read_response(&mut stream).await;
  println!("  GET hello -> {}", resp.trim());

  // 5. SET with EXPIRE
  stream
    .write_all(&encode_resp(&["SET", "tmp", "val", "EX", "60"]))
    .await
    .unwrap();
  let resp = read_response(&mut stream).await;
  println!("  SET tmp val EX 60 -> {}", resp.trim());

  stream
    .write_all(&encode_resp(&["TTL", "tmp"]))
    .await
    .unwrap();
  let resp = read_response(&mut stream).await;
  println!("  TTL tmp -> {}", resp.trim());

  // 6. HSET / HGET
  stream
    .write_all(&encode_resp(&["HSET", "user:1", "name", "Alice"]))
    .await
    .unwrap();
  let resp = read_response(&mut stream).await;
  println!("  HSET user:1 name Alice -> {}", resp.trim());

  stream
    .write_all(&encode_resp(&["HGET", "user:1", "name"]))
    .await
    .unwrap();
  let resp = read_response(&mut stream).await;
  println!("  HGET user:1 name -> {}", resp.trim());

  // 7. INCR
  stream
    .write_all(&encode_resp(&["SET", "counter", "10"]))
    .await
    .unwrap();
  let _ = read_response(&mut stream).await;

  stream
    .write_all(&encode_resp(&["INCR", "counter"]))
    .await
    .unwrap();
  let resp = read_response(&mut stream).await;
  println!("  INCR counter -> {}", resp.trim());

  stream
    .write_all(&encode_resp(&["GET", "counter"]))
    .await
    .unwrap();
  let resp = read_response(&mut stream).await;
  println!("  GET counter -> {}", resp.trim());

  // 8. EXISTS / DEL
  stream
    .write_all(&encode_resp(&["EXISTS", "hello", "nonexist"]))
    .await
    .unwrap();
  let resp = read_response(&mut stream).await;
  println!("  EXISTS hello nonexist -> {}", resp.trim());

  stream
    .write_all(&encode_resp(&["DEL", "hello"]))
    .await
    .unwrap();
  let resp = read_response(&mut stream).await;
  println!("  DEL hello -> {}", resp.trim());

  // 9. INFO
  stream
    .write_all(&encode_resp(&["INFO", "server"]))
    .await
    .unwrap();
  let resp = read_response(&mut stream).await;
  println!("\n  INFO server (摘要):");
  for line in resp.lines().take(5) {
    println!("    {}", line.trim());
  }

  // 10. QUIT
  stream.write_all(&encode_resp(&["QUIT"])).await.unwrap();
  let resp = read_response(&mut stream).await;
  println!("\n  QUIT -> {}", resp.trim());

  println!("\n=== 示例完成 ===");
}
