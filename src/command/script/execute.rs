//! Lua 脚本内 `redis.call` / `redis.pcall` 的异步执行: 参数转换 → KEYS 声明校验 →
//! 按命令名 dispatch 到各域 exec_* → 写操作进 `ScriptTransaction` 缓冲, 读操作优先读缓冲.
//!
//! # 执行流程
//!
//! ```text
//! redis.call("GET", key, …) ── mlua 闭包 ──> redis_call_async(lua, storage, txn, declared, args, throw_error)
//!   │ 1. Lua 参数 → Bytes: String/Integer/Number/Boolean; 其它类型 throw_error → RuntimeError, 否则 Nil
//!   ├─ 2. validate_keys: 用 command_key_indices 定位命令 key 参数, 须在 EVAL 声明的 KEYS 内
//!   │       未声明 → "ERR Script attempted to access undeclared key"
//!   ├─ 3. dispatch(cmd): match 到各域 exec_* (String/Hash/List/Set/ZSet/JSON/EXPIRE)
//!   │       读: txn.get / txn.get_value (写缓冲优先; CollectionHeader 透明展开为完整集合)
//!   │       写: txn.set_* / txn.delete / txn.set_expire_at 缓冲
//!   ├─ 4a. 成功 → resp_to_lua (RESP → LuaValue)
//!   └─ 4b. 失败 → throw_error ? mlua RuntimeError : pcall_error_table ({err="…"})
//! ```
//!
//! # Invariant
//!
//! - KEYS 校验: 脚本访问未声明的 key 报错 (redis.call 抛错 / redis.pcall 返回 `{err}` 表).
//! - 写操作全部进入 `ScriptTransaction.write_buffer`, 由 `script.rs::execute_script` 结束后
//!   单次 `commit` 落盘; 脚本失败时缓冲随 txn drop 丢弃, 保证原子性.
//! - `pcall` 错误返回 `{err="…"}` 表 (非 nil), 与 `convert.rs::pcall_error_table` 对齐.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use bytes::Bytes;

use crate::command::router;
use crate::command::script::convert::{pcall_error_table, resp_to_lua};
use crate::command::script::json_exec;
use crate::command::script::transaction::{
    expire_seconds_to_at, parse_f64_arg, parse_i64_arg, ScriptTransaction,
};
use crate::error::{Error, Result};
use crate::protocol::RespValue;
use crate::storage::KvStorage;
use tracing::Instrument;

fn wrong_count(cmd: &str) -> Error {
    router::wrong_args(cmd, "")
}

fn key(b: &Bytes) -> Vec<u8> {
    b.to_vec()
}

fn validate_keys(declared: &HashSet<Vec<u8>>, cmd: &str, args: &[Bytes]) -> Result<()> {
    for idx in command_key_indices(cmd, args.len()) {
        if idx >= args.len() {
            continue;
        }
        let k = key(&args[idx]);
        if !declared.contains(&k) {
            return Err(Error::Command(format!(
                "ERR Script attempted to access undeclared key '{k:?}'"
            )));
        }
    }
    Ok(())
}

fn command_key_indices(cmd: &str, argc: usize) -> Vec<usize> {
    match cmd {
        "DEL" | "EXISTS" => (0..argc).collect(),
        "HMGET" | "HDEL" | "HSET" | "HMSET" | "HINCRBY" | "HEXISTS" => {
            if argc > 0 {
                vec![0]
            } else {
                vec![]
            }
        }
        "LPUSH" | "RPUSH" | "LPOP" | "RPOP" | "LLEN" | "LRANGE" | "LINDEX" => {
            if argc > 0 {
                vec![0]
            } else {
                vec![]
            }
        }
        "SADD" | "SREM" | "SMEMBERS" | "SISMEMBER" | "SCARD" | "SSCAN" => {
            if argc > 0 {
                vec![0]
            } else {
                vec![]
            }
        }
        "JSON.MGET" => {
            if argc >= 2 {
                (0..argc - 1).collect()
            } else {
                vec![]
            }
        }
        "JSON.SET" | "JSON.GET" | "JSON.DEL" | "JSON.TYPE" | "JSON.STRLEN" | "JSON.ARRAPPEND"
        | "JSON.NUMINCRBY" | "JSON.OBJLEN" | "JSON.ARRLEN" => {
            if argc > 0 {
                vec![0]
            } else {
                vec![]
            }
        }
        "ZADD" | "ZREM" | "ZSCORE" | "ZRANK" | "ZRANGE" | "ZCARD" => {
            if argc > 0 {
                vec![0]
            } else {
                vec![]
            }
        }
        _ => {
            if argc > 0 {
                vec![0]
            } else {
                vec![]
            }
        }
    }
}

