//! Sorted Set 命令

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use tokio::sync::oneshot;
use tracing::instrument;

use crate::command::blocking::{self, BlockedClientGuard, BlockingRegistry};
use crate::command::router::{self, KeyLock};
use crate::command::scan_util;
use crate::error::{Error, Result};
use crate::protocol::RespValue;
use crate::server::ServerMetrics;
use crate::storage::memory::glob_match;
use crate::storage::{KvStorage, StoredValue, ValueType};

pub struct ZSetCommands {
    storage: Arc<dyn KvStorage>,
    key_lock: Arc<KeyLock>,
    metrics: Option<Arc<ServerMetrics>>,
}

impl ZSetCommands {
    pub fn new(storage: Arc<dyn KvStorage>, key_lock: Arc<KeyLock>) -> Self {
        Self {
            storage,
            key_lock,
            metrics: None,
        }
    }

    pub fn with_metrics(
        storage: Arc<dyn KvStorage>,
        key_lock: Arc<KeyLock>,
        metrics: Arc<ServerMetrics>,
    ) -> Self {
        Self {
            storage,
            key_lock,
            metrics: Some(metrics),
        }
    }

    #[instrument(name = "cmd_zset", skip(self, args), fields(cmd.name = "ZADD"))]
    pub async fn zadd(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_min_args("ZADD", args, 3)?;
        if !(args.len() - 1).is_multiple_of(2) {
            return Err(router::wrong_args("ZADD", ""));
        }
        let key = &args[0];
        let _lock = self.key_lock.lock(key).await;
        let mut stored = self.load_or_create_zset(db, key).await?;
        let zset = stored.as_zset_mut()?;
        let mut count = 0i64;
        for chunk in args[1..].chunks(2) {
            let score = parse_score(&chunk[0])?;
            let member = chunk[1].to_vec();
            if !zset.contains_key(&member) {
                count += 1;
            }
            zset.insert(member, score);
        }
        self.storage.set_typed(db, key, stored).await?;
        BlockingRegistry::global().notify(key, RespValue::SimpleString("OK".into()));
        Ok(router::integer(count))
    }

    pub async fn zrem(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_min_args("ZREM", args, 2)?;
        let key = &args[0];
        let _lock = self.key_lock.lock(key).await;
        let Some(mut stored) = self.storage.get_typed(db, key).await? else {
            return Ok(router::integer(0));
        };
        let zset = stored.as_zset_mut()?;
        let mut count = 0i64;
        for member in &args[1..] {
            if zset.remove(member.as_ref()).is_some() {
                count += 1;
            }
        }
        if zset.is_empty() {
            self.storage.delete(db, key).await?;
        } else {
            self.storage.set_typed(db, key, stored).await?;
        }
        Ok(router::integer(count))
    }

    pub async fn zscore(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_args("ZSCORE", args, 2)?;
        let zset = self.load_zset(db, &args[0]).await?;
        let Some(zset) = zset else {
            return Ok(router::nil_bulk());
        };
        match zset.get(args[1].as_ref()) {
            None => Ok(router::nil_bulk()),
            Some(score) => Ok(router::bulk(format_score(*score).into_bytes())),
        }
    }

    pub async fn zrank(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_args("ZRANK", args, 2)?;
        self.rank(db, &args[0], &args[1], false).await
    }

    pub async fn zrevrank(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_args("ZREVRANK", args, 2)?;
        self.rank(db, &args[0], &args[1], true).await
    }

    async fn rank(
        &self,
        db: usize,
        key: &Bytes,
        member: &Bytes,
        reverse: bool,
    ) -> Result<RespValue> {
        let zset = self.load_zset(db, key).await?;
        let Some(zset) = zset else {
            return Ok(router::nil_bulk());
        };
        if !zset.contains_key(member.as_ref()) {
            return Ok(router::nil_bulk());
        }
        let sorted = sorted_by_score(&zset, reverse);
        let pos = sorted
            .iter()
            .position(|(m, _)| m.as_slice() == member.as_ref())
            .unwrap();
        Ok(router::integer(pos as i64))
    }

    pub async fn zrange(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_min_args("ZRANGE", args, 3)?;
        self.range_by_index(db, &args[0], args, false).await
    }

