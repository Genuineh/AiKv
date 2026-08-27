//! List 列表结构命令族测试
//! @component aikv-command

use aikv::protocol::RespValue;

use super::helpers::{assert_int, b, router};

/// 验证 LPUSH / RPUSH 列表推入与 LLEN 长度计算
#[tokio::test]
async fn test_lpush_rpush_llen() {
    let r = router();
    let mut db = 0;
    assert_int(
        r.execute("LPUSH", &[b("mylist"), b("a"), b("b")], &mut db)
            .await
            .unwrap(),
        2,
    );
    assert_int(
        r.execute("RPUSH", &[b("mylist"), b("c")], &mut db)
            .await
            .unwrap(),
        3,
    );
    assert_int(r.execute("LLEN", &[b("mylist")], &mut db).await.unwrap(), 3);
}

#[tokio::test]
async fn test_lpop_rpop() {
    let r = router();
    let mut db = 0;
    r.execute("RPUSH", &[b("l"), b("1"), b("2"), b("3")], &mut db)
        .await
        .unwrap();
    let left = r.execute("LPOP", &[b("l")], &mut db).await.unwrap();
    assert_eq!(left, RespValue::BulkString(Some(b("1"))));
    let right = r.execute("RPOP", &[b("l")], &mut db).await.unwrap();
    assert_eq!(right, RespValue::BulkString(Some(b("3"))));
    assert_int(r.execute("LLEN", &[b("l")], &mut db).await.unwrap(), 1);
}

#[tokio::test]
async fn test_lrange() {
    let r = router();
    let mut db = 0;
    r.execute("RPUSH", &[b("l"), b("a"), b("b"), b("c"), b("d")], &mut db)
        .await
        .unwrap();
    let resp = r
        .execute("LRANGE", &[b("l"), b("1"), b("2")], &mut db)
        .await
        .unwrap();
    let RespValue::Array(Some(items)) = resp else {
        panic!("expected array");
    };
    assert_eq!(items.len(), 2);
    assert_eq!(items[0], RespValue::BulkString(Some(b("b"))));
    assert_eq!(items[1], RespValue::BulkString(Some(b("c"))));
}

#[tokio::test]
async fn test_wrong_type_list() {
    let r = router();
    let mut db = 0;
    r.execute("SET", &[b("s"), b("v")], &mut db).await.unwrap();
    let err = r
        .execute("LPUSH", &[b("s"), b("x")], &mut db)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("WRONGTYPE"));
}

#[tokio::test]
async fn test_linsert_before_after() {
    let r = router();
    let mut db = 0;
    r.execute("RPUSH", &[b("l"), b("a"), b("b"), b("c")], &mut db)
        .await
        .unwrap();
    assert_int(
        r.execute("LINSERT", &[b("l"), b("BEFORE"), b("b"), b("x")], &mut db)
            .await
            .unwrap(),
        4,
    );
    let resp = r
        .execute("LRANGE", &[b("l"), b("0"), b("-1")], &mut db)
        .await
        .unwrap();
    let RespValue::Array(Some(items)) = resp else {
        panic!("expected array");
    };
    assert_eq!(items[0], RespValue::BulkString(Some(b("a"))));
    assert_eq!(items[1], RespValue::BulkString(Some(b("x"))));
    assert_eq!(items[2], RespValue::BulkString(Some(b("b"))));
    assert_eq!(items[3], RespValue::BulkString(Some(b("c"))));

    assert_int(
        r.execute("LINSERT", &[b("l"), b("AFTER"), b("b"), b("y")], &mut db)
            .await
            .unwrap(),
        5,
    );
    let resp = r
        .execute("LRANGE", &[b("l"), b("0"), b("-1")], &mut db)
        .await
        .unwrap();
    let RespValue::Array(Some(items)) = resp else {
        panic!("expected array");
    };
    assert_eq!(items[3], RespValue::BulkString(Some(b("y"))));
}

#[tokio::test]
async fn test_linsert_pivot_not_found() {
    let r = router();
    let mut db = 0;
    r.execute("RPUSH", &[b("l"), b("a"), b("b")], &mut db)
        .await
        .unwrap();
    assert_int(
        r.execute(
            "LINSERT",
            &[b("l"), b("BEFORE"), b("missing"), b("z")],
            &mut db,
        )
        .await
        .unwrap(),
        -1,
    );
}

#[tokio::test]
async fn test_linsert_before_after_case() {
    let r = router();
    let mut db = 0;
    r.execute("RPUSH", &[b("l"), b("a"), b("b")], &mut db)
        .await
        .unwrap();
    assert_int(
        r.execute("LINSERT", &[b("l"), b("before"), b("b"), b("x")], &mut db)
            .await
            .unwrap(),
        3,
    );
    assert_int(
        r.execute("LINSERT", &[b("l"), b("After"), b("b"), b("y")], &mut db)
            .await
            .unwrap(),
        4,
    );
}