pub async fn redis_call_async(
    lua: &mlua::Lua,
    storage: &dyn KvStorage,
    txn: &mut ScriptTransaction,
    declared: &HashSet<Vec<u8>>,
    args: mlua::MultiValue,
    throw_error: bool,
) -> mlua::Result<mlua::Value> {
    let mut cmd_args: Vec<Bytes> = Vec::new();
    for arg in args {
        match arg {
            mlua::Value::String(s) => cmd_args.push(Bytes::from(s.as_bytes().to_vec())),
            mlua::Value::Integer(i) => cmd_args.push(Bytes::from(i.to_string())),
            mlua::Value::Number(n) => cmd_args.push(Bytes::from(n.to_string())),
            mlua::Value::Boolean(b) => cmd_args.push(Bytes::from(if b { "1" } else { "0" })),
            _ => {
                if throw_error {
                    return Err(mlua::Error::RuntimeError(
                        "Invalid argument type".to_string(),
                    ));
                }
                return Ok(mlua::Value::Nil);
            }
        }
    }

    if cmd_args.is_empty() {
        if throw_error {
            return Err(mlua::Error::RuntimeError(
                "No command specified".to_string(),
            ));
        }
        return Ok(mlua::Value::Nil);
    }

    let command = String::from_utf8_lossy(&cmd_args[0]).to_ascii_uppercase();
    let command_args = &cmd_args[1..];

    if let Err(e) = validate_keys(declared, &command, command_args) {
        if throw_error {
            return Err(mlua::Error::RuntimeError(e.to_string()));
        }
        return pcall_error_table(lua, &e.to_string());
    }

    async fn run(
        lua: &mlua::Lua,
        storage: &dyn KvStorage,
        txn: &mut ScriptTransaction,
        command: &str,
        command_args: &[Bytes],
        throw_error: bool,
    ) -> mlua::Result<mlua::Value> {
        let result = dispatch(storage, txn, command, command_args).await;
        match result {
            Ok(resp) => resp_to_lua(lua, resp),
            Err(e) => {
                let msg = match &e {
                    Error::Command(s) => s.clone(),
                    _ => e.to_string(),
                };
                if throw_error {
                    Err(mlua::Error::RuntimeError(msg))
                } else {
                    pcall_error_table(lua, &msg)
                }
            }
        }
    }

    run(lua, storage, txn, &command, command_args, throw_error)
        .instrument(tracing::debug_span!("cmd_lua_redis_call", cmd.name = %command))
        .await
}

