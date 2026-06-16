use std::net::SocketAddr;
use std::time::Duration;

use aikv::protocol::{RespParser, RespValue};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time;

use super::helpers::start_ephemeral_server;

/// 解析 RESP 响应字节为 RespValue
fn parse_response(data: &[u8]) -> RespValue {
    let mut parser = RespParser::new();
    parser.feed(data);
    parser
        .parse()
        .expect("parse error")
        .expect("incomplete response")
}

/// 在同一个连接上发送多个命令，返回所有响应字节
struct TestConn {
    stream: TcpStream,
}

impl TestConn {
    async fn connect(addr: SocketAddr) -> Self {
        let stream = time::timeout(Duration::from_secs(3), TcpStream::connect(addr))
            .await
            .expect("connect timeout")
            .expect("connect failed");
        let mut this = Self { stream };
        // 裸 TCP 测试未协商协议版本，显式切换到 RESP2 以保持测试断言兼容
        let hello_resp = this.send("HELLO", &["2"]).await;
        assert!(
            hello_resp.starts_with(b"*"),
            "HELLO 2 应返回 RESP2 flat array"
        );
        this
    }

    async fn send(&mut self, cmd: &str, args: &[&str]) -> Vec<u8> {
        let mut frame = format!("*{}\r\n", 1 + args.len());
        frame.push_str(&format!("${}\r\n{}\r\n", cmd.len(), cmd));
        for arg in args {
            frame.push_str(&format!("${}\r\n{}\r\n", arg.len(), arg));
        }
        self.stream
            .write_all(frame.as_bytes())
            .await
            .expect("write failed");

        let mut buf = vec![0u8; 4096];
        let n = time::timeout(Duration::from_secs(3), self.stream.read(&mut buf))
            .await
            .expect("read timeout")
            .expect("read failed");
        buf.truncate(n);
        buf
    }
}

// ─── 测试 ─────────────────────────────────────────────────────

#[tokio::test]
async fn test_standard_multi_exec_alias() {
    let (addr, _server) = start_ephemeral_server().await;
    let mut conn = TestConn::connect(addr).await;

    let resp = conn.send("MULTI", &[]).await;
    assert_eq!(parse_response(&resp), RespValue::SimpleString("OK".into()));

    let resp = conn.send("SET", &["sk", "1"]).await;
    assert_eq!(
        parse_response(&resp),
        RespValue::SimpleString("QUEUED".into())
    );

    let resp = conn.send("EXEC", &[]).await;
    match parse_response(&resp) {
        RespValue::Array(Some(items)) => {
            assert_eq!(items.len(), 1);
            assert_eq!(items[0], RespValue::SimpleString("OK".into()));
        }
        other => panic!("expected Array, got {:?}", other),
    }

    let resp = conn.send("GET", &["sk"]).await;
    assert_eq!(
        parse_response(&resp),
        RespValue::BulkString(Some("1".into()))
    );
}

#[tokio::test]
async fn test_standard_watch_conflict() {
    let (addr, _server) = start_ephemeral_server().await;
    let mut conn1 = TestConn::connect(addr).await;
    let mut conn2 = TestConn::connect(addr).await;

    let _ = conn1.send("SET", &["wk", "v"]).await;

    let resp = conn2.send("WATCH", &["wk"]).await;
    assert_eq!(parse_response(&resp), RespValue::SimpleString("OK".into()));

    let _ = conn1.send("SET", &["wk", "v2"]).await;

    let _ = conn2.send("MULTI", &[]).await;
    let _ = conn2.send("SET", &["wk", "v3"]).await;
    let resp = conn2.send("EXEC", &[]).await;
    assert_eq!(
        parse_response(&resp),
        RespValue::BulkString(None),
        "WATCH conflict should return nil"
    );
}

#[tokio::test]
async fn test_atom_multi_exec_basic() {
    let (addr, _server) = start_ephemeral_server().await;
    let mut conn = TestConn::connect(addr).await;

    // SET a 1, GET a, INCR a — in transaction
    let resp = conn.send("ATOM.MULTI", &[]).await;
    assert_eq!(parse_response(&resp), RespValue::SimpleString("OK".into()));

    let resp = conn.send("SET", &["a", "1"]).await;
    assert_eq!(
        parse_response(&resp),
        RespValue::SimpleString("QUEUED".into())
    );

    let resp = conn.send("GET", &["a"]).await;
    assert_eq!(
        parse_response(&resp),
        RespValue::SimpleString("QUEUED".into())
    );

    let resp = conn.send("INCR", &["a"]).await;
    assert_eq!(
        parse_response(&resp),
        RespValue::SimpleString("QUEUED".into())
    );

    // EXEC
    let resp = conn.send("ATOM.EXEC", &[]).await;
    let parsed = parse_response(&resp);
    match parsed {
        RespValue::Array(Some(items)) => {
            assert_eq!(items.len(), 3, "expected 3 results");
            assert_eq!(items[0], RespValue::SimpleString("OK".into()));
            assert_eq!(items[1], RespValue::BulkString(Some("1".into())));
            assert_eq!(items[2], RespValue::Integer(2));
        }
        other => panic!("expected Array, got {:?}", other),
    }

    // Verify the value is 2 (INCR 1 → 2)
    let resp = conn.send("GET", &["a"]).await;
    assert_eq!(
        parse_response(&resp),
        RespValue::BulkString(Some("2".into()))
    );
}