    pub async fn zrevrange(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_min_args("ZREVRANGE", args, 3)?;
        self.range_by_index(db, &args[0], args, true).await
    }

    async fn range_by_index(
        &self,
        db: usize,
        key: &Bytes,
        args: &[Bytes],
        reverse: bool,
    ) -> Result<RespValue> {
        let start = parse_i64(&args[1])?;
        let stop = parse_i64(&args[2])?;
        let withscores = args.iter().any(|a| eq_ignore_case(a, b"WITHSCORES"));
        let zset = self.load_zset(db, key).await?;
        let Some(zset) = zset else {
            return Ok(RespValue::Array(Some(vec![])));
        };
        let sorted = sorted_by_score(&zset, reverse);
        let len = sorted.len();
        let (start_idx, stop_idx) = normalize_range(len, start, stop);
        if start_idx > stop_idx {
            return Ok(RespValue::Array(Some(vec![])));
        }
        let slice = &sorted[start_idx..=stop_idx];
        Ok(members_to_resp(slice, withscores))
    }

    pub async fn zrangebyscore(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_min_args("ZRANGEBYSCORE", args, 3)?;
        self.range_by_score(db, &args[0], args, false).await
    }

    pub async fn zrevrangebyscore(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_min_args("ZREVRANGEBYSCORE", args, 3)?;
        self.range_by_score(db, &args[0], args, true).await
    }

    async fn range_by_score(
        &self,
        db: usize,
        key: &Bytes,
        args: &[Bytes],
        reverse: bool,
    ) -> Result<RespValue> {
        let (min, max) = if reverse {
            (
                parse_score_bound(bytes_to_str(&args[2])?, true)?,
                parse_score_bound(bytes_to_str(&args[1])?, false)?,
            )
        } else {
            (
                parse_score_bound(bytes_to_str(&args[1])?, true)?,
                parse_score_bound(bytes_to_str(&args[2])?, false)?,
            )
        };
        let mut withscores = false;
        let mut limit: Option<(usize, Option<usize>)> = None;
        let mut i = 3;
        while i < args.len() {
            if eq_ignore_case(&args[i], b"WITHSCORES") {
                withscores = true;
                i += 1;
            } else if eq_ignore_case(&args[i], b"LIMIT") {
                if i + 2 >= args.len() {
                    return Err(router::wrong_args("ZRANGEBYSCORE", ""));
                }
                limit = Some(parse_limit_args(
                    "ZRANGEBYSCORE",
                    &args[i + 1],
                    &args[i + 2],
                )?);
                i += 3;
            } else {
                return Err(router::wrong_args("ZRANGEBYSCORE", ""));
            }
        }
        let zset = self.load_zset(db, key).await?;
        let Some(zset) = zset else {
            return Ok(RespValue::Array(Some(vec![])));
        };
        let mut sorted: Vec<(Vec<u8>, f64)> = sorted_by_score(&zset, reverse)
            .into_iter()
            .filter(|(_, score)| score_in_range(*score, &min, &max))
            .collect();
        if let Some((offset, count)) = limit {
            apply_limit(&mut sorted, offset, count);
        }
        Ok(members_to_resp(&sorted, withscores))
    }

    pub async fn zcard(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_args("ZCARD", args, 1)?;
        let zset = self.load_zset(db, &args[0]).await?;
        Ok(router::integer(zset.map(|z| z.len() as i64).unwrap_or(0)))
    }

    pub async fn zcount(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_args("ZCOUNT", args, 3)?;
        let min = parse_score_bound(bytes_to_str(&args[1])?, true)?;
        let max = parse_score_bound(bytes_to_str(&args[2])?, false)?;
        let zset = self.load_zset(db, &args[0]).await?;
        let count = zset
            .map(|z| {
                z.values()
                    .filter(|score| score_in_range(**score, &min, &max))
                    .count() as i64
            })
            .unwrap_or(0);
        Ok(router::integer(count))
    }

    pub async fn zincrby(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_args("ZINCRBY", args, 3)?;
        let increment = parse_score(&args[1])?;
        let member = args[2].to_vec();
        let key = &args[0];
        let _lock = self.key_lock.lock(key).await;
        let mut stored = self.load_or_create_zset(db, key).await?;
        let zset = stored.as_zset_mut()?;
        let new_score = zset.get(&member).copied().unwrap_or(0.0) + increment;
        zset.insert(member, new_score);
        self.storage.set_typed(db, key, stored).await?;
        Ok(router::bulk(format_score(new_score).into_bytes()))
    }