async fn dispatch(
    storage: &dyn KvStorage,
    txn: &mut ScriptTransaction,
    cmd: &str,
    args: &[Bytes],
) -> Result<RespValue> {
    match cmd {
        "GET" => exec_get(storage, txn, args).await,
        "SET" => exec_set(txn, args),
        "DEL" => exec_del(storage, txn, args).await,
        "EXISTS" => exec_exists(storage, txn, args).await,
        "INCR" => exec_incr(storage, txn, args, 1).await,
        "DECR" => exec_incr(storage, txn, args, -1).await,
        "INCRBY" => exec_incrby(storage, txn, args).await,
        "DECRBY" => exec_decrby(storage, txn, args).await,
        "INCRBYFLOAT" => exec_incrbyfloat(storage, txn, args).await,
        "APPEND" => exec_append(storage, txn, args).await,
        "STRLEN" => exec_strlen(storage, txn, args).await,
        "HGET" => exec_hget(storage, txn, args).await,
        "HSET" | "HMSET" => exec_hset(storage, txn, args).await,
        "HDEL" => exec_hdel(storage, txn, args).await,
        "HGETALL" => exec_hgetall(storage, txn, args).await,
        "HMGET" => exec_hmget(storage, txn, args).await,
        "HINCRBY" => exec_hincrby(storage, txn, args).await,
        "HEXISTS" => exec_hexists(storage, txn, args).await,
        "HLEN" => exec_hlen(storage, txn, args).await,
        "LPUSH" => exec_lpush(storage, txn, args, true).await,
        "RPUSH" => exec_lpush(storage, txn, args, false).await,
        "LPOP" => exec_lpop(storage, txn, args, true).await,
        "RPOP" => exec_lpop(storage, txn, args, false).await,
        "LLEN" => exec_llen(storage, txn, args).await,
        "LRANGE" => exec_lrange(storage, txn, args).await,
        "LINDEX" => exec_lindex(storage, txn, args).await,
        "SADD" => exec_sadd(storage, txn, args).await,
        "SREM" => exec_srem(storage, txn, args).await,
        "SMEMBERS" => exec_smembers(storage, txn, args).await,
        "SISMEMBER" => exec_sismember(storage, txn, args).await,
        "SCARD" => exec_scard(storage, txn, args).await,
        "SSCAN" => exec_sscan(storage, txn, args).await,
        "ZADD" => exec_zadd(storage, txn, args).await,
        "ZREM" => exec_zrem(storage, txn, args).await,
        "ZSCORE" => exec_zscore(storage, txn, args).await,
        "ZRANK" => exec_zrank(storage, txn, args).await,
        "ZRANGE" => exec_zrange(storage, txn, args).await,
        "ZCARD" => exec_zcard(storage, txn, args).await,
        "EXPIRE" => exec_expire(storage, txn, args).await,
        "JSON.GET" => json_exec::exec_json_get(storage, txn, args).await,
        "JSON.MGET" => json_exec::exec_json_mget(storage, txn, args).await,
        "JSON.SET" => json_exec::exec_json_set(storage, txn, args).await,
        "JSON.DEL" => json_exec::exec_json_del(storage, txn, args).await,
        "JSON.TYPE" => json_exec::exec_json_type(storage, txn, args).await,
        "JSON.STRLEN" => json_exec::exec_json_strlen(storage, txn, args).await,
        "JSON.ARRLEN" => json_exec::exec_json_arrlen(storage, txn, args).await,
        "JSON.OBJLEN" => json_exec::exec_json_objlen(storage, txn, args).await,
        "JSON.NUMINCRBY" => json_exec::exec_json_numincrby(storage, txn, args).await,
        "JSON.ARRAPPEND" => json_exec::exec_json_arrappend(storage, txn, args).await,
        _ => Err(Error::Command(format!(
            "ERR Command not supported in scripts: {cmd}"
        ))),
    }
}

async fn exec_get(
    storage: &dyn KvStorage,
    txn: &ScriptTransaction,
    args: &[Bytes],
) -> Result<RespValue> {
    if args.len() != 1 {
        return Err(wrong_count("GET"));
    }
    match txn.get(storage, &args[0]).await? {
        Some(v) => Ok(router::bulk(v)),
        None => Ok(RespValue::Null),
    }
}

fn exec_set(txn: &mut ScriptTransaction, args: &[Bytes]) -> Result<RespValue> {
    if args.len() < 2 {
        return Err(wrong_count("SET"));
    }
    txn.set_string(key(&args[0]), args[1].to_vec());
    Ok(RespValue::SimpleString("OK".into()))
}

async fn exec_del(
    storage: &dyn KvStorage,
    txn: &mut ScriptTransaction,
    args: &[Bytes],
) -> Result<RespValue> {
    if args.is_empty() {
        return Err(wrong_count("DEL"));
    }
    let mut count = 0i64;
    for arg in args {
        if txn.exists(storage, arg).await? {
            txn.delete(key(arg));
            count += 1;
        }
    }
    Ok(router::integer(count))
}

async fn exec_exists(
    storage: &dyn KvStorage,
    txn: &ScriptTransaction,
    args: &[Bytes],
) -> Result<RespValue> {
    if args.is_empty() {
        return Err(wrong_count("EXISTS"));
    }
    let mut count = 0i64;
    for arg in args {
        if txn.exists(storage, arg).await? {
            count += 1;
        }
    }
    Ok(router::integer(count))
}

