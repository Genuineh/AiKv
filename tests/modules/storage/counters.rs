use std::sync::Arc;
use tempfile::tempdir;

use aikv::storage::adapter::{KvStorageAdapter, StorageAdapter};
use aikv::storage::types::{now_ms, CollectionKind, KvStorage, StoredValue, ValueType, DB_COUNT};
use aikv::storage::AiDbEngine;

fn make_temp_engine() -> (tempfile::TempDir, Arc<AiDbEngine>) {
    let dir = tempdir().unwrap();
    let engine = AiDbEngine::open_for_testing(dir.path()).unwrap();
    (dir, engine)
}

#[tokio::test]
async fn test_cold_startup_rebuild_counters() {
    let dir = tempdir().unwrap();
    let path = dir.path().to_path_buf();

    // 1. 初始化写入数据
    {
        let engine = AiDbEngine::open_for_testing(&path).unwrap();
        let storage = KvStorageAdapter::open(engine.clone(), None).await.unwrap();

        // DB 0 写入 3 个未过期 key, 1 个已过期 key
        storage.set(0, b"k1", b"v1").await.unwrap();
        storage.set(0, b"k2", b"v2").await.unwrap();
        storage.set(0, b"k3", b"v3").await.unwrap();
        storage
            .set_with_ttl(0, b"k_expired", b"v_exp", now_ms().saturating_sub(1000))
            .await
            .unwrap();

        // DB 1 写入 2 个 key
        storage.set(1, b"db1_k1", b"v1").await.unwrap();
        storage.set(1, b"db1_k2", b"v2").await.unwrap();

        // DB 5 写入 1 个 key
        storage.set(5, b"db5_k1", b"v1").await.unwrap();

        // 验证写入期间的计数 (含未清理的过期 key)
        assert_eq!(storage.len(0).await.unwrap(), 4);
        assert_eq!(storage.len(1).await.unwrap(), 2);
        assert_eq!(storage.len(5).await.unwrap(), 1);

        engine.flush().await.unwrap();
    }

    // 2. 模拟进程重启冷启动, 用 open 重建计数器
    {
        let engine = AiDbEngine::open_for_testing(&path).unwrap();
        let storage = KvStorageAdapter::open(engine, None).await.unwrap();

        // 验证冷启动后各 DB 计数立即准确 (禁止生产冷数据从 0)
        assert_eq!(storage.len(0).await.unwrap(), 3);
        assert_eq!(storage.len(1).await.unwrap(), 2);
        assert_eq!(storage.len(5).await.unwrap(), 1);
        assert_eq!(storage.len(2).await.unwrap(), 0);

        let counts = storage.db_key_counts().await.unwrap();
        assert_eq!(counts.len(), DB_COUNT);
        assert_eq!(counts[0], 3);
        assert_eq!(counts[1], 2);
        assert_eq!(counts[5], 1);
        assert_eq!(counts[2], 0);
    }
}