    pub async fn zscan(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_min_args("ZSCAN", args, 2)?;
        let cursor = scan_util::parse_u64(&args[1])?;
        let opts = scan_util::parse_scan_options("ZSCAN", args, 2)?;
        let zset = self.load_zset(db, &args[0]).await?;
        let Some(zset) = zset else {
            return Ok(scan_response(0, &[]));
        };
        let entries: Vec<(Vec<u8>, f64)> = zset
            .iter()
            .filter(|(m, _)| {
                opts.pattern
                    .as_ref()
                    .is_none_or(|p| glob_match(p, m.as_slice()))
            })
            .map(|(m, s)| (m.clone(), *s))
            .collect();
        let (next_cursor, page) = scan_util::paginate_slice(&entries, cursor, opts.count);
        Ok(scan_response(next_cursor, page))
    }

    pub async fn zpopmin(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_args("ZPOPMIN", args, 1)?;
        let count = if args.len() > 1 {
            parse_i64(&args[1])? as usize
        } else {
            1
        };
        self.pop_extreme(db, &args[0], count, false).await
    }

    pub async fn zpopmax(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_args("ZPOPMAX", args, 1)?;
        let count = if args.len() > 1 {
            parse_i64(&args[1])? as usize
        } else {
            1
        };
        self.pop_extreme(db, &args[0], count, true).await
    }

    async fn pop_extreme(
        &self,
        db: usize,
        key: &Bytes,
        count: usize,
        max: bool,
    ) -> Result<RespValue> {
        let _lock = self.key_lock.lock(key).await;
        let Some(mut stored) = self.storage.get_typed(db, key).await? else {
            return Ok(RespValue::Array(Some(vec![])));
        };
        let zset = stored.as_zset_mut()?;
        let sorted = sorted_by_score(zset, max);
        let n = count.min(sorted.len());
        let mut items = Vec::with_capacity(n * 2);
        for (member, score) in sorted.into_iter().take(n) {
            zset.remove(&member);
            items.push(router::bulk(member));
            items.push(router::bulk(format_score(score).into_bytes()));
        }
        if zset.is_empty() {
            self.storage.delete(db, key).await?;
        } else {
            self.storage.set_typed(db, key, stored).await?;
        }
        Ok(RespValue::Array(Some(items)))
    }

    pub async fn zrangebylex(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_min_args("ZRANGEBYLEX", args, 3)?;
        self.range_by_lex(db, &args[0], args, false).await
    }

    pub async fn zrevrangebylex(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_min_args("ZREVRANGEBYLEX", args, 3)?;
        self.range_by_lex(db, &args[0], args, true).await
    }

    async fn range_by_lex(
        &self,
        db: usize,
        key: &Bytes,
        args: &[Bytes],
        reverse: bool,
    ) -> Result<RespValue> {
        let (min, max) = if reverse {
            (
                parse_lex_bound(bytes_to_str(&args[2])?)?,
                parse_lex_bound(bytes_to_str(&args[1])?)?,
            )
        } else {
            (
                parse_lex_bound(bytes_to_str(&args[1])?)?,
                parse_lex_bound(bytes_to_str(&args[2])?)?,
            )
        };
        let mut limit: Option<(usize, Option<usize>)> = None;
        let mut i = 3;
        while i < args.len() {
            if eq_ignore_case(&args[i], b"LIMIT") {
                if i + 2 >= args.len() {
                    return Err(router::wrong_args("ZRANGEBYLEX", ""));
                }
                limit = Some(parse_limit_args("ZRANGEBYLEX", &args[i + 1], &args[i + 2])?);
                i += 3;
            } else {
                return Err(router::wrong_args("ZRANGEBYLEX", ""));
            }
        }
        let zset = self.load_zset(db, key).await?;
        let Some(zset) = zset else {
            return Ok(RespValue::Array(Some(vec![])));
        };
        let mut members: Vec<Vec<u8>> = zset
            .keys()
            .filter(|m| lex_in_range(m.as_slice(), &min, &max))
            .cloned()
            .collect();
        if reverse {
            members.reverse();
        }
        if let Some((offset, count)) = limit {
            apply_limit(&mut members, offset, count);
        }
        Ok(array_of_bulk(members))
    }