async fn exec_incr(
    storage: &dyn KvStorage,
    txn: &mut ScriptTransaction,
    args: &[Bytes],
    delta: i64,
) -> Result<RespValue> {
    if args.len() != 1 {
        return Err(wrong_count("INCR"));
    }
    let current = match txn.get(storage, &args[0]).await? {
        Some(v) => parse_i64_arg(&Bytes::from(v), "value")?,
        None => 0,
    };
    let new_val = current.saturating_add(delta);
    txn.set_string(key(&args[0]), new_val.to_string().into_bytes());
    Ok(router::integer(new_val))
}

async fn exec_incrby(
    storage: &dyn KvStorage,
    txn: &mut ScriptTransaction,
    args: &[Bytes],
) -> Result<RespValue> {
    if args.len() != 2 {
        return Err(wrong_count("INCRBY"));
    }
    let inc = parse_i64_arg(&args[1], "increment")?;
    exec_incr(storage, txn, &[args[0].clone()], inc).await
}

async fn exec_decrby(
    storage: &dyn KvStorage,
    txn: &mut ScriptTransaction,
    args: &[Bytes],
) -> Result<RespValue> {
    if args.len() != 2 {
        return Err(wrong_count("DECRBY"));
    }
    let dec = parse_i64_arg(&args[1], "decrement")?;
    exec_incr(storage, txn, &[args[0].clone()], -dec).await
}

async fn exec_incrbyfloat(
    storage: &dyn KvStorage,
    txn: &mut ScriptTransaction,
    args: &[Bytes],
) -> Result<RespValue> {
    if args.len() != 2 {
        return Err(wrong_count("INCRBYFLOAT"));
    }
    let inc = parse_f64_arg(&args[1], "increment")?;
    let current = match txn.get(storage, &args[0]).await? {
        Some(v) => parse_f64_arg(&Bytes::from(v), "value")?,
        None => 0.0,
    };
    let new_val = current + inc;
    let s = new_val.to_string();
    txn.set_string(key(&args[0]), s.clone().into_bytes());
    Ok(router::bulk(s.into_bytes()))
}

async fn exec_append(
    storage: &dyn KvStorage,
    txn: &mut ScriptTransaction,
    args: &[Bytes],
) -> Result<RespValue> {
    if args.len() != 2 {
        return Err(wrong_count("APPEND"));
    }
    let mut cur = txn.get(storage, &args[0]).await?.unwrap_or_default();
    cur.extend_from_slice(&args[1]);
    let len = cur.len() as i64;
    txn.set_string(key(&args[0]), cur);
    Ok(router::integer(len))
}

async fn exec_strlen(
    storage: &dyn KvStorage,
    txn: &ScriptTransaction,
    args: &[Bytes],
) -> Result<RespValue> {
    if args.len() != 1 {
        return Err(wrong_count("STRLEN"));
    }
    let len = txn
        .get(storage, &args[0])
        .await?
        .map(|v| v.len())
        .unwrap_or(0) as i64;
    Ok(router::integer(len))
}

async fn exec_hget(
    storage: &dyn KvStorage,
    txn: &ScriptTransaction,
    args: &[Bytes],
) -> Result<RespValue> {
    if args.len() != 2 {
        return Err(wrong_count("HGET"));
    }
    if let Some(stored) = txn.get_value(storage, &args[0]).await? {
        let hash = stored.as_hash()?;
        Ok(match hash.get(args[1].as_ref()) {
            Some(v) => router::bulk(v.clone()),
            None => RespValue::Null,
        })
    } else {
        Ok(RespValue::Null)
    }
}

async fn exec_hset(
    storage: &dyn KvStorage,
    txn: &mut ScriptTransaction,
    args: &[Bytes],
) -> Result<RespValue> {
    if args.len() < 3 || args.len().is_multiple_of(2) {
        return Err(wrong_count("HSET"));
    }
    let mut hash = if let Some(stored) = txn.get_value(storage, &args[0]).await? {
        stored.as_hash()?.clone()
    } else {
        HashMap::new()
    };
    let mut added = 0i64;
    for i in (1..args.len()).step_by(2) {
        if hash
            .insert(args[i].to_vec(), args[i + 1].to_vec())
            .is_none()
        {
            added += 1;
        }
    }
    txn.set_hash(key(&args[0]), hash);
    if args.len() > 3 {
        Ok(RespValue::SimpleString("OK".into()))
    } else {
        Ok(router::integer(added))
    }
}