#[tokio::test]
async fn test_lmove_diff_key() {
    let r = router();
    let mut db = 0;
    r.execute("RPUSH", &[b("src"), b("a"), b("b"), b("c")], &mut db)
        .await
        .unwrap();
    let moved = r
        .execute(
            "LMOVE",
            &[b("src"), b("dst"), b("LEFT"), b("RIGHT")],
            &mut db,
        )
        .await
        .unwrap();
    assert_eq!(moved, RespValue::BulkString(Some(b("a"))));

    let src = r
        .execute("LRANGE", &[b("src"), b("0"), b("-1")], &mut db)
        .await
        .unwrap();
    let RespValue::Array(Some(src_items)) = src else {
        panic!("expected array");
    };
    assert_eq!(src_items.len(), 2);
    assert_eq!(src_items[0], RespValue::BulkString(Some(b("b"))));

    let dst = r
        .execute("LRANGE", &[b("dst"), b("0"), b("-1")], &mut db)
        .await
        .unwrap();
    let RespValue::Array(Some(dst_items)) = dst else {
        panic!("expected array");
    };
    assert_eq!(dst_items.len(), 1);
    assert_eq!(dst_items[0], RespValue::BulkString(Some(b("a"))));
}

#[tokio::test]
async fn test_lmove_same_key() {
    let r = router();
    let mut db = 0;
    r.execute("RPUSH", &[b("l"), b("a"), b("b"), b("c")], &mut db)
        .await
        .unwrap();
    let moved = r
        .execute("LMOVE", &[b("l"), b("l"), b("LEFT"), b("RIGHT")], &mut db)
        .await
        .unwrap();
    assert_eq!(moved, RespValue::BulkString(Some(b("a"))));

    let resp = r
        .execute("LRANGE", &[b("l"), b("0"), b("-1")], &mut db)
        .await
        .unwrap();
    let RespValue::Array(Some(items)) = resp else {
        panic!("expected array");
    };
    assert_eq!(items.len(), 3);
    assert_eq!(items[0], RespValue::BulkString(Some(b("b"))));
    assert_eq!(items[1], RespValue::BulkString(Some(b("c"))));
    assert_eq!(items[2], RespValue::BulkString(Some(b("a"))));
}

#[tokio::test]
async fn test_lmove_dest_wrong_type() {
    let r = router();
    let mut db = 0;
    r.execute("SET", &[b("dst"), b("v")], &mut db)
        .await
        .unwrap();
    r.execute("RPUSH", &[b("src"), b("a")], &mut db)
        .await
        .unwrap();
    let err = r
        .execute(
            "LMOVE",
            &[b("src"), b("dst"), b("LEFT"), b("RIGHT")],
            &mut db,
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("WRONGTYPE"));
}

#[tokio::test]
async fn test_lpos_basic() {
    let r = router();
    let mut db = 0;
    r.execute("RPUSH", &[b("l"), b("a"), b("b"), b("c"), b("b")], &mut db)
        .await
        .unwrap();
    assert_int(
        r.execute("LPOS", &[b("l"), b("b")], &mut db).await.unwrap(),
        1,
    );
}

#[tokio::test]
async fn test_lpos_rank_zero() {
    let r = router();
    let mut db = 0;
    r.execute("RPUSH", &[b("l"), b("a"), b("b")], &mut db)
        .await
        .unwrap();
    let err = r
        .execute("LPOS", &[b("l"), b("b"), b("RANK"), b("0")], &mut db)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("RANK can't be zero"));
}

/// timeout=0 必须无限阻塞, 直到另一任务 LPUSH. 负数必须 `ERR timeout is negative`.
/// 草稿: `2026-08-14-fix-blpop-timeout-zero.md`
#[tokio::test]
async fn blpop_zero_blocks_until_lpush() {
    let r = std::sync::Arc::new(router());
    let r_wait = r.clone();
    let waiter = tokio::spawn(async move {
        let mut db = 0;
        r_wait
            .execute("BLPOP", &[b("blq"), b("0")], &mut db)
            .await
            .unwrap()
    });
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    assert!(
        !waiter.is_finished(),
        "BLPOP 0 on empty list must still be blocked"
    );
    let mut db = 0;
    r.execute("LPUSH", &[b("blq"), b("hello")], &mut db)
        .await
        .unwrap();
    let resp = tokio::time::timeout(std::time::Duration::from_secs(2), waiter)
        .await
        .expect("BLPOP 0 should be woken by LPUSH")
        .unwrap();
    match resp {
        RespValue::Array(Some(items)) => {
            assert_eq!(items.len(), 2);
            assert_eq!(items[0], RespValue::BulkString(Some(b("blq"))));
            assert_eq!(items[1], RespValue::BulkString(Some(b("hello"))));
        }
        other => panic!("expected array [blq, hello], got {other:?}"),
    }
}

#[tokio::test]
async fn blpop_negative_timeout_is_error() {
    let r = router();
    let mut db = 0;
    let err = r
        .execute("BLPOP", &[b("blq"), b("-1")], &mut db)
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("ERR timeout is negative"),
        "got {err}"
    );
}

#[tokio::test]
async fn blpop_nan_timeout_is_error() {
    let r = router();
    let mut db = 0;
    let err = r
        .execute("BLPOP", &[b("blq"), b("nan")], &mut db)
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("ERR timeout is negative"),
        "got {err}"
    );
}