    pub async fn zlexcount(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_args("ZLEXCOUNT", args, 3)?;
        let min = parse_lex_bound(bytes_to_str(&args[1])?)?;
        let max = parse_lex_bound(bytes_to_str(&args[2])?)?;
        let zset = self.load_zset(db, &args[0]).await?;
        let count = zset
            .map(|z| {
                z.keys()
                    .filter(|m| lex_in_range(m.as_slice(), &min, &max))
                    .count() as i64
            })
            .unwrap_or(0);
        Ok(router::integer(count))
    }

    async fn load_zset(&self, db: usize, key: &[u8]) -> Result<Option<BTreeMap<Vec<u8>, f64>>> {
        let Some(stored) = self.storage.get_typed(db, key).await? else {
            return Ok(None);
        };
        Ok(Some(stored.as_zset()?.clone()))
    }

    async fn load_or_create_zset(&self, db: usize, key: &[u8]) -> Result<StoredValue> {
        match self.storage.get_typed(db, key).await? {
            None => Ok(StoredValue::new_zset(BTreeMap::new())),
            Some(stored) => match stored.value {
                ValueType::ZSet(_) => Ok(stored),
                _ => Err(router::wrongtype()),
            },
        }
    }

    pub async fn zinter(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        let opts = parse_zset_combine_args("ZINTER", args)?;
        let mut zsets = Vec::with_capacity(opts.keys.len());
        for key in &opts.keys {
            let zs = self.load_zset(db, key).await?.unwrap_or_default();
            zsets.push(zs);
        }
        let smallest = zsets
            .iter()
            .enumerate()
            .min_by_key(|(_, z)| z.len())
            .map(|(i, _)| i)
            .unwrap_or(0);
        let mut results = Vec::new();
        for member in zsets[smallest].keys() {
            if zsets.iter().all(|z| z.contains_key(member)) {
                let score = aggregate_score(&zsets, member, &opts.weights, &opts.aggregate);
                results.push((member.clone(), score));
            }
        }
        sort_by_score_then_member(&mut results);
        Ok(members_to_resp(&results, opts.withscores))
    }

    pub async fn zunion(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        let opts = parse_zset_combine_args("ZUNION", args)?;
        let mut zsets = Vec::with_capacity(opts.keys.len());
        for key in &opts.keys {
            let zs = self.load_zset(db, key).await?.unwrap_or_default();
            zsets.push(zs);
        }
        let mut seen: Vec<Vec<u8>> = Vec::new();
        for zset in &zsets {
            for member in zset.keys() {
                if !seen.contains(member) {
                    seen.push(member.clone());
                }
            }
        }
        let mut results = Vec::with_capacity(seen.len());
        for member in &seen {
            let score = aggregate_score(&zsets, member, &opts.weights, &opts.aggregate);
            results.push((member.clone(), score));
        }
        sort_by_score_then_member(&mut results);
        Ok(members_to_resp(&results, opts.withscores))
    }

    pub async fn zdiff(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_min_args("ZDIFF", args, 2)?;
        let nkeys_str = bytes_to_str(&args[0])?;
        let numkeys: usize = nkeys_str
            .parse()
            .map_err(|_| Error::Command("ERR value is not an integer or out of range".into()))?;
        if numkeys < 1 || args.len() < 1 + numkeys {
            return Err(router::wrong_args("ZDIFF", ""));
        }
        let withscores = args.iter().any(|a| eq_ignore_case(a, b"WITHSCORES"));
        let mut zsets = Vec::with_capacity(numkeys);
        for i in 0..numkeys {
            let zs = self.load_zset(db, &args[1 + i]).await?.unwrap_or_default();
            zsets.push(zs);
        }
        let mut results = Vec::new();
        for (member, score) in &zsets[0] {
            let in_others = zsets[1..].iter().any(|z| z.contains_key(member));
            if !in_others {
                results.push((member.clone(), *score));
            }
        }
        sort_by_score_then_member(&mut results);
        Ok(members_to_resp(&results, withscores))
    }

    // -- 阻塞命令 --