async fn exec_hdel(
    storage: &dyn KvStorage,
    txn: &mut ScriptTransaction,
    args: &[Bytes],
) -> Result<RespValue> {
    if args.len() < 2 {
        return Err(wrong_count("HDEL"));
    }
    let Some(stored) = txn.get_value(storage, &args[0]).await? else {
        return Ok(router::integer(0));
    };
    let mut hash = stored.as_hash()?.clone();
    let mut count = 0i64;
    for f in &args[1..] {
        if hash.remove(f.as_ref()).is_some() {
            count += 1;
        }
    }
    if hash.is_empty() {
        txn.delete(key(&args[0]));
    } else {
        txn.set_hash(key(&args[0]), hash);
    }
    Ok(router::integer(count))
}

async fn exec_hgetall(
    storage: &dyn KvStorage,
    txn: &ScriptTransaction,
    args: &[Bytes],
) -> Result<RespValue> {
    if args.len() != 1 {
        return Err(wrong_count("HGETALL"));
    }
    let Some(stored) = txn.get_value(storage, &args[0]).await? else {
        return Ok(RespValue::Array(Some(Vec::new())));
    };
    let hash = stored.as_hash()?;
    let mut out = Vec::new();
    for (f, v) in hash {
        out.push(router::bulk(f.clone()));
        out.push(router::bulk(v.clone()));
    }
    Ok(RespValue::Array(Some(out)))
}

async fn exec_hmget(
    storage: &dyn KvStorage,
    txn: &ScriptTransaction,
    args: &[Bytes],
) -> Result<RespValue> {
    if args.len() < 2 {
        return Err(wrong_count("HMGET"));
    }
    let hash = if let Some(stored) = txn.get_value(storage, &args[0]).await? {
        Some(stored.as_hash()?.clone())
    } else {
        None
    };
    let out: Vec<RespValue> = args[1..]
        .iter()
        .map(|f| {
            if let Some(h) = &hash {
                if let Some(v) = h.get(f.as_ref()) {
                    return router::bulk(v.clone());
                }
            }
            RespValue::Null
        })
        .collect();
    Ok(RespValue::Array(Some(out)))
}

async fn exec_hincrby(
    storage: &dyn KvStorage,
    txn: &mut ScriptTransaction,
    args: &[Bytes],
) -> Result<RespValue> {
    if args.len() != 3 {
        return Err(wrong_count("HINCRBY"));
    }
    let inc = parse_i64_arg(&args[2], "increment")?;
    let mut hash = if let Some(stored) = txn.get_value(storage, &args[0]).await? {
        stored.as_hash()?.clone()
    } else {
        HashMap::new()
    };
    let cur = hash
        .get(args[1].as_ref())
        .map(|v| parse_i64_arg(&Bytes::from(v.clone()), "field value"))
        .transpose()?
        .unwrap_or(0);
    let new_val = cur.saturating_add(inc);
    hash.insert(args[1].to_vec(), new_val.to_string().into_bytes());
    txn.set_hash(key(&args[0]), hash);
    Ok(router::integer(new_val))
}

async fn exec_hexists(
    storage: &dyn KvStorage,
    txn: &ScriptTransaction,
    args: &[Bytes],
) -> Result<RespValue> {
    if args.len() != 2 {
        return Err(wrong_count("HEXISTS"));
    }
    let exists = if let Some(stored) = txn.get_value(storage, &args[0]).await? {
        stored.as_hash()?.contains_key(args[1].as_ref())
    } else {
        false
    };
    Ok(router::integer(i64::from(exists)))
}

async fn exec_hlen(
    storage: &dyn KvStorage,
    txn: &ScriptTransaction,
    args: &[Bytes],
) -> Result<RespValue> {
    if args.len() != 1 {
        return Err(wrong_count("HLEN"));
    }
    let len = if let Some(stored) = txn.get_value(storage, &args[0]).await? {
        stored.as_hash()?.len()
    } else {
        0
    };
    Ok(router::integer(len as i64))
}

