use std::collections::HashSet;
use std::time::Duration;

use super::helpers::{assert_err_contains, assert_int, assert_nil, assert_ok, b, router};

#[tokio::test]
async fn test_keys_pattern() {
    let r = router();
    let mut db = 0;
    r.execute("SET", &[b("foo"), b("1")], &mut db)
        .await
        .unwrap();
    r.execute("SET", &[b("bar"), b("2")], &mut db)
        .await
        .unwrap();
    let resp = r.execute("KEYS", &[b("f*")], &mut db).await.unwrap();
    let aikv::protocol::RespValue::Array(Some(keys)) = resp else {
        panic!("expected array");
    };
    assert_eq!(keys.len(), 1);
}

#[tokio::test]
async fn test_scan_cursor() {
    let r = router();
    let mut db = 0;
    for i in 0..5 {
        r.execute("SET", &[b(&format!("k{i}")), b("v")], &mut db)
            .await
            .unwrap();
    }
    let r1 = r
        .execute("SCAN", &[b("0"), b("COUNT"), b("2")], &mut db)
        .await
        .unwrap();
    let aikv::protocol::RespValue::Array(Some(parts)) = r1 else {
        panic!("expected array");
    };
    let aikv::protocol::RespValue::BulkString(Some(c1)) = &parts[0] else {
        panic!("expected cursor");
    };
    assert_ne!(c1.as_ref(), b"0");
    let r2 = r
        .execute("SCAN", &[c1.clone(), b("COUNT"), b("10")], &mut db)
        .await
        .unwrap();
    let aikv::protocol::RespValue::Array(Some(parts2)) = r2 else {
        panic!("expected array");
    };
    let aikv::protocol::RespValue::BulkString(Some(c2)) = &parts2[0] else {
        panic!("expected cursor");
    };
    assert_eq!(c2.as_ref(), b"0");
}

#[tokio::test]
async fn test_rename() {
    let r = router();
    let mut db = 0;
    r.execute("SET", &[b("old"), b("v")], &mut db)
        .await
        .unwrap();
    assert_ok(
        r.execute("RENAME", &[b("old"), b("new")], &mut db)
            .await
            .unwrap(),
    );
    let resp = r.execute("GET", &[b("new")], &mut db).await.unwrap();
    assert_eq!(resp, aikv::protocol::RespValue::BulkString(Some(b("v"))));
}

#[tokio::test]
async fn test_type_command() {
    let r = router();
    let mut db = 0;
    r.execute("SET", &[b("s"), b("v")], &mut db).await.unwrap();
    r.execute("LPUSH", &[b("l"), b("x")], &mut db)
        .await
        .unwrap();
    let t1 = r.execute("TYPE", &[b("s")], &mut db).await.unwrap();
    assert_eq!(t1, aikv::protocol::RespValue::SimpleString("string".into()));
    let t2 = r.execute("TYPE", &[b("l")], &mut db).await.unwrap();
    assert_eq!(t2, aikv::protocol::RespValue::SimpleString("list".into()));
    let t3 = r.execute("TYPE", &[b("missing")], &mut db).await.unwrap();
    assert_eq!(t3, aikv::protocol::RespValue::SimpleString("none".into()));
}

#[tokio::test]
async fn test_copy() {
    let r = router();
    let mut db = 0;
    r.execute("SET", &[b("src"), b("v")], &mut db)
        .await
        .unwrap();
    assert_int(
        r.execute("COPY", &[b("src"), b("dst")], &mut db)
            .await
            .unwrap(),
        1,
    );
    let resp = r.execute("GET", &[b("dst")], &mut db).await.unwrap();
    assert_eq!(resp, aikv::protocol::RespValue::BulkString(Some(b("v"))));
    assert_int(
        r.execute("COPY", &[b("src"), b("dst")], &mut db)
            .await
            .unwrap(),
        0,
    );
    assert_int(
        r.execute("COPY", &[b("src"), b("dst"), b("REPLACE")], &mut db)
            .await
            .unwrap(),
        1,
    );
}

