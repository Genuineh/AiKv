//! List 命令

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use tokio::sync::oneshot;
use tracing::instrument;

use crate::command::blocking::{self, BlockedClientGuard, BlockingRegistry};
use crate::command::router::{self, KeyLock};
use crate::error::{Error, Result};
use crate::protocol::RespValue;
use crate::server::ServerMetrics;
use crate::storage::{KvStorage, StoredValue, ValueType};

pub struct ListCommands {
    storage: Arc<dyn KvStorage>,
    key_lock: Arc<KeyLock>,
    metrics: Option<Arc<ServerMetrics>>,
}

impl ListCommands {
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

    #[instrument(name = "cmd_list", skip(self, args), fields(cmd.name = "LPUSH"))]
    pub async fn lpush(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_min_args("LPUSH", args, 2)?;
        self.push(db, &args[0], &args[1..], true).await
    }

    pub async fn rpush(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_min_args("RPUSH", args, 2)?;
        self.push(db, &args[0], &args[1..], false).await
    }

    async fn push(
        &self,
        db: usize,
        key: &Bytes,
        elements: &[Bytes],
        front: bool,
    ) -> Result<RespValue> {
        let _lock = self.key_lock.lock(key).await;
        let mut stored = self.load_or_create_list(db, key).await?;
        let list = stored.as_list_mut()?;
        if front {
            for el in elements {
                list.push_front(el.to_vec());
            }
        } else {
            for el in elements {
                list.push_back(el.to_vec());
            }
        }
        let len = list.len() as i64;
        self.storage.set_typed(db, key, stored).await?;
        BlockingRegistry::global().notify(key, RespValue::SimpleString("OK".into()));
        Ok(router::integer(len))
    }

    pub async fn lpop(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_args("LPOP", args, 1)?;
        let count = if args.len() > 1 {
            Some(parse_pop_count(&args[1])?)
        } else {
            None
        };
        self.pop(db, &args[0], count, true).await
    }

    pub async fn rpop(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_args("RPOP", args, 1)?;
        let count = if args.len() > 1 {
            Some(parse_pop_count(&args[1])?)
        } else {
            None
        };
        self.pop(db, &args[0], count, false).await
    }

    async fn pop(
        &self,
        db: usize,
        key: &Bytes,
        count: Option<i64>,
        front: bool,
    ) -> Result<RespValue> {
        let _lock = self.key_lock.lock(key).await;
        let Some(mut stored) = self.storage.get_typed(db, key).await? else {
            return Ok(router::nil_bulk());
        };
        let list = stored.as_list_mut()?;
        let count = count.unwrap_or(1);
        if count == 0 {
            return Ok(RespValue::Array(Some(vec![])));
        }
        let n = (count as usize).min(list.len());
        let mut popped = Vec::with_capacity(n);
        for _ in 0..n {
            let v = if front {
                list.pop_front()
            } else {
                list.pop_back()
            };
            popped.push(v.expect("len checked"));
        }
        if list.is_empty() {
            self.storage.delete(db, key).await?;
        } else {
            self.storage.set_typed(db, key, stored).await?;
        }
        if count == 1 && popped.len() == 1 {
            return Ok(router::bulk(popped.into_iter().next().unwrap()));
        }
        Ok(array_of_bulk(popped))
    }

    pub async fn llen(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_args("LLEN", args, 1)?;
        let list = self.load_list(db, &args[0]).await?;
        Ok(router::integer(list.map(|l| l.len() as i64).unwrap_or(0)))
    }

    pub async fn lrange(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_args("LRANGE", args, 3)?;
        let start = parse_i64(&args[1])?;
        let stop = parse_i64(&args[2])?;
        let list = self.load_list(db, &args[0]).await?;
        let Some(list) = list else {
            return Ok(RespValue::Array(Some(vec![])));
        };
        let (start_idx, stop_idx) = normalize_range(list.len(), start, stop);
        if start_idx > stop_idx {
            return Ok(RespValue::Array(Some(vec![])));
        }
        let items: Vec<Vec<u8>> = list
            .iter()
            .skip(start_idx)
            .take(stop_idx - start_idx + 1)
            .cloned()
            .collect();
        Ok(array_of_bulk(items))
    }

