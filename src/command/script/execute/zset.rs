//! Lua `redis.call` ZSet 域 `exec_*` (ZADD/ZREM/ZSCORE/ZRANK/ZRANGE/ZCARD).

use std::collections::BTreeMap;

use bytes::Bytes;

use crate::command::router;
use crate::command::script::transaction::{parse_f64_arg, parse_i64_arg, ScriptTransaction};
use crate::error::Result;
use crate::protocol::RespValue;
use crate::storage::KvStorage;

use super::{key, wrong_count};

pub(super) async fn exec_zadd(
    storage: &dyn KvStorage,
    txn: &mut ScriptTransaction,
    args: &[Bytes],
) -> Result<RespValue> {
    if args.len() < 3 || !(args.len() - 1).is_multiple_of(2) {
        return Err(wrong_count("ZADD"));
    }
    let mut zset = if let Some(stored) = txn.get_value(storage, &args[0]).await? {
        stored.as_zset()?.clone()
    } else {
        BTreeMap::new()
    };
    let mut added = 0i64;
    for i in (1..args.len()).step_by(2) {
        let score = parse_f64_arg(&args[i], "score")?;
        if zset.insert(args[i + 1].to_vec(), score).is_none() {
            added += 1;
        }
    }
    txn.set_zset(key(&args[0]), zset);
    Ok(router::integer(added))
}

pub(super) async fn exec_zrem(
    storage: &dyn KvStorage,
    txn: &mut ScriptTransaction,
    args: &[Bytes],
) -> Result<RespValue> {
    if args.len() < 2 {
        return Err(wrong_count("ZREM"));
    }
    let Some(stored) = txn.get_value(storage, &args[0]).await? else {
        return Ok(router::integer(0));
    };
    let mut zset = stored.as_zset()?.clone();
    let mut count = 0i64;
    for m in &args[1..] {
        if zset.remove(&m.to_vec()).is_some() {
            count += 1;
        }
    }
    if zset.is_empty() {
        txn.delete(key(&args[0]));
    } else {
        txn.set_zset(key(&args[0]), zset);
    }
    Ok(router::integer(count))
}

pub(super) async fn exec_zscore(
    storage: &dyn KvStorage,
    txn: &ScriptTransaction,
    args: &[Bytes],
) -> Result<RespValue> {
    if args.len() != 2 {
        return Err(wrong_count("ZSCORE"));
    }
    if let Some(stored) = txn.get_value(storage, &args[0]).await? {
        let zset = stored.as_zset()?;
        Ok(match zset.get(args[1].as_ref()) {
            Some(s) => router::bulk(s.to_string().into_bytes()),
            None => RespValue::Null,
        })
    } else {
        Ok(RespValue::Null)
    }
}

pub(super) async fn exec_zrank(
    storage: &dyn KvStorage,
    txn: &ScriptTransaction,
    args: &[Bytes],
) -> Result<RespValue> {
    if args.len() != 2 {
        return Err(wrong_count("ZRANK"));
    }
    let Some(stored) = txn.get_value(storage, &args[0]).await? else {
        return Ok(RespValue::Null);
    };
    let zset = stored.as_zset()?;
    if !zset.contains_key(args[1].as_ref()) {
        return Ok(RespValue::Null);
    }
    let mut sorted: Vec<_> = zset.iter().collect();
    sorted.sort_by(|a, b| {
        a.1.partial_cmp(b.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(b.0))
    });
    for (rank, (m, _)) in sorted.iter().enumerate() {
        if *m == args[1].as_ref() {
            return Ok(router::integer(rank as i64));
        }
    }
    Ok(RespValue::Null)
}

pub(super) async fn exec_zrange(
    storage: &dyn KvStorage,
    txn: &ScriptTransaction,
    args: &[Bytes],
) -> Result<RespValue> {
    if args.len() < 3 {
        return Err(wrong_count("ZRANGE"));
    }
    let start = parse_i64_arg(&args[1], "start")?;
    let stop = parse_i64_arg(&args[2], "stop")?;
    let with_scores = args.len() > 3 && args[3].eq_ignore_ascii_case(b"WITHSCORES");
    let Some(stored) = txn.get_value(storage, &args[0]).await? else {
        return Ok(RespValue::Array(Some(Vec::new())));
    };
    let zset = stored.as_zset()?;
    let len = zset.len() as i64;
    let mut sorted: Vec<_> = zset.iter().collect();
    sorted.sort_by(|a, b| {
        a.1.partial_cmp(b.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(b.0))
    });
    let start_idx = (if start < 0 { len + start } else { start }).clamp(0, len) as usize;
    let stop_idx = (if stop < 0 { len + stop + 1 } else { stop + 1 }).clamp(0, len) as usize;
    if start_idx >= stop_idx {
        return Ok(RespValue::Array(Some(Vec::new())));
    }
    let mut out = Vec::new();
    for (member, score) in sorted.iter().skip(start_idx).take(stop_idx - start_idx) {
        out.push(router::bulk((*member).clone()));
        if with_scores {
            out.push(router::bulk(score.to_string().into_bytes()));
        }
    }
    Ok(RespValue::Array(Some(out)))
}

pub(super) async fn exec_zcard(
    storage: &dyn KvStorage,
    txn: &ScriptTransaction,
    args: &[Bytes],
) -> Result<RespValue> {
    if args.len() != 1 {
        return Err(wrong_count("ZCARD"));
    }
    let n = if let Some(stored) = txn.get_value(storage, &args[0]).await? {
        stored.as_zset()?.len()
    } else {
        0
    };
    Ok(router::integer(n as i64))
}