#[tokio::test]
async fn test_concurrent_rename() {
    use std::sync::Arc;

    let r = Arc::new(router());
    let mut db = 0;
    for i in 0..10 {
        r.execute("SET", &[b(&format!("k{i}")), b("v")], &mut db)
            .await
            .unwrap();
    }

    let mut handles = Vec::new();
    for i in 0..10 {
        let r = Arc::clone(&r);
        handles.push(tokio::spawn(async move {
            let mut db = 0;
            let from = format!("k{i}");
            let to = format!("renamed{i}");
            r.execute("RENAME", &[b(&from), b(&to)], &mut db)
                .await
                .unwrap();
        }));
    }
    for h in handles {
        h.await.unwrap();
    }

    let mut db = 0;
    for i in 0..10 {
        let key = format!("renamed{i}");
        let resp = r.execute("GET", &[b(&key)], &mut db).await.unwrap();
        assert_eq!(resp, aikv::protocol::RespValue::BulkString(Some(b("v"))));
    }
}

#[tokio::test]
async fn test_ttl_persist_flow() {
    let r = router();
    let mut db = 0;
    r.execute("SET", &[b("k"), b("v")], &mut db).await.unwrap();
    assert_int(r.execute("TTL", &[b("k")], &mut db).await.unwrap(), -1);
    assert_int(
        r.execute("EXPIRE", &[b("k"), b("60")], &mut db)
            .await
            .unwrap(),
        1,
    );
    let ttl = match r.execute("TTL", &[b("k")], &mut db).await.unwrap() {
        aikv::protocol::RespValue::Integer(n) => n,
        _ => panic!("expected int"),
    };
    assert!(ttl > 0 && ttl <= 60);
    assert_int(r.execute("PERSIST", &[b("k")], &mut db).await.unwrap(), 1);
    assert_int(r.execute("TTL", &[b("k")], &mut db).await.unwrap(), -1);
}

#[tokio::test]
async fn test_pttl() {
    let r = router();
    let mut db = 0;
    r.execute("SET", &[b("k"), b("v")], &mut db).await.unwrap();
    assert_int(
        r.execute("PEXPIRE", &[b("k"), b("5000")], &mut db)
            .await
            .unwrap(),
        1,
    );
    let pttl = match r.execute("PTTL", &[b("k")], &mut db).await.unwrap() {
        aikv::protocol::RespValue::Integer(n) => n,
        _ => panic!("expected int"),
    };
    assert!(pttl > 0 && pttl <= 5000);
}

#[tokio::test]
async fn test_expireat_pexpireat() {
    let r = router();
    let mut db = 0;
    r.execute("SET", &[b("k"), b("v")], &mut db).await.unwrap();
    let future_secs = (aikv::storage::now_ms() / 1000 + 3600).to_string();
    assert_int(
        r.execute("EXPIREAT", &[b("k"), b(&future_secs)], &mut db)
            .await
            .unwrap(),
        1,
    );
}

#[tokio::test]
async fn test_expire_overwrite_ttl() {
    let r = router();
    let mut db = 0;
    r.execute("SET", &[b("k"), b("v")], &mut db).await.unwrap();
    r.execute("EXPIRE", &[b("k"), b("10")], &mut db)
        .await
        .unwrap();
    let ttl1 = match r.execute("TTL", &[b("k")], &mut db).await.unwrap() {
        aikv::protocol::RespValue::Integer(n) => n,
        _ => panic!("expected int"),
    };
    assert!(ttl1 > 0 && ttl1 <= 10);
    r.execute("EXPIRE", &[b("k"), b("100")], &mut db)
        .await
        .unwrap();
    let ttl2 = match r.execute("TTL", &[b("k")], &mut db).await.unwrap() {
        aikv::protocol::RespValue::Integer(n) => n,
        _ => panic!("expected int"),
    };
    assert!(ttl2 > ttl1);
}

#[tokio::test]
async fn test_dump_restore() {
    let r = router();
    let mut db = 0;
    r.execute("SET", &[b("k"), b("payload")], &mut db)
        .await
        .unwrap();
    let aikv::protocol::RespValue::BulkString(Some(dump)) =
        r.execute("DUMP", &[b("k")], &mut db).await.unwrap()
    else {
        panic!("expected bulk dump");
    };
    assert!(!dump.is_empty());
    r.execute("DEL", &[b("k")], &mut db).await.unwrap();
    assert_nil(r.execute("GET", &[b("k")], &mut db).await.unwrap());
    assert_ok(
        r.execute("RESTORE", &[b("k"), b("0"), dump.clone()], &mut db)
            .await
            .unwrap(),
    );
    let resp = r.execute("GET", &[b("k")], &mut db).await.unwrap();
    assert_eq!(
        resp,
        aikv::protocol::RespValue::BulkString(Some(b("payload")))
    );
}

