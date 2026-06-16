use aikv::protocol::RespValue;

use super::helpers::{assert_int, assert_nil, assert_ok, b, router};

#[tokio::test]
async fn test_set_with_ex_nx_xx() {
    let r = router();
    let mut db = 0;
    assert_ok(
        r.execute("SET", &[b("k"), b("v"), b("EX"), b("60")], &mut db)
            .await
            .unwrap(),
    );
    assert_nil(
        r.execute("SET", &[b("k"), b("v2"), b("NX")], &mut db)
            .await
            .unwrap(),
    );
    assert_nil(
        r.execute("SET", &[b("k2"), b("v"), b("XX")], &mut db)
            .await
            .unwrap(),
    );
}

#[tokio::test]
async fn test_set_nx_xx_conflict() {
    let r = router();
    let mut db = 0;
    let err = r
        .execute("SET", &[b("k"), b("v"), b("NX"), b("XX")], &mut db)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("syntax error"));
}

#[tokio::test]
async fn test_get_missing_key() {
    let r = router();
    let mut db = 0;
    assert_nil(r.execute("GET", &[b("missing")], &mut db).await.unwrap());
}

#[tokio::test]
async fn test_mget_mset() {
    let r = router();
    let mut db = 0;
    assert_ok(
        r.execute("MSET", &[b("a"), b("1"), b("b"), b("2")], &mut db)
            .await
            .unwrap(),
    );
    let resp = r
        .execute("MGET", &[b("a"), b("b"), b("c")], &mut db)
        .await
        .unwrap();
    let RespValue::Array(Some(items)) = resp else {
        panic!("expected array");
    };
    assert_eq!(items.len(), 3);
}

#[tokio::test]
async fn test_mget_mset_param_validation() {
    let r = router();
    let mut db = 0;
    assert!(r.execute("MSET", &[b("a")], &mut db).await.is_err());
    r.execute("HSET", &[b("h"), b("f"), b("v")], &mut db)
        .await
        .unwrap();
    let resp = r.execute("MGET", &[b("h")], &mut db).await.unwrap();
    let RespValue::Array(Some(items)) = resp else {
        panic!("expected array");
    };
    assert_eq!(items[0], RespValue::BulkString(None));
}

#[tokio::test]
async fn test_del_multiple_keys() {
    let r = router();
    let mut db = 0;
    r.execute("SET", &[b("a"), b("1")], &mut db).await.unwrap();
    r.execute("SET", &[b("b"), b("2")], &mut db).await.unwrap();
    assert_int(
        r.execute("DEL", &[b("a"), b("b"), b("c")], &mut db)
            .await
            .unwrap(),
        2,
    );
}

#[tokio::test]
async fn test_append_strlen() {
    let r = router();
    let mut db = 0;
    assert_int(
        r.execute("APPEND", &[b("k"), b("hi")], &mut db)
            .await
            .unwrap(),
        2,
    );
    assert_int(r.execute("STRLEN", &[b("k")], &mut db).await.unwrap(), 2);
}

#[tokio::test]
async fn test_append_wrong_type() {
    let r = router();
    let mut db = 0;
    r.execute("HSET", &[b("h"), b("f"), b("v")], &mut db)
        .await
        .unwrap();
    assert!(r
        .execute("APPEND", &[b("h"), b("x")], &mut db)
        .await
        .unwrap_err()
        .to_string()
        .contains("WRONGTYPE"));
}

#[tokio::test]
async fn test_strlen_wrong_type() {
    let r = router();
    let mut db = 0;
    r.execute("HSET", &[b("h"), b("f"), b("v")], &mut db)
        .await
        .unwrap();
    assert!(r
        .execute("STRLEN", &[b("h")], &mut db)
        .await
        .unwrap_err()
        .to_string()
        .contains("WRONGTYPE"));
}

#[tokio::test]
async fn test_incr_decr_incrby() {
    let r = router();
    let mut db = 0;
    assert_int(r.execute("INCR", &[b("n")], &mut db).await.unwrap(), 1);
    assert_int(r.execute("INCR", &[b("n")], &mut db).await.unwrap(), 2);
    assert_int(r.execute("DECR", &[b("n")], &mut db).await.unwrap(), 1);
    assert_int(
        r.execute("INCRBY", &[b("n"), b("10")], &mut db)
            .await
            .unwrap(),
        11,
    );
    assert_int(
        r.execute("DECRBY", &[b("n"), b("5")], &mut db)
            .await
            .unwrap(),
        6,
    );
}

#[tokio::test]
async fn test_incrbyfloat() {
    let r = router();
    let mut db = 0;
    let resp = r
        .execute("INCRBYFLOAT", &[b("f"), b("2.5")], &mut db)
        .await
        .unwrap();
    assert_eq!(resp, RespValue::BulkString(Some(b("2.5"))));
    let resp = r
        .execute("INCRBYFLOAT", &[b("f"), b("0.5")], &mut db)
        .await
        .unwrap();
    assert_eq!(resp, RespValue::BulkString(Some(b("3"))));
}

#[tokio::test]
async fn test_exists_expired_key() {
    let r = router();
    let mut db = 0;
    r.execute("SET", &[b("k"), b("v")], &mut db).await.unwrap();
    r.execute("PEXPIRE", &[b("k"), b("1")], &mut db)
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    assert_int(r.execute("EXISTS", &[b("k")], &mut db).await.unwrap(), 0);
}

/// Mirrors `TL.Redis` `BaseRelationTests.创建bits` — bytes [1, 128, 3].
#[tokio::test]
async fn test_setbit_base_relation_bits_pattern() {
    let r = router();
    let mut db = 0;
    // TL.Redis: byte 1 → bit j=7; 128 → j=0; 3 → j=6,7
    let bits: &[(u64, u8)] = &[(7, 1), (8, 1), (22, 1), (23, 1)];
    for &(offset, value) in bits {
        let off = offset.to_string();
        r.execute(
            "SETBIT",
            &[b("k"), b(&off), b(if value == 1 { "1" } else { "0" })],
            &mut db,
        )
        .await
        .unwrap();
    }
    let got = r.execute("GET", &[b("k")], &mut db).await.unwrap();
    match got {
        RespValue::BulkString(Some(bytes)) => {
            assert_eq!(bytes.len(), 3);
            assert_eq!(bytes[0], 1);
            assert_eq!(bytes[1], 128);
            assert_eq!(bytes[2], 3);
        }
        other => panic!("expected bulk string, got {:?}", other),
    }
    assert_int(
        r.execute("GETBIT", &[b("k"), b("7")], &mut db)
            .await
            .unwrap(),
        1,
    );
    assert_int(
        r.execute("GETBIT", &[b("k"), b("99")], &mut db)
            .await
            .unwrap(),
        0,
    );
}

#[tokio::test]
async fn test_setbit_returns_previous_and_clears() {
    let r = router();
    let mut db = 0;
    assert_int(
        r.execute("SETBIT", &[b("k"), b("0"), b("1")], &mut db)
            .await
            .unwrap(),
        0,
    );
    assert_int(
        r.execute("SETBIT", &[b("k"), b("0"), b("0")], &mut db)
            .await
            .unwrap(),
        1,
    );
    assert_int(
        r.execute("GETBIT", &[b("k"), b("0")], &mut db)
            .await
            .unwrap(),
        0,
    );
}