#[tokio::test]
async fn test_random_mixed_reconcile_counters() {
    let (_dir, engine) = make_temp_engine();
    let storage = KvStorageAdapter::open(engine, None).await.unwrap();

    let db = 0;

    // 1. 批量插入新 key
    for i in 0..50 {
        let k = format!("key_{i}").into_bytes();
        let v = format!("val_{i}").into_bytes();
        storage.set(db, &k, &v).await.unwrap();
    }
    assert_eq!(storage.len(db).await.unwrap(), 50);

    // 2. 覆盖写 (不应增加计数)
    for i in 0..20 {
        let k = format!("key_{i}").into_bytes();
        let v = format!("updated_val_{i}").into_bytes();
        storage.set(db, &k, &v).await.unwrap();
    }
    assert_eq!(storage.len(db).await.unwrap(), 50);

    // 3. 删除一部分 key
    for i in 0..15 {
        let k = format!("key_{i}").into_bytes();
        assert!(storage.delete(db, &k).await.unwrap());
    }
    assert_eq!(storage.len(db).await.unwrap(), 35);

    // 重复删除不存在的 key (不应减少计数)
    for i in 0..15 {
        let k = format!("key_{i}").into_bytes();
        assert!(!storage.delete(db, &k).await.unwrap());
    }
    assert_eq!(storage.len(db).await.unwrap(), 35);

    // 4. 重命名 key
    // rename 到新 key (总数不变)
    storage
        .rename_key(db, b"key_20", b"key_20_renamed")
        .await
        .unwrap();
    assert_eq!(storage.len(db).await.unwrap(), 35);

    // rename 覆盖已有 key (总数减 1)
    storage.rename_key(db, b"key_21", b"key_22").await.unwrap();
    assert_eq!(storage.len(db).await.unwrap(), 34);

    // 5. 复制 key
    // copy 到不存在的 key (总数加 1)
    assert!(storage
        .copy_key(db, db, b"key_22", b"key_copied", false)
        .await
        .unwrap());
    assert_eq!(storage.len(db).await.unwrap(), 35);

    // copy 到已有 key 且 replace=true (总数不变)
    assert!(storage
        .copy_key(db, db, b"key_22", b"key_copied", true)
        .await
        .unwrap());
    assert_eq!(storage.len(db).await.unwrap(), 35);

    // 6. 惰性过期触发删除 (计数减少 1)
    storage
        .set_with_ttl(db, b"exp_key", b"val", now_ms().saturating_sub(100))
        .await
        .unwrap();
    assert_eq!(storage.len(db).await.unwrap(), 36);

    // 读取触发惰性过期
    assert!(storage.get(db, b"exp_key").await.unwrap().is_none());
    assert_eq!(storage.len(db).await.unwrap(), 35);

    // 7. Reconcile 严格断言: 内存计数器与全库扫描所得存活 key 数量 100% 一致
    let scanned_keys = storage.keys(db, b"*").await.unwrap();
    assert_eq!(storage.len(db).await.unwrap(), scanned_keys.len());
    assert_eq!(
        storage.db_key_counts().await.unwrap()[db],
        scanned_keys.len()
    );
}

#[tokio::test]
async fn test_subkey_counters_isolation() {
    let (_dir, engine) = make_temp_engine();
    let storage = KvStorageAdapter::open(engine.clone(), None).await.unwrap();

    let db = 1;

    // 写入一个逻辑 Hash header
    let header = StoredValue {
        value: ValueType::CollectionHeader {
            kind: CollectionKind::Hash,
            count: 2,
        },
        expires_at: None,
    };
    storage.set_typed(db, b"my_hash", header).await.unwrap();
    assert_eq!(storage.len(db).await.unwrap(), 1);

    // 写入底层 raw subkeys (如 Hash fields)
    let subkey1 = b"1:my_hash\x01Hfield1".to_vec();
    let subkey2 = b"1:my_hash\x01Hfield2".to_vec();
    storage
        .raw_subkey_set(db, subkey1.clone(), b"val1".to_vec())
        .await
        .unwrap();
    storage
        .raw_subkey_set(db, subkey2.clone(), b"val2".to_vec())
        .await
        .unwrap();

    // 验证 subkey 不增加逻辑 user key 计数
    assert_eq!(storage.len(db).await.unwrap(), 1);

    // 删除 subkey 也不影响逻辑 user key 计数
    storage.raw_subkey_delete(db, subkey1).await.unwrap();
    assert_eq!(storage.len(db).await.unwrap(), 1);

    // 删除逻辑 Hash header
    storage.delete(db, b"my_hash").await.unwrap();
    assert_eq!(storage.len(db).await.unwrap(), 0);
}

#[tokio::test]
async fn test_swap_db_and_clear_all_counters() {
    let (_dir, engine) = make_temp_engine();
    let storage = KvStorageAdapter::open(engine, None).await.unwrap();

    // DB 1 写入 3 个 key
    storage.set(1, b"a1", b"v").await.unwrap();
    storage.set(1, b"a2", b"v").await.unwrap();
    storage.set(1, b"a3", b"v").await.unwrap();

    // DB 2 写入 1 个 key
    storage.set(2, b"b1", b"v").await.unwrap();

    assert_eq!(storage.len(1).await.unwrap(), 3);
    assert_eq!(storage.len(2).await.unwrap(), 1);

    // SWAPDB 1 2
    storage.swap_db(1, 2).await.unwrap();
    assert_eq!(storage.len(1).await.unwrap(), 1);
    assert_eq!(storage.len(2).await.unwrap(), 3);

    // FLUSHALL
    storage.clear_all().await.unwrap();
    for db in 0..DB_COUNT {
        assert_eq!(storage.len(db).await.unwrap(), 0);
    }
    let counts = storage.db_key_counts().await.unwrap();
    assert!(counts.iter().all(|&c| c == 0));
}