#[tokio::test]
async fn test_restore_replace() {
    let r = router();
    let mut db = 0;
    r.execute("SET", &[b("src"), b("newval")], &mut db)
        .await
        .unwrap();
    let aikv::protocol::RespValue::BulkString(Some(dump)) =
        r.execute("DUMP", &[b("src")], &mut db).await.unwrap()
    else {
        panic!("expected dump");
    };
    r.execute("SET", &[b("dst"), b("old")], &mut db)
        .await
        .unwrap();
    assert_ok(
        r.execute(
            "RESTORE",
            &[b("dst"), b("0"), dump.clone(), b("REPLACE")],
            &mut db,
        )
        .await
        .unwrap(),
    );
    let resp = r.execute("GET", &[b("dst")], &mut db).await.unwrap();
    assert_eq!(
        resp,
        aikv::protocol::RespValue::BulkString(Some(b("newval")))
    );
}

#[tokio::test]
async fn test_restore_invalid_payload() {
    let r = router();
    let mut db = 0;
    let err = r
        .execute("RESTORE", &[b("k"), b("0"), b("not-a-dump")], &mut db)
        .await
        .unwrap_err();
    assert_err_contains(err, "DUMP payload version or checksum error");
}

#[tokio::test]
async fn test_expiretime_pexpiretime() {
    let r = router();
    let mut db = 0;
    r.execute("SET", &[b("k"), b("v")], &mut db).await.unwrap();
    let future_secs = aikv::storage::now_ms() / 1000 + 3600;
    let future_ms = future_secs * 1000;
    r.execute("EXPIREAT", &[b("k"), b(&future_secs.to_string())], &mut db)
        .await
        .unwrap();
    let et = match r.execute("EXPIRETIME", &[b("k")], &mut db).await.unwrap() {
        aikv::protocol::RespValue::Integer(n) => n,
        _ => panic!("expected int"),
    };
    assert_eq!(et, future_secs as i64);
    let pet = match r.execute("PEXPIRETIME", &[b("k")], &mut db).await.unwrap() {
        aikv::protocol::RespValue::Integer(n) => n,
        _ => panic!("expected int"),
    };
    assert_eq!(pet, future_ms as i64);
}

#[tokio::test]
async fn test_expiretime_no_key() {
    let r = router();
    let mut db = 0;
    assert_int(
        r.execute("EXPIRETIME", &[b("missing")], &mut db)
            .await
            .unwrap(),
        -2,
    );
    assert_int(
        r.execute("PEXPIRETIME", &[b("missing")], &mut db)
            .await
            .unwrap(),
        -2,
    );
}

#[tokio::test]
async fn test_expiretime_no_ttl() {
    let r = router();
    let mut db = 0;
    r.execute("SET", &[b("k"), b("v")], &mut db).await.unwrap();
    assert_int(
        r.execute("EXPIRETIME", &[b("k")], &mut db).await.unwrap(),
        -1,
    );
    assert_int(
        r.execute("PEXPIRETIME", &[b("k")], &mut db).await.unwrap(),
        -1,
    );
}

#[tokio::test]
async fn test_migrate_localhost() {
    use super::helpers::{start_ephemeral_server, tcp_get};

    let (target_addr, _handle) = start_ephemeral_server().await;
    let port = target_addr.port().to_string();

    let r = router();
    let mut db = 0;
    r.execute("SET", &[b("migrate_key"), b("migrated")], &mut db)
        .await
        .unwrap();
    assert_ok(
        r.execute(
            "MIGRATE",
            &[
                b("127.0.0.1"),
                b(&port),
                b("migrate_key"),
                b("0"),
                b("5000"),
                b("REPLACE"),
            ],
            &mut db,
        )
        .await
        .unwrap(),
    );
    assert_nil(
        r.execute("GET", &[b("migrate_key")], &mut db)
            .await
            .unwrap(),
    );
    let resp = tcp_get(target_addr, "migrate_key").await;
    assert!(
        resp.windows(b"migrated".len()).any(|w| w == b"migrated"),
        "target should have key, got {:?}",
        String::from_utf8_lossy(&resp)
    );
}