    pub async fn bzpopmin(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_min_args("BZPOPMIN", args, 2)?;
        self.bzpop_common(db, args, false).await
    }

    pub async fn bzpopmax(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_min_args("BZPOPMAX", args, 2)?;
        self.bzpop_common(db, args, true).await
    }

    async fn bzpop_common(&self, db: usize, args: &[Bytes], max: bool) -> Result<RespValue> {
        let keys: Vec<&Bytes> = args[..args.len() - 1].iter().collect();
        let timeout_s =
            crate::command::list::ListCommands::parse_timeout_secs(&args[args.len() - 1])?;
        // Non-blocking first pass
        for key in &keys {
            let one = Bytes::from("1");
            let sk = [(*key).clone(), one];
            let r = if max {
                self.zpopmax(db, &sk).await?
            } else {
                self.zpopmin(db, &sk).await?
            };
            if let RespValue::Array(Some(ref items)) = r {
                if !items.is_empty() {
                    return Ok(r);
                }
            }
        }
        if timeout_s == 0.0 {
            return Ok(blocking::nil_blocking_response());
        }
        let dur = if timeout_s > 0.0 {
            Duration::from_secs_f64(timeout_s)
        } else {
            Duration::from_secs(300)
        };
        let _blocked = BlockedClientGuard::enter(&self.metrics);
        let registry = BlockingRegistry::global();
        let deadline = Instant::now() + dur;
        let mut rx: Vec<oneshot::Receiver<RespValue>> = keys
            .iter()
            .map(|k| registry.register(k.to_vec(), dur))
            .collect();
        while Instant::now() < deadline {
            for recv in &mut rx {
                match recv.try_recv() {
                    Ok(_) | Err(oneshot::error::TryRecvError::Closed) => {
                        let rem = deadline.saturating_duration_since(Instant::now());
                        if rem.is_zero() {
                            return Ok(blocking::nil_blocking_response());
                        }
                        for key in &keys {
                            let one = Bytes::from("1");
                            let sk = [(*key).clone(), one];
                            let r = if max {
                                self.zpopmax(db, &sk).await?
                            } else {
                                self.zpopmin(db, &sk).await?
                            };
                            if let RespValue::Array(Some(ref items)) = r {
                                if !items.is_empty() {
                                    return Ok(r);
                                }
                            }
                        }
                        rx = keys
                            .iter()
                            .map(|k| registry.register(k.to_vec(), rem))
                            .collect();
                        break;
                    }
                    Err(oneshot::error::TryRecvError::Empty) => {}
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Ok(blocking::nil_blocking_response())
    }
}

#[derive(Debug, Clone)]
struct ScoreBound {
    value: f64,
    inclusive: bool,
}

#[derive(Debug, Clone)]
enum LexBound {
    NegInf,
    PosInf,
    Value(Vec<u8>, bool),
}

/// `LIMIT offset count`; `count < 0` means unlimited (StackExchange.Redis convention).
fn parse_limit_args(
    cmd: &str,
    offset_b: &Bytes,
    count_b: &Bytes,
) -> Result<(usize, Option<usize>)> {
    let offset = parse_i64(offset_b)?;
    if offset < 0 {
        return Err(router::wrong_args(cmd, ""));
    }
    let count_raw = parse_i64(count_b)?;
    let count = if count_raw < 0 {
        None
    } else {
        Some(count_raw as usize)
    };
    Ok((offset as usize, count))
}

fn apply_limit<T: Clone>(items: &mut Vec<T>, offset: usize, count: Option<usize>) {
    if offset >= items.len() {
        items.clear();
        return;
    }
    match count {
        None => *items = items[offset..].to_vec(),
        Some(c) => {
            let end = (offset + c).min(items.len());
            *items = items[offset..end].to_vec();
        }
    }
}

fn sorted_by_score(zset: &BTreeMap<Vec<u8>, f64>, reverse: bool) -> Vec<(Vec<u8>, f64)> {
    let mut v: Vec<(Vec<u8>, f64)> = zset.iter().map(|(k, s)| (k.clone(), *s)).collect();
    if reverse {
        v.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.0.cmp(&a.0))
        });
    } else {
        v.sort_by(|a, b| {
            a.1.partial_cmp(&b.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
    }
    v
}

fn normalize_range(len: usize, start: i64, stop: i64) -> (usize, usize) {
    let len_i = len as i64;
    let start_idx = if start < 0 {
        (len_i + start).max(0) as usize
    } else {
        (start as usize).min(len.saturating_sub(1))
    };
    let stop_idx = if stop < 0 {
        (len_i + stop).max(0) as usize
    } else {
        (stop as usize).min(len.saturating_sub(1))
    };
    (start_idx, stop_idx)
}

fn members_to_resp(members: &[(Vec<u8>, f64)], withscores: bool) -> RespValue {
    let mut items = Vec::new();
    for (member, score) in members {
        items.push(router::bulk(member.clone()));
        if withscores {
            items.push(router::bulk(format_score(*score).into_bytes()));
        }
    }
    RespValue::Array(Some(items))
}

fn scan_response(cursor: u64, page: &[(Vec<u8>, f64)]) -> RespValue {
    let mut items = vec![RespValue::BulkString(Some(Bytes::from(cursor.to_string())))];
    let mut members = Vec::new();
    for (member, score) in page {
        members.push(router::bulk(member.clone()));
        members.push(router::bulk(format_score(*score).into_bytes()));
    }
    items.push(RespValue::Array(Some(members)));
    RespValue::Array(Some(items))
}

fn array_of_bulk(items: Vec<Vec<u8>>) -> RespValue {
    RespValue::Array(Some(items.into_iter().map(router::bulk).collect()))
}

fn format_score(score: f64) -> String {
    if score.fract() == 0.0 && score.is_finite() {
        format!("{}", score as i64)
    } else {
        score.to_string()
    }
}

fn parse_score(b: &Bytes) -> Result<f64> {
    let s = bytes_to_str(b)?;
    let score = s
        .parse::<f64>()
        .map_err(|_| Error::Command("ERR value is not a valid float".into()))?;
    if !score.is_finite() {
        return Err(Error::Command("ERR value is not a valid float".into()));
    }
    Ok(score)
}

fn parse_score_bound(s: &str, _is_min: bool) -> Result<ScoreBound> {
    if s == "-inf" {
        return Ok(ScoreBound {
            value: f64::NEG_INFINITY,
            inclusive: true,
        });
    }
    if s == "+inf" {
        return Ok(ScoreBound {
            value: f64::INFINITY,
            inclusive: true,
        });
    }
    let (inclusive, num_str) = if let Some(rest) = s.strip_prefix('(') {
        (false, rest)
    } else if let Some(rest) = s.strip_prefix('[') {
        (true, rest)
    } else {
        (true, s)
    };
    let value = num_str
        .parse::<f64>()
        .map_err(|_| Error::Command("ERR value is not a valid float".into()))?;
    Ok(ScoreBound { value, inclusive })
}

fn score_in_range(score: f64, min: &ScoreBound, max: &ScoreBound) -> bool {
    let above = if min.inclusive {
        score >= min.value
    } else {
        score > min.value
    };
    let below = if max.inclusive {
        score <= max.value
    } else {
        score < max.value
    };
    above && below
}

fn parse_lex_bound(s: &str) -> Result<LexBound> {
    if s == "-" {
        return Ok(LexBound::NegInf);
    }
    if s == "+" {
        return Ok(LexBound::PosInf);
    }
    let (inclusive, rest) = if let Some(r) = s.strip_prefix('(') {
        (false, r)
    } else if let Some(r) = s.strip_prefix('[') {
        (true, r)
    } else {
        (true, s)
    };
    Ok(LexBound::Value(rest.as_bytes().to_vec(), inclusive))
}

fn lex_in_range(member: &[u8], min: &LexBound, max: &LexBound) -> bool {
    let above_min = match min {
        LexBound::NegInf => true,
        LexBound::PosInf => false,
        LexBound::Value(v, inclusive) => {
            if *inclusive {
                member >= v.as_slice()
            } else {
                member > v.as_slice()
            }
        }
    };
    let below_max = match max {
        LexBound::PosInf => true,
        LexBound::NegInf => false,
        LexBound::Value(v, inclusive) => {
            if *inclusive {
                member <= v.as_slice()
            } else {
                member < v.as_slice()
            }
        }
    };
    above_min && below_max
}

fn bytes_to_str(b: &Bytes) -> Result<&str> {
    std::str::from_utf8(b).map_err(|_| Error::Command("ERR syntax error".into()))
}

fn parse_i64(b: &Bytes) -> Result<i64> {
    let s =
        std::str::from_utf8(b).map_err(|_| Error::Command("ERR value is not an integer".into()))?;
    s.parse::<i64>()
        .map_err(|_| Error::Command("ERR value is not an integer".into()))
}

fn eq_ignore_case(a: &Bytes, b: &[u8]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b.iter())
            .all(|(x, y)| x.eq_ignore_ascii_case(y))
}

// ---- ZINTER / ZUNION 辅助类型 ----

#[derive(Debug, Clone, Copy, PartialEq)]
enum Aggregate {
    Sum,
    Min,
    Max,
}

struct ZSetCombineArgs {
    keys: Vec<Bytes>,
    weights: Vec<f64>,
    aggregate: Aggregate,
    withscores: bool,
}

fn parse_zset_combine_args(cmd: &str, args: &[Bytes]) -> Result<ZSetCombineArgs> {
    router::require_min_args(cmd, args, 2)?;
    let nkeys_str = bytes_to_str(&args[0])?;
    let numkeys: usize = nkeys_str
        .parse()
        .map_err(|_| Error::Command("ERR value is not an integer or out of range".into()))?;
    if numkeys < 1 || args.len() < 1 + numkeys {
        return Err(Error::Command(format!(
            "ERR at least 1 input key is needed for the '{cmd}' command"
        )));
    }
    let keys = args[1..1 + numkeys].to_vec();
    let mut weights: Vec<f64> = Vec::new();
    let mut aggregate = Aggregate::Sum;
    let mut withscores = false;
    let mut i = 1 + numkeys;
    while i < args.len() {
        if eq_ignore_case(&args[i], b"WEIGHTS") {
            if i + numkeys >= args.len() {
                return Err(router::wrong_args(cmd, ""));
            }
            for j in 0..numkeys {
                weights.push(parse_score(&args[i + 1 + j])?);
            }
            i += 1 + numkeys;
        } else if eq_ignore_case(&args[i], b"AGGREGATE") {
            if i + 1 >= args.len() {
                return Err(router::wrong_args(cmd, ""));
            }
            aggregate = parse_aggregate(&args[i + 1])?;
            i += 2;
        } else if eq_ignore_case(&args[i], b"WITHSCORES") {
            withscores = true;
            i += 1;
        } else {
            return Err(router::wrong_args(cmd, ""));
        }
    }
    Ok(ZSetCombineArgs {
        keys,
        weights,
        aggregate,
        withscores,
    })
}

fn parse_aggregate(b: &Bytes) -> Result<Aggregate> {
    let s = bytes_to_str(b)?;
    match s.to_ascii_uppercase().as_str() {
        "SUM" => Ok(Aggregate::Sum),
        "MIN" => Ok(Aggregate::Min),
        "MAX" => Ok(Aggregate::Max),
        _ => Err(Error::Command(
            "ERR AGGREGATE must be SUM, MIN, or MAX".into(),
        )),
    }
}

fn aggregate_score(
    zsets: &[BTreeMap<Vec<u8>, f64>],
    member: &[u8],
    weights: &[f64],
    aggregate: &Aggregate,
) -> f64 {
    let mut result = match aggregate {
        Aggregate::Sum => 0.0,
        Aggregate::Min => f64::INFINITY,
        Aggregate::Max => f64::NEG_INFINITY,
    };
    let mut has_score = false;
    for (i, zset) in zsets.iter().enumerate() {
        if let Some(score) = zset.get(member) {
            let w = weights.get(i).copied().unwrap_or(1.0);
            let weighted = score * w;
            match aggregate {
                Aggregate::Sum => result += weighted,
                Aggregate::Min => {
                    if !has_score || weighted < result {
                        result = weighted;
                    }
                }
                Aggregate::Max => {
                    if !has_score || weighted > result {
                        result = weighted;
                    }
                }
            }
            has_score = true;
        }
    }
    result
}

fn sort_by_score_then_member(items: &mut [(Vec<u8>, f64)]) {
    items.sort_by(|a, b| {
        a.1.partial_cmp(&b.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
}