#[tokio::test]
async fn test_atom_discard() {
    let (addr, _server) = start_ephemeral_server().await;
    let mut conn = TestConn::connect(addr).await;

    // MULTI → SET a 1 → DISCARD
    let _ = conn.send("ATOM.MULTI", &[]).await;
    let _ = conn.send("SET", &["a", "1"]).await;
    let resp = conn.send("ATOM.DISCARD", &[]).await;
    assert_eq!(parse_response(&resp), RespValue::SimpleString("OK".into()));

    // After DISCARD, SET a should not have been applied
    let resp = conn.send("GET", &["a"]).await;
    assert_eq!(parse_response(&resp), RespValue::BulkString(None));
}

#[tokio::test]
async fn test_atom_multi_nesting_error() {
    let (addr, _server) = start_ephemeral_server().await;
    let mut conn = TestConn::connect(addr).await;

    let _ = conn.send("ATOM.MULTI", &[]).await;
    let resp = conn.send("ATOM.MULTI", &[]).await;
    let parsed = parse_response(&resp);
    assert!(
        matches!(parsed, RespValue::Error(ref msg) if msg.contains("MULTI calls can not be nested")),
        "expected MULTI nested error, got {:?}",
        parsed
    );
}

#[tokio::test]
async fn test_atom_exec_empty() {
    let (addr, _server) = start_ephemeral_server().await;
    let mut conn = TestConn::connect(addr).await;

    let _ = conn.send("ATOM.MULTI", &[]).await;
    let resp = conn.send("ATOM.EXEC", &[]).await;
    let parsed = parse_response(&resp);
    assert_eq!(parsed, RespValue::Array(Some(vec![])));
}

#[tokio::test]
async fn test_atom_exec_without_multi() {
    let (addr, _server) = start_ephemeral_server().await;
    let mut conn = TestConn::connect(addr).await;

    let resp = conn.send("ATOM.EXEC", &[]).await;
    let parsed = parse_response(&resp);
    assert!(
        matches!(parsed, RespValue::Error(ref msg) if msg.contains("EXEC without MULTI")),
        "expected EXEC without MULTI error, got {:?}",
        parsed
    );
}

#[tokio::test]
async fn test_atom_discard_without_multi() {
    let (addr, _server) = start_ephemeral_server().await;
    let mut conn = TestConn::connect(addr).await;

    let resp = conn.send("ATOM.DISCARD", &[]).await;
    let parsed = parse_response(&resp);
    assert!(
        matches!(parsed, RespValue::Error(ref msg) if msg.contains("DISCARD without MULTI")),
        "expected DISCARD without MULTI error, got {:?}",
        parsed
    );
}

#[tokio::test]
async fn test_atom_watch_conflict() {
    let (addr, _server) = start_ephemeral_server().await;
    let mut conn1 = TestConn::connect(addr).await;
    let mut conn2 = TestConn::connect(addr).await;

    // conn1: SET k v
    let _ = conn1.send("SET", &["k", "v"]).await;

    // conn2: WATCH k
    let resp = conn2.send("ATOM.WATCH", &["k"]).await;
    assert_eq!(parse_response(&resp), RespValue::SimpleString("OK".into()));

    // conn1: modify k (triggers version change)
    let _ = conn1.send("SET", &["k", "v2"]).await;

    // conn2: MULTI → SET k v3 → EXEC (should fail due to WATCH conflict)
    let _ = conn2.send("ATOM.MULTI", &[]).await;
    let _ = conn2.send("SET", &["k", "v3"]).await;
    let resp = conn2.send("ATOM.EXEC", &[]).await;
    // Conflict → nil (transaction aborted)
    assert_eq!(
        parse_response(&resp),
        RespValue::BulkString(None),
        "WATCH conflict should return nil"
    );

    // k should still be v2 (conn2's SET was not executed)
    let resp = conn1.send("GET", &["k"]).await;
    assert_eq!(
        parse_response(&resp),
        RespValue::BulkString(Some("v2".into()))
    );
}

#[tokio::test]
async fn test_atom_watch_no_conflict() {
    let (addr, _server) = start_ephemeral_server().await;
    let mut conn1 = TestConn::connect(addr).await;
    let mut conn2 = TestConn::connect(addr).await;

    // conn1: SET k v
    let _ = conn1.send("SET", &["k", "v"]).await;

    // conn2: WATCH k
    let _ = conn2.send("ATOM.WATCH", &["k"]).await;

    // conn2: MULTI → GET k → EXEC (no conflict, k not modified by others)
    let _ = conn2.send("ATOM.MULTI", &[]).await;
    let _ = conn2.send("GET", &["k"]).await;
    let resp = conn2.send("ATOM.EXEC", &[]).await;
    let parsed = parse_response(&resp);
    match parsed {
        RespValue::Array(Some(items)) => {
            assert_eq!(items.len(), 1);
            assert_eq!(items[0], RespValue::BulkString(Some("v".into())));
        }
        other => panic!("expected Array, got {:?}", other),
    }
}