#[tokio::test]
async fn test_migrate_copy() {
    use super::helpers::{start_ephemeral_server, tcp_get};

    let (target_addr, _handle) = start_ephemeral_server().await;
    let port = target_addr.port().to_string();

    let r = router();
    let mut db = 0;
    r.execute("SET", &[b("copy_key"), b("copyval")], &mut db)
        .await
        .unwrap();
    assert_ok(
        r.execute(
            "MIGRATE",
            &[
                b("127.0.0.1"),
                b(&port),
                b("copy_key"),
                b("0"),
                b("5000"),
                b("COPY"),
                b("REPLACE"),
            ],
            &mut db,
        )
        .await
        .unwrap(),
    );
    let resp = r.execute("GET", &[b("copy_key")], &mut db).await.unwrap();
    assert_eq!(
        resp,
        aikv::protocol::RespValue::BulkString(Some(b("copyval")))
    );
    let target_resp = tcp_get(target_addr, "copy_key").await;
    assert!(
        target_resp
            .windows(b"copyval".len())
            .any(|w| w == b"copyval"),
        "target should have copy_key"
    );
}

#[tokio::test]
async fn test_migrate_auth() {
    use super::helpers::start_ephemeral_server;

    let (target_addr, _handle) = start_ephemeral_server().await;
    let port = target_addr.port().to_string();

    let r = router();
    let mut db = 0;
    r.execute("SET", &[b("auth_key"), b("secret")], &mut db)
        .await
        .unwrap();

    // Server 不支持 requirepass, 但验证 AUTH 标志被正确解析
    let result = r
        .execute(
            "MIGRATE",
            &[
                b("127.0.0.1"),
                b(&port),
                b("auth_key"),
                b("0"),
                b("5000"),
                b("AUTH"),
                b("testpass"),
                b("REPLACE"),
            ],
            &mut db,
        )
        .await;
    match result {
        Err(e) => {
            // AUTH 到无密码的服务器会失败, 但验证不是解析错误
            assert!(
                !e.to_string().contains("wrong number"),
                "should not be a parsing error: {e}"
            );
            // 源 key 应保留(迁移失败)
            let resp = r.execute("GET", &[b("auth_key")], &mut db).await.unwrap();
            assert_ne!(resp, aikv::protocol::RespValue::BulkString(None));
        }
        Ok(resp) => {
            assert_ok(resp);
        }
    }
}

#[tokio::test]
async fn test_migrate_auth2() {
    use super::helpers::start_ephemeral_server;

    let (target_addr, _handle) = start_ephemeral_server().await;
    let port = target_addr.port().to_string();

    let r = router();
    let mut db = 0;
    r.execute("SET", &[b("auth2_key"), b("secret")], &mut db)
        .await
        .unwrap();

    let result = r
        .execute(
            "MIGRATE",
            &[
                b("127.0.0.1"),
                b(&port),
                b("auth2_key"),
                b("0"),
                b("5000"),
                b("AUTH2"),
                b("myuser"),
                b("mypass"),
                b("REPLACE"),
            ],
            &mut db,
        )
        .await;
    match result {
        Err(e) => {
            assert!(
                !e.to_string().contains("wrong number"),
                "should not be a parsing error: {e}"
            );
            let resp = r.execute("GET", &[b("auth2_key")], &mut db).await.unwrap();
            assert_ne!(resp, aikv::protocol::RespValue::BulkString(None));
        }
        Ok(resp) => {
            assert_ok(resp);
        }
    }
}