    pub async fn lindex(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_args("LINDEX", args, 2)?;
        let index = parse_i64(&args[1])?;
        let list = self.load_list(db, &args[0]).await?;
        let Some(list) = list else {
            return Ok(router::nil_bulk());
        };
        let idx = normalize_index(list.len(), index);
        match list.get(idx) {
            None => Ok(router::nil_bulk()),
            Some(v) => Ok(router::bulk(v.clone())),
        }
    }

    pub async fn lset(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_args("LSET", args, 3)?;
        let index = parse_i64(&args[1])?;
        let _lock = self.key_lock.lock(&args[0]).await;
        let Some(mut stored) = self.storage.get_typed(db, &args[0]).await? else {
            return Err(Error::Command("ERR no such key".into()));
        };
        let list = stored.as_list_mut()?;
        let idx = normalize_index(list.len(), index);
        let Some(slot) = list.get_mut(idx) else {
            return Err(Error::Command("ERR index out of range".into()));
        };
        *slot = args[2].to_vec();
        self.storage.set_typed(db, &args[0], stored).await?;
        Ok(router::ok())
    }

    pub async fn lrem(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_args("LREM", args, 3)?;
        let count = parse_i64(&args[1])?;
        let element = args[2].as_ref();
        let _lock = self.key_lock.lock(&args[0]).await;
        let Some(mut stored) = self.storage.get_typed(db, &args[0]).await? else {
            return Ok(router::integer(0));
        };
        let list = stored.as_list_mut()?;
        let removed = if count == 0 {
            let before = list.len();
            list.retain(|v| v.as_slice() != element);
            before - list.len()
        } else if count > 0 {
            remove_direction(list, element, count as usize, true)
        } else {
            remove_direction(list, element, (-count) as usize, false)
        };
        if list.is_empty() {
            self.storage.delete(db, &args[0]).await?;
        } else {
            self.storage.set_typed(db, &args[0], stored).await?;
        }
        Ok(router::integer(removed as i64))
    }

    #[instrument(name = "cmd_list", skip(self, args), fields(cmd.name = "LINSERT"))]
    pub async fn linsert(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_args("LINSERT", args, 4)?;
        let key = &args[0];
        let where_arg = &args[1];
        let pivot = args[2].as_ref();
        let element = args[3].to_vec();
        let before = if eq_ignore_case(where_arg, b"BEFORE") {
            true
        } else if eq_ignore_case(where_arg, b"AFTER") {
            false
        } else {
            return Err(Error::Command("ERR syntax error".into()));
        };

        let _lock = self.key_lock.lock(key).await;
        let Some(mut stored) = self.storage.get_typed(db, key).await? else {
            return Ok(router::integer(0));
        };
        let list = stored.as_list_mut()?;
        let Some(idx) = list.iter().position(|v| v.as_slice() == pivot) else {
            return Ok(router::integer(-1));
        };
        let insert_at = if before { idx } else { idx + 1 };
        list.insert(insert_at, element);
        let len = list.len() as i64;
        self.storage.set_typed(db, key, stored).await?;
        Ok(router::integer(len))
    }

    #[instrument(name = "cmd_list", skip(self, args), fields(cmd.name = "LMOVE"))]
    pub async fn lmove(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_args("LMOVE", args, 4)?;
        let source = &args[0];
        let destination = &args[1];
        let from_left = parse_side(&args[2])?;
        let to_left = parse_side(&args[3])?;

        let (_lock_a, _lock_b) = self.key_lock.lock_two(source, destination).await;
        let same_key = source == destination;

        let Some(mut src_stored) = self.storage.get_typed(db, source).await? else {
            return Ok(router::nil_bulk());
        };
        let src_list = src_stored.as_list_mut()?;
        if src_list.is_empty() {
            return Ok(router::nil_bulk());
        }

        if !same_key {
            if let Some(dest_stored) = self.storage.get_typed(db, destination).await? {
                if !matches!(dest_stored.value, ValueType::List(_)) {
                    return Err(router::wrongtype());
                }
            }
        }

        let element = if from_left {
            src_list.pop_front().expect("non-empty")
        } else {
            src_list.pop_back().expect("non-empty")
        };

        if same_key {
            if to_left {
                src_list.push_front(element.clone());
            } else {
                src_list.push_back(element.clone());
            }
            self.storage.set_typed(db, source, src_stored).await?;
            return Ok(router::bulk(element));
        }

        if src_list.is_empty() {
            self.storage.delete(db, source).await?;
        } else {
            self.storage.set_typed(db, source, src_stored).await?;
        }

        let mut dest_stored = match self.storage.get_typed(db, destination).await? {
            None => StoredValue::new_list(VecDeque::new()),
            Some(stored) => stored,
        };
        let dest_list = dest_stored.as_list_mut()?;
        if to_left {
            dest_list.push_front(element.clone());
        } else {
            dest_list.push_back(element.clone());
        }
        self.storage.set_typed(db, destination, dest_stored).await?;
        Ok(router::bulk(element))
    }