async fn exec_lpush(
    storage: &dyn KvStorage,
    txn: &mut ScriptTransaction,
    args: &[Bytes],
    front: bool,
) -> Result<RespValue> {
    if args.len() < 2 {
        return Err(wrong_count("LPUSH"));
    }
    let mut list = if let Some(stored) = txn.get_value(storage, &args[0]).await? {
        stored.as_list()?.clone()
    } else {
        VecDeque::new()
    };
    if front {
        for i in (1..args.len()).rev() {
            list.push_front(args[i].to_vec());
        }
    } else {
        for arg in &args[1..] {
            list.push_back(arg.to_vec());
        }
    }
    let len = list.len() as i64;
    txn.set_list(key(&args[0]), list);
    Ok(router::integer(len))
}

async fn exec_lpop(
    storage: &dyn KvStorage,
    txn: &mut ScriptTransaction,
    args: &[Bytes],
    from_front: bool,
) -> Result<RespValue> {
    if args.is_empty() {
        return Err(wrong_count("LPOP"));
    }
    let Some(stored) = txn.get_value(storage, &args[0]).await? else {
        return Ok(RespValue::Null);
    };
    let mut list = stored.as_list()?.clone();
    let val = if from_front {
        list.pop_front()
    } else {
        list.pop_back()
    };
    let Some(v) = val else {
        return Ok(RespValue::Null);
    };
    if list.is_empty() {
        txn.delete(key(&args[0]));
    } else {
        txn.set_list(key(&args[0]), list);
    }
    Ok(router::bulk(v))
}

async fn exec_llen(
    storage: &dyn KvStorage,
    txn: &ScriptTransaction,
    args: &[Bytes],
) -> Result<RespValue> {
    if args.len() != 1 {
        return Err(wrong_count("LLEN"));
    }
    let len = if let Some(stored) = txn.get_value(storage, &args[0]).await? {
        stored.as_list()?.len()
    } else {
        0
    };
    Ok(router::integer(len as i64))
}

async fn exec_lrange(
    storage: &dyn KvStorage,
    txn: &ScriptTransaction,
    args: &[Bytes],
) -> Result<RespValue> {
    if args.len() != 3 {
        return Err(wrong_count("LRANGE"));
    }
    let start = parse_i64_arg(&args[1], "start")?;
    let stop = parse_i64_arg(&args[2], "stop")?;
    let Some(stored) = txn.get_value(storage, &args[0]).await? else {
        return Ok(RespValue::Array(Some(Vec::new())));
    };
    let list = stored.as_list()?;
    let len = list.len() as i64;
    let start_idx = (if start < 0 { len + start } else { start }).clamp(0, len) as usize;
    let stop_idx = (if stop < 0 { len + stop + 1 } else { stop + 1 }).clamp(0, len) as usize;
    if start_idx >= stop_idx {
        return Ok(RespValue::Array(Some(Vec::new())));
    }
    let out: Vec<RespValue> = list
        .iter()
        .skip(start_idx)
        .take(stop_idx - start_idx)
        .map(|v| router::bulk(v.clone()))
        .collect();
    Ok(RespValue::Array(Some(out)))
}

async fn exec_lindex(
    storage: &dyn KvStorage,
    txn: &ScriptTransaction,
    args: &[Bytes],
) -> Result<RespValue> {
    if args.len() != 2 {
        return Err(wrong_count("LINDEX"));
    }
    let idx = parse_i64_arg(&args[1], "index")?;
    let Some(stored) = txn.get_value(storage, &args[0]).await? else {
        return Ok(RespValue::Null);
    };
    let list = stored.as_list()?;
    let len = list.len() as i64;
    let actual = if idx < 0 { len + idx } else { idx };
    if actual < 0 || actual >= len {
        return Ok(RespValue::Null);
    }
    Ok(router::bulk(list[actual as usize].clone()))
}