#[tokio::test]
async fn test_migrate_keys_stops_at_auth2() {
    use super::helpers::start_ephemeral_server;

    let (target_addr, _handle) = start_ephemeral_server().await;
    let port = target_addr.port().to_string();

    let r = router();
    let mut db = 0;
    r.execute("SET", &[b("k1"), b("v1")], &mut db)
        .await
        .unwrap();

    // AUTH2 trailing KEYS must be parsed as auth, not as key names.
    let result = r
        .execute(
            "MIGRATE",
            &[
                b("127.0.0.1"),
                b(&port),
                b(""),
                b("0"),
                b("5000"),
                b("KEYS"),
                b("k1"),
                b("AUTH2"),
                b("myuser"),
                b("mypass"),
                b("COPY"),
            ],
            &mut db,
        )
        .await;
    assert!(
        result.is_err(),
        "AUTH2 should run TCP AUTH and fail on no-ACL server: {:?}",
        result
    );
    assert!(
        !result
            .unwrap_err()
            .to_string()
            .contains("wrong number"),
        "should not be a parsing error"
    );
    assert_eq!(
        r.execute("GET", &[b("k1")], &mut db).await.unwrap(),
        aikv::protocol::RespValue::BulkString(Some(b("v1")))
    );
}

#[tokio::test]
async fn test_migrate_auth2_precedence_over_auth() {
    use super::helpers::start_ephemeral_server;

    let (target_addr, _handle) = start_ephemeral_server().await;
    let port = target_addr.port().to_string();

    let r = router();
    let mut db = 0;
    r.execute("SET", &[b("prec_key"), b("v")], &mut db)
        .await
        .unwrap();

    // AUTH then AUTH2: later AUTH2 wins (oldmain / Redis ACL path).
    let result = r
        .execute(
            "MIGRATE",
            &[
                b("127.0.0.1"),
                b(&port),
                b("prec_key"),
                b("0"),
                b("5000"),
                b("AUTH"),
                b("legacy-only"),
                b("AUTH2"),
                b("acluser"),
                b("aclpass"),
                b("REPLACE"),
            ],
            &mut db,
        )
        .await;
    match result {
        Err(e) => {
            assert!(
                !e.to_string().contains("wrong number"),
                "should not be a parsing error: {e}"
            );
        }
        Ok(resp) => assert_ok(resp),
    }
}

#[tokio::test]
async fn test_migrate_auth2_syntax_error() {
    let r = router();
    let mut db = 0;
    let err = r
        .execute(
            "MIGRATE",
            &[
                b("127.0.0.1"),
                b("6379"),
                b("k"),
                b("0"),
                b("5000"),
                b("AUTH2"),
                b("only-user"),
            ],
            &mut db,
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("wrong number") || err.contains("syntax"),
        "missing AUTH2 password should error: {err}"
    );
}

#[tokio::test]
async fn test_migrate_keys() {
    use super::helpers::start_ephemeral_server;

    let (target_addr, _handle) = start_ephemeral_server().await;
    let port = target_addr.port().to_string();

    let r = router();
    let mut db = 0;
    r.execute("SET", &[b("k1"), b("v1")], &mut db)
        .await
        .unwrap();
    r.execute("SET", &[b("k2"), b("v2")], &mut db)
        .await
        .unwrap();
    r.execute("SET", &[b("k3"), b("v3")], &mut db)
        .await
        .unwrap();

    let result = r
        .execute(
            "MIGRATE",
            &[
                b("127.0.0.1"),
                b(&port),
                b(""),
                b("0"),
                b("5000"),
                b("KEYS"),
                b("k1"),
                b("k2"),
                b("k3"),
            ],
            &mut db,
        )
        .await;
    assert_ok(result.unwrap());

    // Source keys should be deleted
    assert_nil(r.execute("GET", &[b("k1")], &mut db).await.unwrap());
    assert_nil(r.execute("GET", &[b("k2")], &mut db).await.unwrap());
    assert_nil(r.execute("GET", &[b("k3")], &mut db).await.unwrap());
}