/// Compaction 丢弃最新过期 Put 时必须通过 RemovalListener decr, 与惰性删除语义对齐.
#[tokio::test]
async fn test_compaction_ttl_filter_decr_counters() {
    use std::sync::Arc;

    use aikv::storage::ttl_filter::{DbKeyCounterRemovalListener, TtlExpireFilter};
    use aikv::storage::types::DbKeyCounters;

    let dir = tempdir().unwrap();
    let mut opts = aikv::storage::aidb_options::testing_db_options();
    opts.memtable_size = 256;
    opts.level0_compaction_trigger = 1;
    opts.background_compaction = false;

    let engine = AiDbEngine::open_with_options(dir.path(), opts).unwrap();
    let counters = Arc::new(DbKeyCounters::new());
    let expire_gate = Arc::new(aikv::storage::types::ExpireDecrGate::new());
    engine
        .db
        .set_compaction_filter(Some(Arc::new(TtlExpireFilter)));
    engine
        .db
        .set_compaction_removal_listener(Some(Arc::new(DbKeyCounterRemovalListener::new(
            counters.clone(),
            expire_gate.clone(),
            engine.db.clone(),
        ))));

    let storage = KvStorageAdapter::open_with_counters(engine.clone(), None, counters, expire_gate)
        .await
        .unwrap();

    storage.set(0, b"alive", b"keep").await.unwrap();
    for i in 0..12 {
        let k = format!("exp_{i}");
        storage
            .set_with_ttl(0, k.as_bytes(), b"v", now_ms().saturating_sub(1000))
            .await
            .unwrap();
        if i % 2 == 1 {
            engine.flush().await.unwrap();
        }
    }
    engine.flush().await.unwrap();
    assert!(storage.len(0).await.unwrap() >= 13);

    engine.db.drain_compactions().unwrap();

    // 不先 KEYS (会惰性删); 仅 compaction 须把过期 key 扣掉, 留下 alive.
    assert_eq!(storage.len(0).await.unwrap(), 1);
    assert_eq!(storage.keys(0, b"*").await.unwrap().len(), 1);
}

/// 先惰性删除再 compaction: 不得双重 decr.
#[tokio::test]
async fn test_lazy_expire_then_compaction_no_double_decr() {
    use std::sync::Arc;

    use aikv::storage::ttl_filter::{DbKeyCounterRemovalListener, TtlExpireFilter};
    use aikv::storage::types::DbKeyCounters;

    let dir = tempdir().unwrap();
    let mut opts = aikv::storage::aidb_options::testing_db_options();
    opts.memtable_size = 256;
    opts.level0_compaction_trigger = 1;
    opts.background_compaction = false;

    let engine = AiDbEngine::open_with_options(dir.path(), opts).unwrap();
    let counters = Arc::new(DbKeyCounters::new());
    let expire_gate = Arc::new(aikv::storage::types::ExpireDecrGate::new());
    engine
        .db
        .set_compaction_filter(Some(Arc::new(TtlExpireFilter)));
    engine
        .db
        .set_compaction_removal_listener(Some(Arc::new(DbKeyCounterRemovalListener::new(
            counters.clone(),
            expire_gate.clone(),
            engine.db.clone(),
        ))));

    let storage = KvStorageAdapter::open_with_counters(engine.clone(), None, counters, expire_gate)
        .await
        .unwrap();

    storage.set(0, b"alive", b"keep").await.unwrap();
    storage
        .set_with_ttl(0, b"lazy_then_gc", b"v", now_ms().saturating_sub(1000))
        .await
        .unwrap();
    assert_eq!(storage.len(0).await.unwrap(), 2);
    engine.flush().await.unwrap();

    // 惰性读触发删除 → 计数只剩 alive
    assert!(storage.get(0, b"lazy_then_gc").await.unwrap().is_none());
    assert_eq!(storage.len(0).await.unwrap(), 1);

    // 再 compact: 不得把 alive 也扣掉 (双重 decr 会被 saturating 掩盖若库已空)
    engine.db.drain_compactions().unwrap();
    assert_eq!(storage.len(0).await.unwrap(), 1);
    assert_eq!(storage.keys(0, b"*").await.unwrap().len(), 1);
}