async fn exec_sadd(
    storage: &dyn KvStorage,
    txn: &mut ScriptTransaction,
    args: &[Bytes],
) -> Result<RespValue> {
    if args.len() < 2 {
        return Err(wrong_count("SADD"));
    }
    let mut set = if let Some(stored) = txn.get_value(storage, &args[0]).await? {
        stored.as_set()?.clone()
    } else {
        HashSet::new()
    };
    let mut added = 0i64;
    for m in &args[1..] {
        if set.insert(m.to_vec()) {
            added += 1;
        }
    }
    txn.set_set(key(&args[0]), set);
    Ok(router::integer(added))
}

async fn exec_srem(
    storage: &dyn KvStorage,
    txn: &mut ScriptTransaction,
    args: &[Bytes],
) -> Result<RespValue> {
    if args.len() < 2 {
        return Err(wrong_count("SREM"));
    }
    let Some(stored) = txn.get_value(storage, &args[0]).await? else {
        return Ok(router::integer(0));
    };
    let mut set = stored.as_set()?.clone();
    let mut count = 0i64;
    for m in &args[1..] {
        if set.remove(&m.to_vec()) {
            count += 1;
        }
    }
    if set.is_empty() {
        txn.delete(key(&args[0]));
    } else {
        txn.set_set(key(&args[0]), set);
    }
    Ok(router::integer(count))
}

async fn exec_smembers(
    storage: &dyn KvStorage,
    txn: &ScriptTransaction,
    args: &[Bytes],
) -> Result<RespValue> {
    if args.len() != 1 {
        return Err(wrong_count("SMEMBERS"));
    }
    let out = if let Some(stored) = txn.get_value(storage, &args[0]).await? {
        stored
            .as_set()?
            .iter()
            .map(|m| router::bulk(m.clone()))
            .collect()
    } else {
        Vec::new()
    };
    Ok(RespValue::Array(Some(out)))
}

async fn exec_sismember(
    storage: &dyn KvStorage,
    txn: &ScriptTransaction,
    args: &[Bytes],
) -> Result<RespValue> {
    if args.len() != 2 {
        return Err(wrong_count("SISMEMBER"));
    }
    let member = args[1].to_vec();
    let ok = if let Some(stored) = txn.get_value(storage, &args[0]).await? {
        stored.as_set()?.contains(&member)
    } else {
        false
    };
    Ok(router::integer(i64::from(ok)))
}

async fn exec_scard(
    storage: &dyn KvStorage,
    txn: &ScriptTransaction,
    args: &[Bytes],
) -> Result<RespValue> {
    if args.len() != 1 {
        return Err(wrong_count("SCARD"));
    }
    let n = if let Some(stored) = txn.get_value(storage, &args[0]).await? {
        stored.as_set()?.len()
    } else {
        0
    };
    Ok(router::integer(n as i64))
}

async fn exec_sscan(
    storage: &dyn KvStorage,
    txn: &ScriptTransaction,
    args: &[Bytes],
) -> Result<RespValue> {
    if args.len() < 2 {
        return Err(wrong_count("SSCAN"));
    }
    let cursor: u64 = std::str::from_utf8(&args[1])
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let members = if let Some(stored) = txn.get_value(storage, &args[0]).await? {
        stored
            .as_set()?
            .iter()
            .map(|m| router::bulk(m.clone()))
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let (new_cursor, items) = if cursor == 0 {
        (1, members)
    } else {
        (0, Vec::new())
    };
    Ok(RespValue::Array(Some(vec![
        router::bulk(new_cursor.to_string().into_bytes()),
        RespValue::Array(Some(items)),
    ])))
}

async fn exec_zadd(
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

async fn exec_zrem(
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

async fn exec_zscore(
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

async fn exec_zrank(
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

async fn exec_zrange(
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

async fn exec_zcard(
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

async fn exec_expire(
    storage: &dyn KvStorage,
    txn: &mut ScriptTransaction,
    args: &[Bytes],
) -> Result<RespValue> {
    if args.len() != 2 {
        return Err(wrong_count("EXPIRE"));
    }
    let seconds = parse_i64_arg(&args[1], "seconds")?;
    if !txn.exists(storage, &args[0]).await? {
        return Ok(router::integer(0));
    }
    if seconds <= 0 {
        txn.delete(key(&args[0]));
    } else {
        txn.set_expire_at(key(&args[0]), expire_seconds_to_at(seconds));
    }
    Ok(router::integer(1))
}