    #[instrument(name = "cmd_list", skip(self, args), fields(cmd.name = "LPOS"))]
    pub async fn lpos(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_min_args("LPOS", args, 2)?;
        let key = &args[0];
        let element = args[1].as_ref();
        let opts = parse_lpos_options(&args[2..])?;

        let Some(stored) = self.storage.get_typed(db, key).await? else {
            return if opts.count.is_some() {
                Ok(RespValue::Array(Some(vec![])))
            } else {
                Ok(router::nil_bulk())
            };
        };
        let list = stored.as_list()?;
        find_lpos(list, element, &opts)
    }

    pub async fn ltrim(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_args("LTRIM", args, 3)?;
        let start = parse_i64(&args[1])?;
        let stop = parse_i64(&args[2])?;
        let _lock = self.key_lock.lock(&args[0]).await;
        let Some(mut stored) = self.storage.get_typed(db, &args[0]).await? else {
            return Ok(router::ok());
        };
        let list = stored.as_list_mut()?;
        let (start_idx, stop_idx) = normalize_range(list.len(), start, stop);
        if start_idx > stop_idx {
            self.storage.delete(db, &args[0]).await?;
            return Ok(router::ok());
        }
        let trimmed: VecDeque<Vec<u8>> = list
            .iter()
            .skip(start_idx)
            .take(stop_idx - start_idx + 1)
            .cloned()
            .collect();
        *list = trimmed;
        if list.is_empty() {
            self.storage.delete(db, &args[0]).await?;
        } else {
            self.storage.set_typed(db, &args[0], stored).await?;
        }
        Ok(router::ok())
    }

    async fn load_list(&self, db: usize, key: &[u8]) -> Result<Option<VecDeque<Vec<u8>>>> {
        let Some(stored) = self.storage.get_typed(db, key).await? else {
            return Ok(None);
        };
        Ok(Some(stored.as_list()?.clone()))
    }

    async fn load_or_create_list(&self, db: usize, key: &[u8]) -> Result<StoredValue> {
        match self.storage.get_typed(db, key).await? {
            None => Ok(StoredValue::new_list(VecDeque::new())),
            Some(stored) => match stored.value {
                ValueType::List(_) => Ok(stored),
                _ => Err(router::wrongtype()),
            },
        }
    }

    // -- 阻塞命令 --

    /// Parse timeout float from Bytes (last arg for BLPOP/BRPOP/BZPOP*)
    pub(crate) fn parse_timeout_secs(b: &Bytes) -> Result<f64> {
        let s = std::str::from_utf8(b)
            .map_err(|_| Error::Command("ERR timeout is not a float".into()))?;
        s.parse::<f64>()
            .map_err(|_| Error::Command("ERR timeout is not a float".into()))
    }

    /// Non-blocking: try to pop from first non-empty key. Returns None if all empty.
    async fn try_pop_any(
        &self,
        db: usize,
        keys: &[&Bytes],
        left: bool,
    ) -> Result<Option<RespValue>> {
        for &key in keys {
            let Some(mut stored) = self.storage.get_typed(db, key).await? else {
                continue;
            };
            let list = match stored.as_list_mut() {
                Ok(l) => l,
                Err(_) => continue,
            };
            if list.is_empty() {
                continue;
            }
            let element = if left {
                list.pop_front().expect("non-empty")
            } else {
                list.pop_back().expect("non-empty")
            };
            if list.is_empty() {
                self.storage.delete(db, key).await?;
            } else {
                self.storage.set_typed(db, key, stored).await?;
            }
            return Ok(Some(RespValue::Array(Some(vec![
                RespValue::BulkString(Some(key.clone())),
                RespValue::BulkString(Some(bytes::Bytes::from(element))),
            ]))));
        }
        Ok(None)
    }

    pub async fn blpop(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_min_args("BLPOP", args, 2)?;
        let keys: Vec<&Bytes> = args[..args.len() - 1].iter().collect();
        let timeout = Self::parse_timeout_secs(&args[args.len() - 1])?;
        self.blocking_pop(db, &keys, timeout, true).await
    }