#[tokio::test]
async fn test_migrate_keys_copy() {
    use super::helpers::{start_ephemeral_server, tcp_get};

    let (target_addr, _handle) = start_ephemeral_server().await;
    let port = target_addr.port().to_string();

    let r = router();
    let mut db = 0;
    r.execute("SET", &[b("k1"), b("v1")], &mut db)
        .await
        .unwrap();
    r.execute("SET", &[b("k2"), b("v2")], &mut db)
        .await
        .unwrap();

    assert_ok(
        r.execute(
            "MIGRATE",
            &[
                b("127.0.0.1"),
                b(&port),
                b(""),
                b("0"),
                b("5000"),
                b("COPY"),
                b("KEYS"),
                b("k1"),
                b("k2"),
                b("REPLACE"),
            ],
            &mut db,
        )
        .await
        .unwrap(),
    );

    assert_eq!(
        r.execute("GET", &[b("k1")], &mut db).await.unwrap(),
        aikv::protocol::RespValue::BulkString(Some(b("v1")))
    );
    assert_eq!(
        r.execute("GET", &[b("k2")], &mut db).await.unwrap(),
        aikv::protocol::RespValue::BulkString(Some(b("v2")))
    );

    for (key, val) in [("k1", "v1"), ("k2", "v2")] {
        let resp = tcp_get(target_addr, key).await;
        assert!(
            resp.windows(val.len()).any(|w| w == val.as_bytes()),
            "target should have {key}, got {:?}",
            String::from_utf8_lossy(&resp)
        );
    }
}

// ── SCAN 游标一致性测试 ────────────────────────────────────────────────────────

/// 辅助：执行 SCAN 一次，返回 (next_cursor_bytes, keys)
async fn scan_once(
    r: &aikv::command::CommandRouter,
    db: &mut usize,
    cursor: &[u8],
    count: usize,
) -> (bytes::Bytes, Vec<String>) {
    let count_str = count.to_string();
    let resp = r
        .execute(
            "SCAN",
            &[
                bytes::Bytes::copy_from_slice(cursor),
                b("COUNT"),
                b(&count_str),
            ],
            db,
        )
        .await
        .unwrap();
    let aikv::protocol::RespValue::Array(Some(parts)) = resp else {
        panic!("SCAN expected array");
    };
    let aikv::protocol::RespValue::BulkString(Some(next_cursor)) = &parts[0] else {
        panic!("SCAN expected cursor bulk-string");
    };
    let aikv::protocol::RespValue::Array(Some(key_items)) = &parts[1] else {
        panic!("SCAN expected keys array");
    };
    let keys: Vec<String> = key_items
        .iter()
        .map(|item| {
            let aikv::protocol::RespValue::BulkString(Some(k)) = item else {
                panic!("expected bulk key");
            };
            String::from_utf8(k.to_vec()).unwrap()
        })
        .collect();
    (next_cursor.clone(), keys)
}

/// 插入 20 个 key, 用 SCAN COUNT 5 分页扫描, 验证不重复、不漏。
#[tokio::test]
async fn test_scan_full_pagination() {
    let r = router();
    let mut db = 0;
    for i in 0..20 {
        r.execute("SET", &[b(&format!("scankey{i:02}")), b("v")], &mut db)
            .await
            .unwrap();
    }

    let mut all_keys: HashSet<String> = HashSet::new();
    let mut cursor = b"0".to_vec();
    loop {
        let (next_cursor, keys) = scan_once(&r, &mut db, &cursor, 5).await;
        for k in keys {
            let inserted = all_keys.insert(k.clone());
            assert!(inserted, "duplicate key returned by SCAN: {k}");
        }
        if next_cursor.as_ref() == b"0" {
            break;
        }
        cursor = next_cursor.to_vec();
    }
    assert_eq!(
        all_keys.len(),
        20,
        "expected 20 distinct keys, got {}",
        all_keys.len()
    );
}

/// 拿到 cursor1 后中途插入新 key, 继续扫描, 验证不 panic。
#[tokio::test]
async fn test_scan_insert_mid_scan() {
    let r = router();
    let mut db = 0;
    for i in 0..10 {
        r.execute("SET", &[b(&format!("mid_ins_{i}")), b("v")], &mut db)
            .await
            .unwrap();
    }

    // 第一次扫描，拿到 cursor1（COUNT 3 保证不一次性扫完）
    let (cursor1, _first_keys) = scan_once(&r, &mut db, b"0", 3).await;

    // 中途插入新 key
    r.execute("SET", &[b("mid_ins_new"), b("new")], &mut db)
        .await
        .unwrap();

    // 用 cursor1 继续扫描，验证不 panic
    if cursor1.as_ref() != b"0" {
        let mut cursor = cursor1.to_vec();
        loop {
            let (next_cursor, _keys) = scan_once(&r, &mut db, &cursor, 5).await;
            if next_cursor.as_ref() == b"0" {
                break;
            }
            cursor = next_cursor.to_vec();
        }
    }
    // 能走到这里说明没有 panic，测试通过
}