/// 同 key 过期扣减后重生再过期, 门闩不得导致漏 decr.
#[tokio::test]
async fn test_reincarnate_after_expire_decr() {
    let (_dir, engine) = make_temp_engine();
    let storage = KvStorageAdapter::open(engine, None).await.unwrap();

    storage
        .set_with_ttl(0, b"reinc", b"v1", now_ms().saturating_sub(1000))
        .await
        .unwrap();
    assert_eq!(storage.len(0).await.unwrap(), 1);
    assert!(storage.get(0, b"reinc").await.unwrap().is_none());
    assert_eq!(storage.len(0).await.unwrap(), 0);

    storage.set(0, b"reinc", b"v2").await.unwrap();
    assert_eq!(storage.len(0).await.unwrap(), 1);
    storage
        .set_with_ttl(0, b"reinc", b"v3", now_ms().saturating_sub(1000))
        .await
        .unwrap();
    assert_eq!(storage.len(0).await.unwrap(), 1);
    assert!(storage.get(0, b"reinc").await.unwrap().is_none());
    assert_eq!(storage.len(0).await.unwrap(), 0);
}

/// MSET / write_batch 必须维护 DbKeyCounters, 否则 DBSIZE 会系统性漂移.
#[tokio::test]
async fn test_mset_write_batch_maintains_counters() {
    let (_dir, engine) = make_temp_engine();
    let storage = KvStorageAdapter::open(engine, None).await.unwrap();
    let db = 0;

    // 全新 key 批量写入
    let pairs: Vec<(Vec<u8>, Vec<u8>)> = (0..10)
        .map(|i| {
            (
                format!("mset_{i}").into_bytes(),
                format!("v{i}").into_bytes(),
            )
        })
        .collect();
    storage.mset(db, &pairs).await.unwrap();
    assert_eq!(storage.len(db).await.unwrap(), 10);

    // 覆盖写一半: 计数不变
    let overwrite: Vec<(Vec<u8>, Vec<u8>)> = (0..5)
        .map(|i| {
            (
                format!("mset_{i}").into_bytes(),
                format!("ov{i}").into_bytes(),
            )
        })
        .collect();
    storage.mset(db, &overwrite).await.unwrap();
    assert_eq!(storage.len(db).await.unwrap(), 10);

    // 混入 3 个新 key
    let mixed: Vec<(Vec<u8>, Vec<u8>)> = vec![
        (b"mset_0".to_vec(), b"again".to_vec()),
        (b"mset_new_a".to_vec(), b"a".to_vec()),
        (b"mset_new_b".to_vec(), b"b".to_vec()),
        (b"mset_new_c".to_vec(), b"c".to_vec()),
    ];
    storage.mset(db, &mixed).await.unwrap();
    assert_eq!(storage.len(db).await.unwrap(), 13);

    let scanned = storage.keys(db, b"*").await.unwrap();
    assert_eq!(storage.len(db).await.unwrap(), scanned.len());
}

/// write_batch 混合 Delete: 不存在的 key 不得截前缀认领真实删除的门闩.
/// 随后模拟 compaction listener 点查 `exists`, 截前缀回退会再扣一次.
#[tokio::test]
async fn test_write_batch_delete_missing_then_existing() {
    use std::sync::Arc;

    use aidb::engine::compaction::CompactionRemovalListener;
    use aikv::storage::ttl_filter::DbKeyCounterRemovalListener;
    use aikv::storage::types::{DbKeyCounters, WriteOp};

    let (_dir, engine) = make_temp_engine();
    let counters = Arc::new(DbKeyCounters::new());
    let expire_gate = Arc::new(aikv::storage::types::ExpireDecrGate::new());
    let storage = KvStorageAdapter::open_with_counters(
        engine.clone(),
        None,
        counters.clone(),
        expire_gate.clone(),
    )
    .await
    .unwrap();
    storage.set(0, b"exists", b"v").await.unwrap();
    storage.set(0, b"alive", b"keep").await.unwrap();
    assert_eq!(storage.len(0).await.unwrap(), 2);

    storage
        .write_batch(
            0,
            vec![
                (b"missing".to_vec(), WriteOp::Delete),
                (b"exists".to_vec(), WriteOp::Delete),
            ],
        )
        .await
        .unwrap();
    assert_eq!(storage.len(0).await.unwrap(), 1);

    let listener = DbKeyCounterRemovalListener::new(counters, expire_gate, engine.db.clone());
    listener.on_latest_put_removed(&AiDbEngine::encode_key(0, b"exists"));
    assert_eq!(storage.len(0).await.unwrap(), 1);
    assert_eq!(storage.keys(0, b"*").await.unwrap().len(), 1);
}