    pub async fn brpop(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_min_args("BRPOP", args, 2)?;
        let keys: Vec<&Bytes> = args[..args.len() - 1].iter().collect();
        let timeout = Self::parse_timeout_secs(&args[args.len() - 1])?;
        self.blocking_pop(db, &keys, timeout, false).await
    }

    async fn blocking_pop(
        &self,
        db: usize,
        keys: &[&Bytes],
        timeout_secs: f64,
        left: bool,
    ) -> Result<RespValue> {
        // Try non-blocking first
        if let Some(result) = self.try_pop_any(db, keys, left).await? {
            return Ok(result);
        }

        if timeout_secs == 0.0 {
            return Ok(blocking::nil_blocking_response());
        }

        let dur = if timeout_secs > 0.0 {
            Duration::from_secs_f64(timeout_secs)
        } else {
            Duration::from_secs(300)
        };

        let _blocked = BlockedClientGuard::enter(&self.metrics);

        let registry = BlockingRegistry::global();
        let mut receivers: Vec<oneshot::Receiver<RespValue>> = keys
            .iter()
            .map(|k| registry.register(k.to_vec(), dur))
            .collect();

        let deadline = Instant::now() + dur;
        while Instant::now() < deadline {
            for rx in &mut receivers {
                match rx.try_recv() {
                    Ok(_) | Err(oneshot::error::TryRecvError::Closed) => {
                        let remaining = deadline.saturating_duration_since(Instant::now());
                        if remaining.is_zero() {
                            return Ok(blocking::nil_blocking_response());
                        }
                        if let Some(result) = self.try_pop_any(db, keys, left).await? {
                            return Ok(result);
                        }
                        // Element taken by another waiter, re-register
                        receivers = keys
                            .iter()
                            .map(|k| registry.register(k.to_vec(), remaining))
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

    /// BLMOVE with blocking when source is empty
    pub async fn blmove_blocking(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        router::require_args("BLMOVE", args, 5)?;
        let source = &args[0];
        let timeout = Self::parse_timeout_secs(&args[4])?;

        // Try non-blocking first
        let immediate = self.lmove(db, &args[..4]).await?;
        if !matches!(&immediate, RespValue::BulkString(None)) {
            return Ok(immediate);
        }

        if timeout == 0.0 {
            return Ok(blocking::nil_blocking_response());
        }

        let dur = if timeout > 0.0 {
            Duration::from_secs_f64(timeout)
        } else {
            Duration::from_secs(300)
        };

        let _blocked = BlockedClientGuard::enter(&self.metrics);

        let registry = BlockingRegistry::global();
        let deadline = Instant::now() + dur;

        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(blocking::nil_blocking_response());
            }
            let mut rx = registry.register(source.to_vec(), remaining);

            let result = tokio::time::timeout(remaining, &mut rx).await;
            match result {
                Ok(_) => {
                    // Notified or sender dropped, retry LMOVE
                    let retry = self.lmove(db, &args[..4]).await?;
                    if !matches!(&retry, RespValue::BulkString(None)) {
                        return Ok(retry);
                    }
                    // Element taken, loop to re-register
                }
                Err(_) => {
                    // Timeout
                    return Ok(blocking::nil_blocking_response());
                }
            }
        }

        Ok(blocking::nil_blocking_response())
    }
}

fn normalize_range(len: usize, start: i64, stop: i64) -> (usize, usize) {
    let len_i = len as i64;
    let start_idx = if start < 0 {
        (len_i + start).max(0) as usize
    } else {
        (start as usize).min(len)
    };
    let stop_idx = if stop < 0 {
        (len_i + stop).max(0) as usize
    } else {
        (stop as usize).min(len.saturating_sub(1))
    };
    (start_idx, stop_idx)
}

fn normalize_index(len: usize, index: i64) -> usize {
    if index < 0 {
        (len as i64 + index) as usize
    } else {
        index as usize
    }
}

fn remove_direction(
    list: &mut VecDeque<Vec<u8>>,
    element: &[u8],
    max: usize,
    from_front: bool,
) -> usize {
    let mut removed = 0;
    if from_front {
        let mut i = 0;
        while i < list.len() && removed < max {
            if list[i].as_slice() == element {
                list.remove(i);
                removed += 1;
            } else {
                i += 1;
            }
        }
    } else {
        let mut i = list.len();
        while i > 0 && removed < max {
            i -= 1;
            if list[i].as_slice() == element {
                list.remove(i);
                removed += 1;
            }
        }
    }
    removed
}

fn array_of_bulk(items: Vec<Vec<u8>>) -> RespValue {
    RespValue::Array(Some(items.into_iter().map(router::bulk).collect()))
}

fn parse_i64(b: &Bytes) -> Result<i64> {
    let s =
        std::str::from_utf8(b).map_err(|_| Error::Command("ERR value is not an integer".into()))?;
    s.parse::<i64>()
        .map_err(|_| Error::Command("ERR value is not an integer".into()))
}

fn parse_pop_count(b: &Bytes) -> Result<i64> {
    let n = parse_i64(b)?;
    if n < 0 {
        return Err(Error::Command(
            "ERR value is not an integer or out of range".into(),
        ));
    }
    Ok(n)
}

fn eq_ignore_case(a: &Bytes, b: &[u8]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b.iter())
            .all(|(x, y)| x.eq_ignore_ascii_case(y))
}

fn parse_side(b: &Bytes) -> Result<bool> {
    if eq_ignore_case(b, b"LEFT") {
        Ok(true)
    } else if eq_ignore_case(b, b"RIGHT") {
        Ok(false)
    } else {
        Err(Error::Command("ERR syntax error".into()))
    }
}

struct LposOptions {
    rank: i64,
    count: Option<i64>,
    maxlen: i64,
}

fn parse_lpos_options(args: &[Bytes]) -> Result<LposOptions> {
    let mut rank = 1_i64;
    let mut count = None;
    let mut maxlen = 0_i64;
    let mut i = 0;
    while i < args.len() {
        if eq_ignore_case(&args[i], b"RANK") {
            if i + 1 >= args.len() {
                return Err(router::wrong_args("LPOS", ""));
            }
            rank = parse_i64(&args[i + 1])?;
            i += 2;
        } else if eq_ignore_case(&args[i], b"COUNT") {
            if i + 1 >= args.len() {
                return Err(router::wrong_args("LPOS", ""));
            }
            count = Some(parse_i64(&args[i + 1])?);
            i += 2;
        } else if eq_ignore_case(&args[i], b"MAXLEN") {
            if i + 1 >= args.len() {
                return Err(router::wrong_args("LPOS", ""));
            }
            maxlen = parse_i64(&args[i + 1])?;
            if maxlen < 0 {
                return Err(Error::Command(
                    "ERR value is not an integer or out of range".into(),
                ));
            }
            i += 2;
        } else {
            return Err(router::wrong_args("LPOS", ""));
        }
    }
    Ok(LposOptions {
        rank,
        count,
        maxlen,
    })
}

fn find_lpos(list: &VecDeque<Vec<u8>>, element: &[u8], opts: &LposOptions) -> Result<RespValue> {
    if opts.rank == 0 {
        return Err(Error::Command("ERR RANK can't be zero".into()));
    }

    let len = list.len();
    let abs_rank = opts.rank.unsigned_abs() as usize;

    let indices: Vec<usize> = if opts.rank > 0 {
        let end = if opts.maxlen > 0 {
            (opts.maxlen as usize).min(len)
        } else {
            len
        };
        (0..end)
            .filter(|&i| list[i].as_slice() == element)
            .collect()
    } else {
        let start = if opts.maxlen > 0 {
            len.saturating_sub(opts.maxlen as usize)
        } else {
            0
        };
        (start..len)
            .rev()
            .filter(|&i| list[i].as_slice() == element)
            .collect()
    };

    if indices.len() < abs_rank {
        return if opts.count.is_some() {
            Ok(RespValue::Array(Some(vec![])))
        } else {
            Ok(router::nil_bulk())
        };
    }

    let remaining = &indices[abs_rank - 1..];
    match opts.count {
        None => Ok(router::integer(remaining[0] as i64)),
        Some(0) => Ok(RespValue::Array(Some(
            remaining
                .iter()
                .map(|&i| router::integer(i as i64))
                .collect(),
        ))),
        Some(n) if n > 0 => {
            let take = (n as usize).min(remaining.len());
            Ok(RespValue::Array(Some(
                remaining[..take]
                    .iter()
                    .map(|&i| router::integer(i as i64))
                    .collect(),
            )))
        }
        Some(_) => Err(Error::Command(
            "ERR value is not an integer or out of range".into(),
        )),
    }
}