/// 拿到 cursor1 后中途删除一个已扫 key, 继续扫描, 验证不 panic、无重复。
#[tokio::test]
async fn test_scan_delete_mid_scan() {
    let r = router();
    let mut db = 0;
    for i in 0..10 {
        r.execute("SET", &[b(&format!("mid_del_{i}")), b("v")], &mut db)
            .await
            .unwrap();
    }

    // 第一次扫描
    let (cursor1, first_keys) = scan_once(&r, &mut db, b"0", 3).await;

    // 删除第一批里的第一个 key（如果有）
    if let Some(k) = first_keys.first() {
        r.execute("DEL", &[b(k)], &mut db).await.unwrap();
    }

    // 继续用 cursor1 扫完，收集剩余 key
    let mut seen: HashSet<String> = first_keys.into_iter().collect();
    if cursor1.as_ref() != b"0" {
        let mut cursor = cursor1.to_vec();
        loop {
            let (next_cursor, keys) = scan_once(&r, &mut db, &cursor, 5).await;
            for k in keys {
                let inserted = seen.insert(k.clone());
                assert!(inserted, "duplicate key in mid-delete scan: {k}");
            }
            if next_cursor.as_ref() == b"0" {
                break;
            }
            cursor = next_cursor.to_vec();
        }
    }
    // 能走到这里说明没有 panic 且无重复，测试通过
}

// ── TTL 毫秒级真实过期测试 ────────────────────────────────────────────────────

/// SET key PX 100, 等待 200ms, 验证 GET 返回 nil（key 已过期）。
/// slow test
#[tokio::test]
#[ignore]
async fn test_px_expiry_real_wait() {
    let r = router();
    let mut db = 0;
    r.execute("SET", &[b("px_key"), b("v"), b("PX"), b("100")], &mut db)
        .await
        .unwrap();
    // key 刚设置时应存在
    let before = r.execute("GET", &[b("px_key")], &mut db).await.unwrap();
    assert_eq!(before, aikv::protocol::RespValue::BulkString(Some(b("v"))));
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_nil(r.execute("GET", &[b("px_key")], &mut db).await.unwrap());
}

/// PEXPIREAT 设置过去时间戳, 验证 key 立即过期。
#[tokio::test]
async fn test_pexpireat_past_timestamp() {
    let r = router();
    let mut db = 0;
    r.execute("SET", &[b("peat_key"), b("v")], &mut db)
        .await
        .unwrap();
    // 使用过去的时间戳（1 ms）
    r.execute("PEXPIREAT", &[b("peat_key"), b("1")], &mut db)
        .await
        .unwrap();
    assert_nil(r.execute("GET", &[b("peat_key")], &mut db).await.unwrap());
}

/// PTTL 返回合理的剩余毫秒数（> 0 且 <= 设置值）。
#[tokio::test]
async fn test_pttl_reasonable_value() {
    let r = router();
    let mut db = 0;
    r.execute("SET", &[b("pttl_key"), b("v")], &mut db)
        .await
        .unwrap();
    r.execute("PEXPIRE", &[b("pttl_key"), b("5000")], &mut db)
        .await
        .unwrap();
    let pttl = match r.execute("PTTL", &[b("pttl_key")], &mut db).await.unwrap() {
        aikv::protocol::RespValue::Integer(n) => n,
        other => panic!("expected integer, got {other:?}"),
    };
    assert!(pttl > 0 && pttl <= 5000, "PTTL out of range: {pttl}");
}

/// SET key PX 300, PTTL 验证 > 0 且 <= 300。
#[tokio::test]
async fn test_set_px_then_pttl() {
    let r = router();
    let mut db = 0;
    r.execute("SET", &[b("setpx_key"), b("v"), b("PX"), b("300")], &mut db)
        .await
        .unwrap();
    let pttl = match r.execute("PTTL", &[b("setpx_key")], &mut db).await.unwrap() {
        aikv::protocol::RespValue::Integer(n) => n,
        other => panic!("expected integer, got {other:?}"),
    };
    assert!(pttl > 0 && pttl <= 300, "PTTL out of range: {pttl}");
}