#[tokio::test]
async fn test_atom_watch_auto_clear_after_exec() {
    let (addr, _server) = start_ephemeral_server().await;
    let mut conn1 = TestConn::connect(addr).await;
    let mut conn2 = TestConn::connect(addr).await;

    // conn1: SET k v
    let _ = conn1.send("SET", &["k", "v"]).await;

    // conn2: WATCH k → MULTI → GET k → EXEC (successful, clears watch)
    let _ = conn2.send("ATOM.WATCH", &["k"]).await;
    let _ = conn2.send("ATOM.MULTI", &[]).await;
    let _ = conn2.send("GET", &["k"]).await;
    let _ = conn2.send("ATOM.EXEC", &[]).await;

    // conn1 modifies k (this should now NOT affect conn2's next EXEC)
    let _ = conn1.send("SET", &["k", "v3"]).await;

    // conn2: MULTI → GET k → EXEC (should succeed because WATCH was cleared)
    let _ = conn2.send("ATOM.MULTI", &[]).await;
    let _ = conn2.send("GET", &["k"]).await;
    let resp = conn2.send("ATOM.EXEC", &[]).await;
    let parsed = parse_response(&resp);
    match parsed {
        RespValue::Array(Some(items)) => {
            assert_eq!(items.len(), 1);
            assert_eq!(items[0], RespValue::BulkString(Some("v3".into())));
        }
        other => panic!("expected Array, got {:?}", other),
    }
}

#[tokio::test]
async fn test_atom_unwatch() {
    let (addr, _server) = start_ephemeral_server().await;
    let mut conn1 = TestConn::connect(addr).await;
    let mut conn2 = TestConn::connect(addr).await;

    // conn1: SET k v
    let _ = conn1.send("SET", &["k", "v"]).await;

    // conn2: WATCH k → UNWATCH
    let _ = conn2.send("ATOM.WATCH", &["k"]).await;
    let resp = conn2.send("ATOM.UNWATCH", &[]).await;
    assert_eq!(parse_response(&resp), RespValue::SimpleString("OK".into()));

    // conn1 modifies k
    let _ = conn1.send("SET", &["k", "v2"]).await;

    // conn2: MULTI → GET k → EXEC (should succeed because WATCH was removed)
    let _ = conn2.send("ATOM.MULTI", &[]).await;
    let _ = conn2.send("GET", &["k"]).await;
    let resp = conn2.send("ATOM.EXEC", &[]).await;
    let parsed = parse_response(&resp);
    match parsed {
        RespValue::Array(Some(items)) => {
            assert_eq!(items.len(), 1);
            assert_eq!(items[0], RespValue::BulkString(Some("v2".into())));
        }
        other => panic!("expected Array, got {:?}", other),
    }
}

#[tokio::test]
async fn test_atom_watch_multiple_keys() {
    let (addr, _server) = start_ephemeral_server().await;
    let mut conn1 = TestConn::connect(addr).await;
    let mut conn2 = TestConn::connect(addr).await;

    // conn2: WATCH k1 k2 k3
    let _ = conn2.send("ATOM.WATCH", &["k1", "k2", "k3"]).await;

    // conn1: modify k2 (one of the watched keys)
    let _ = conn1.send("SET", &["k2", "changed"]).await;

    // conn2: EXEC should fail
    let _ = conn2.send("ATOM.MULTI", &[]).await;
    let _ = conn2.send("SET", &["k1", "x"]).await;
    let resp = conn2.send("ATOM.EXEC", &[]).await;
    assert_eq!(
        parse_response(&resp),
        RespValue::BulkString(None),
        "WATCH on any key should abort transaction"
    );
}

#[tokio::test]
async fn test_atom_watch_discard_clears() {
    let (addr, _server) = start_ephemeral_server().await;
    let mut conn1 = TestConn::connect(addr).await;
    let mut conn2 = TestConn::connect(addr).await;

    // conn2: WATCH k
    let _ = conn2.send("ATOM.WATCH", &["k"]).await;
    let _ = conn2.send("ATOM.MULTI", &[]).await;
    let _ = conn2.send("SET", &["k", "x"]).await;

    // DISCARD clears both queue AND watched keys
    let _ = conn2.send("ATOM.DISCARD", &[]).await;

    // conn1 modifies k
    let _ = conn1.send("SET", &["k", "v1"]).await;

    // conn2: new MULTI → GET k → EXEC (should succeed, watched keys cleared by DISCARD)
    let _ = conn2.send("ATOM.MULTI", &[]).await;
    let _ = conn2.send("GET", &["k"]).await;
    let resp = conn2.send("ATOM.EXEC", &[]).await;
    let parsed = parse_response(&resp);
    match parsed {
        RespValue::Array(Some(items)) => {
            assert_eq!(items.len(), 1);
            assert_eq!(items[0], RespValue::BulkString(Some("v1".into())));
        }
        other => panic!("expected Array, got {:?}", other),
    }
}
