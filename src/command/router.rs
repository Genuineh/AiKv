//! 命令路由与 key 级锁

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use tokio::sync::Mutex;
use tracing::{instrument, Instrument};

use crate::command::{
    database, hash, json, key, list, persistence, script, server, set, string, zset,
};
use crate::error::{Error, Result};
use crate::protocol::{ProtocolVersion, RespValue};
use crate::server::{ServerMetrics, ServerSharedState};
use crate::storage::KvStorage;

/// 分桶 key 写锁 (Hash / SET NX/XX / INCR 等)
pub struct KeyLock {
    locks: Vec<Mutex<()>>,
}

impl KeyLock {
    pub fn new(buckets: usize) -> Self {
        let buckets = buckets.max(1);
        Self {
            locks: (0..buckets).map(|_| Mutex::new(())).collect(),
        }
    }

    pub async fn lock(&self, key: &[u8]) -> tokio::sync::MutexGuard<'_, ()> {
        let idx = hash_key(key) % self.locks.len();
        self.locks[idx].lock().await
    }

    /// 按 key 字节序加锁; 同一 key 只锁一次 (避免 Mutex 重入死锁)
    pub async fn lock_two(
        &self,
        a: &[u8],
        b: &[u8],
    ) -> (
        tokio::sync::MutexGuard<'_, ()>,
        Option<tokio::sync::MutexGuard<'_, ()>>,
    ) {
        if a == b {
            return (self.lock(a).await, None);
        }
        if a < b {
            let ga = self.lock(a).await;
            let gb = self.lock(b).await;
            (ga, Some(gb))
        } else {
            let gb = self.lock(b).await;
            let ga = self.lock(a).await;
            (ga, Some(gb))
        }
    }

    /// 多 key 字典序加锁 (去重); Drop 时逆序释放
    pub async fn lock_keys_sorted<'a>(&'a self, keys: &[&[u8]]) -> KeyLocksGuard<'a> {
        let mut unique: Vec<&[u8]> = keys.to_vec();
        unique.sort();
        unique.dedup();
        let mut guards = Vec::with_capacity(unique.len());
        for k in unique {
            guards.push(self.lock(k).await);
        }
        KeyLocksGuard { locks: guards }
    }

    /// 多 key 字典序加锁, 带总超时; 超时或部分失败时已持有锁随 guard drop 释放.
    pub async fn lock_keys_sorted_with_timeout<'a>(
        &'a self,
        keys: &[&[u8]],
        timeout: Duration,
    ) -> Result<KeyLocksGuard<'a>> {
        let mut unique: Vec<&[u8]> = keys.to_vec();
        unique.sort();
        unique.dedup();
        if unique.is_empty() {
            return Ok(KeyLocksGuard { locks: Vec::new() });
        }

        let deadline = Instant::now() + timeout;
        let mut guards = Vec::with_capacity(unique.len());
        for k in unique {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(script_lock_timeout_err(timeout));
            }
            match tokio::time::timeout(remaining, self.lock(k)).await {
                Ok(guard) => guards.push(guard),
                Err(_) => return Err(script_lock_timeout_err(timeout)),
            }
        }
        Ok(KeyLocksGuard { locks: guards })
    }
}

fn script_lock_timeout_err(timeout: Duration) -> Error {
    Error::Command(format!(
        "ERR Lock acquisition timeout after {timeout:?}"
    ))
}

/// 多 key 锁 RAII guard; Vec 逆序 drop 释放锁
pub struct KeyLocksGuard<'a> {
    locks: Vec<tokio::sync::MutexGuard<'a, ()>>,
}

impl Drop for KeyLocksGuard<'_> {
    fn drop(&mut self) {
        while self.locks.pop().is_some() {}
    }
}

fn hash_key(key: &[u8]) -> usize {
    let mut h = DefaultHasher::new();
    key.hash(&mut h);
    h.finish() as usize
}

pub struct CommandRouter {
    storage: Arc<dyn KvStorage>,
    #[expect(dead_code)]
    key_lock: Arc<KeyLock>,
    metrics: Option<Arc<ServerMetrics>>,
    tcp_port: u16,
    string: string::StringCommands,
    hash: hash::HashCommands,
    list: list::ListCommands,
    set: set::SetCommands,
    zset: zset::ZSetCommands,
    json: json::JsonCommands,
    script: script::ScriptCommands,
    database: database::DatabaseCommands,
    key_cmds: key::KeyCommands,
    server: Option<server::ServerCommands>,
    persistence: Option<persistence::PersistenceCommands>,
}

impl CommandRouter {
    pub fn new(storage: Arc<dyn KvStorage>) -> Self {
        let key_lock = Arc::new(KeyLock::new(1024));
        Self {
            string: string::StringCommands::new(storage.clone(), key_lock.clone()),
            hash: hash::HashCommands::new(storage.clone(), key_lock.clone()),
            list: list::ListCommands::new(storage.clone(), key_lock.clone()),
            set: set::SetCommands::new(storage.clone(), key_lock.clone()),
            zset: zset::ZSetCommands::new(storage.clone(), key_lock.clone()),
            json: json::JsonCommands::new(storage.clone(), key_lock.clone()),
            script: script::ScriptCommands::new(storage.clone(), key_lock.clone()),
            database: database::DatabaseCommands::new(storage.clone()),
            key_cmds: key::KeyCommands::new(storage.clone(), key_lock.clone()),
            server: None,
            persistence: None,
            metrics: None,
            storage,
            key_lock,
            tcp_port: 6379,
        }
    }

    pub fn new_with_shared(storage: Arc<dyn KvStorage>, shared: Arc<ServerSharedState>) -> Self {
        let tcp_port = shared.tcp_port;
        let key_lock = Arc::new(KeyLock::new(1024));
        Self {
            string: string::StringCommands::new(storage.clone(), key_lock.clone()),
            hash: hash::HashCommands::new(storage.clone(), key_lock.clone()),
            list: list::ListCommands::with_metrics(
                storage.clone(),
                key_lock.clone(),
                shared.metrics.clone(),
            ),
            set: set::SetCommands::new(storage.clone(), key_lock.clone()),
            zset: zset::ZSetCommands::with_metrics(
                storage.clone(),
                key_lock.clone(),
                shared.metrics.clone(),
            ),
            json: json::JsonCommands::with_metrics(
                storage.clone(),
                key_lock.clone(),
                shared.metrics.clone(),
            ),
            script: script::ScriptCommands::with_metrics(
                storage.clone(),
                key_lock.clone(),
                shared.metrics.clone(),
            ),
            database: database::DatabaseCommands::new(storage.clone()),
            key_cmds: key::KeyCommands::new(storage.clone(), key_lock.clone()),
            server: Some(server::ServerCommands::new(storage.clone(), shared.clone())),
            persistence: Some(persistence::PersistenceCommands::new(
                storage.clone(),
                shared.clone(),
            )),
            metrics: Some(Arc::clone(&shared.metrics)),
            storage,
            key_lock,
            tcp_port,
        }
    }

    pub fn storage(&self) -> Arc<dyn KvStorage> {
        Arc::clone(&self.storage)
    }

    #[instrument(name = "kv_command", skip(self, args), fields(cmd.name = cmd, args_len = args.len()))]
    pub async fn execute(&self, cmd: &str, args: &[Bytes], db: &mut usize) -> Result<RespValue> {
        self.execute_with_client(
            cmd,
            args,
            db,
            None,
            None,
            ProtocolVersion::Resp2,
            #[cfg(feature = "cluster")]
            None,
        )
        .await
    }

    pub async fn execute_with_client(
        &self,
        cmd: &str,
        args: &[Bytes],
        db: &mut usize,
        client_id: Option<usize>,
        client_addr: Option<SocketAddr>,
        protocol_version: ProtocolVersion,
        #[cfg(feature = "cluster")] conn_state: Option<
            &crate::cluster::connection::ClusterConnectionState,
        >,
    ) -> Result<RespValue> {
        #[cfg(feature = "cluster")]
        if let Some(state) = conn_state {
            if let Some(result) = self.cluster_route(cmd, args, state).await {
                record_command_outcome(&self.metrics, cmd, &result);
                return result;
            }
        }
        let span = tracing::info_span!(
            "kv_command",
            otel.kind = "server",
            cmd = cmd,
            args_len = args.len(),
            client_id = client_id.unwrap_or(0),
            client.address = tracing::field::Empty,
            network.peer.address = tracing::field::Empty,
            network.peer.port = tracing::field::Empty,
            db = *db,
            server.port = self.tcp_port,
            db.system = "redis",
            db.operation.name = cmd,
            trace_id = tracing::field::Empty,
            span_id = tracing::field::Empty,
        );
        if let Some(addr) = client_addr {
            span.record("client.address", tracing::field::display(addr.ip()));
            span.record("network.peer.address", tracing::field::display(addr.ip()));
            span.record("network.peer.port", addr.port() as i64);
        }
        let result = async {
            #[cfg(feature = "monitoring")]
            crate::server::otel::record_trace_ids_on_span(&tracing::Span::current());
            self.execute_inner(cmd, args, db, client_id, protocol_version)
                .await
        }
        .instrument(span)
        .await;
        record_command_outcome(&self.metrics, cmd, &result);
        #[cfg(feature = "monitoring")]
        record_command_span_status(&result);
        result
    }

    /// Cluster routing check: returns Some(response) if the request should be
    /// redirected (MOVED/ASK) or rejected (CROSSSLOT/CLUSTERDOWN).
    #[cfg(feature = "cluster")]
    async fn cluster_route(
        &self,
        cmd: &str,
        args: &[Bytes],
        conn_state: &crate::cluster::connection::ClusterConnectionState,
    ) -> Option<Result<RespValue>> {
        let _ = crate::cluster::state::CLUSTER_STATE_MGR.get()?;
        let lower = cmd.to_ascii_lowercase();
        let admin_cmds = [
            "cluster",
            "ping",
            "echo",
            "hello",
            "quit",
            "reset",
            "info",
            "time",
            "config",
            "client",
            "shutdown",
            "readonly",
            "readwrite",
            "asking",
            "select",
            "auth",
            "save",
            "bgsave",
            "lastsave",
            "command",
            "latency",
            "slowlog",
            // MIGRATE reads locally then RESTOREs remotely; must not be MOVED/ASK redirected.
            "migrate",
            // Cursor-based iteration — these scan the local keyspace and MUST
            // NOT be routed by key (args.first() is the cursor, not a key).
            "scan",
            "hscan",
            "sscan",
            "zscan",
        ];
        if admin_cmds.contains(&lower.as_str()) {
            return None;
        }
        // Multi-key CROSSSLOT check
        if args.len() > 1 && is_multi_key_cmd(cmd) {
            // MSET args are [key, val, key, val, ...] — only even indices are keys.
            let is_mset = cmd.eq_ignore_ascii_case("mset");
            let key_bytes: Vec<&[u8]> = if is_mset {
                args.iter().step_by(2).map(|a| a.as_ref()).collect()
            } else {
                args.iter().map(|a| a.as_ref()).collect()
            };
            if let Err(msg) = crate::cluster::router::check_cross_slot(&key_bytes) {
                return Some(Ok(RespValue::Error(msg)));
            }
        }
        // Single-key routing (EVAL/EVALSHA use first declared key, not script body).
        if let Some(key) = crate::cluster::forward::cluster_routing_key(cmd, args) {
            let cmd_type = classify_command(cmd);
            let decision = crate::cluster::router::ClusterRouter::decide(
                key,
                cmd_type,
                conn_state.is_asking(),
                conn_state.is_readonly(),
            );
            return match decision {
                crate::cluster::router::RouteDecision::Execute => None,
                crate::cluster::router::RouteDecision::Moved {
                    slot,
                    node_id,
                    addr,
                    ..
                } => {
                    if let Some(m) = self.metrics.as_ref() {
                        m.on_cluster_redirect("moved");
                    }
                    let connect_addr = crate::cluster::state::CLUSTER_STATE_MGR
                        .get()
                        .and_then(|mgr| {
                            mgr.announce_resolver.tcp_connect_addr(
                                node_id,
                                &addr,
                                &mgr.meta_raft.get_cluster_meta(),
                            )
                        })
                        .unwrap_or_else(|| addr.clone());
                    match crate::cluster::forward::forward_command(&connect_addr, false, cmd, args)
                        .await
                    {
                        Ok(resp) => Some(Ok(resp)),
                        Err(_) => Some(Ok(RespValue::Error(format!("MOVED {slot} {addr}")))),
                    }
                }
                crate::cluster::router::RouteDecision::Ask {
                    slot,
                    node_id,
                    addr,
                    ..
                } => {
                    if let Some(m) = self.metrics.as_ref() {
                        m.on_cluster_redirect("ask");
                    }
                    let connect_addr = crate::cluster::state::CLUSTER_STATE_MGR
                        .get()
                        .and_then(|mgr| {
                            mgr.announce_resolver.tcp_connect_addr(
                                node_id,
                                &addr,
                                &mgr.meta_raft.get_cluster_meta(),
                            )
                        })
                        .unwrap_or_else(|| addr.clone());
                    match crate::cluster::forward::forward_command(&connect_addr, true, cmd, args)
                        .await
                    {
                        Ok(resp) => Some(Ok(resp)),
                        Err(_) => Some(Ok(RespValue::Error(format!("ASK {slot} {addr}")))),
                    }
                }
                crate::cluster::router::RouteDecision::ClusterDown(msg) => {
                    Some(Ok(RespValue::Error(msg)))
                }
            };
        }
        None
    }

    async fn execute_inner(
        &self,
        cmd: &str,
        args: &[Bytes],
        db: &mut usize,
        client_id: Option<usize>,
        protocol_version: ProtocolVersion,
    ) -> Result<RespValue> {
        match cmd {
            "GET" => {
                let result = self.string.get(*db, args).await?;
                record_keyspace_lookup(
                    &self.metrics,
                    matches!(&result, RespValue::BulkString(Some(_))),
                );
                Ok(result)
            }
            "SET" => self.string.set(*db, args).await,
            "MGET" => {
                let result = self.string.mget(*db, args).await?;
                if let RespValue::Array(Some(items)) = &result {
                    for item in items {
                        record_keyspace_lookup(
                            &self.metrics,
                            matches!(item, RespValue::BulkString(Some(_))),
                        );
                    }
                }
                Ok(result)
            }
            "MSET" => self.string.mset(*db, args).await,
            "DEL" => self.string.del(*db, args).await,
            "EXISTS" => {
                if self.metrics.is_some() {
                    for key in args {
                        let hit = self.storage.exists(*db, key).await?;
                        record_keyspace_lookup(&self.metrics, hit);
                    }
                }
                self.string.exists(*db, args).await
            }
            "STRLEN" => self.string.strlen(*db, args).await,
            "GETRANGE" => self.string.getrange(*db, args).await,
            "SETRANGE" => self.string.setrange(*db, args).await,
            "SETBIT" => self.string.setbit(*db, args).await,
            "GETBIT" => self.string.getbit(*db, args).await,
            "APPEND" => self.string.append(*db, args).await,
            "INCR" => self.string.incr(*db, args).await,
            "DECR" => self.string.decr(*db, args).await,
            "INCRBY" => self.string.incrby(*db, args).await,
            "DECRBY" => self.string.decrby(*db, args).await,
            "INCRBYFLOAT" => self.string.incrbyfloat(*db, args).await,
            "GETDEL" => self.string.getdel(*db, args).await,
            "GETEX" => self.string.getex(*db, args).await,
            "SETNX" => self.string.setnx(*db, args).await,
            "SETEX" => self.string.setex(*db, args).await,
            "PSETEX" => self.string.psetex(*db, args).await,
            "HSET" => self.hash.hset(*db, args).await,
            "HMSET" => self.hash.hmset(*db, args).await,
            "HGET" => {
                let result = self.hash.hget(*db, args).await?;
                record_keyspace_lookup(
                    &self.metrics,
                    matches!(&result, RespValue::BulkString(Some(_))),
                );
                Ok(result)
            }
            "HDEL" => self.hash.hdel(*db, args).await,
            "HEXISTS" => self.hash.hexists(*db, args).await,
            "HLEN" => self.hash.hlen(*db, args).await,
            "HKEYS" => self.hash.hkeys(*db, args).await,
            "HVALS" => self.hash.hvals(*db, args).await,
            "HGETALL" => self.hash.hgetall(*db, args).await,
            "HMGET" => self.hash.hmget(*db, args).await,
            "HSETNX" => self.hash.hsetnx(*db, args).await,
            "HINCRBY" => self.hash.hincrby(*db, args).await,
            "HINCRBYFLOAT" => self.hash.hincrbyfloat(*db, args).await,
            "HSCAN" => self.hash.hscan(*db, args).await,
            "LPUSH" => self.list.lpush(*db, args).await,
            "RPUSH" => self.list.rpush(*db, args).await,
            "LPOP" => self.list.lpop(*db, args).await,
            "RPOP" => self.list.rpop(*db, args).await,
            "LLEN" => self.list.llen(*db, args).await,
            "LRANGE" => self.list.lrange(*db, args).await,
            "LINDEX" => self.list.lindex(*db, args).await,
            "LSET" => self.list.lset(*db, args).await,
            "LREM" => self.list.lrem(*db, args).await,
            "LTRIM" => self.list.ltrim(*db, args).await,
            "LINSERT" => self.list.linsert(*db, args).await,
            "LMOVE" => self.list.lmove(*db, args).await,
            "LPOS" => self.list.lpos(*db, args).await,
            "BLPOP" => self.list.blpop(*db, args).await,
            "BRPOP" => self.list.brpop(*db, args).await,
            "BLMOVE" => self.list.blmove_blocking(*db, args).await,
            "SADD" => self.set.sadd(*db, args).await,
            "SREM" => self.set.srem(*db, args).await,
            "SISMEMBER" => self.set.sismember(*db, args).await,
            "SMEMBERS" => self.set.smembers(*db, args).await,
            "SCARD" => self.set.scard(*db, args).await,
            "SPOP" => self.set.spop(*db, args).await,
            "SRANDMEMBER" => self.set.srandmember(*db, args).await,
            "SUNION" => self.set.sunion(*db, args).await,
            "SINTER" => self.set.sinter(*db, args).await,
            "SDIFF" => self.set.sdiff(*db, args).await,
            "SUNIONSTORE" => self.set.sunionstore(*db, args).await,
            "SINTERSTORE" => self.set.sinterstore(*db, args).await,
            "SDIFFSTORE" => self.set.sdiffstore(*db, args).await,
            "SMOVE" => self.set.smove(*db, args).await,
            "SSCAN" => self.set.sscan(*db, args).await,
            "ZADD" => self.zset.zadd(*db, args).await,
            "ZREM" => self.zset.zrem(*db, args).await,
            "ZSCORE" => self.zset.zscore(*db, args).await,
            "ZRANK" => self.zset.zrank(*db, args).await,
            "ZREVRANK" => self.zset.zrevrank(*db, args).await,
            "ZRANGE" => self.zset.zrange(*db, args).await,
            "ZREVRANGE" => self.zset.zrevrange(*db, args).await,
            "ZRANGEBYSCORE" => self.zset.zrangebyscore(*db, args).await,
            "ZREVRANGEBYSCORE" => self.zset.zrevrangebyscore(*db, args).await,
            "ZCARD" => self.zset.zcard(*db, args).await,
            "ZCOUNT" => self.zset.zcount(*db, args).await,
            "ZINCRBY" => self.zset.zincrby(*db, args).await,
            "ZSCAN" => self.zset.zscan(*db, args).await,
            "ZPOPMIN" => self.zset.zpopmin(*db, args).await,
            "ZPOPMAX" => self.zset.zpopmax(*db, args).await,
            "BZPOPMIN" => self.zset.bzpopmin(*db, args).await,
            "BZPOPMAX" => self.zset.bzpopmax(*db, args).await,
            "ZRANGEBYLEX" => self.zset.zrangebylex(*db, args).await,
            "ZREVRANGEBYLEX" => self.zset.zrevrangebylex(*db, args).await,
            "ZLEXCOUNT" => self.zset.zlexcount(*db, args).await,
            "ZINTER" => self.zset.zinter(*db, args).await,
            "ZUNION" => self.zset.zunion(*db, args).await,
            "ZDIFF" => self.zset.zdiff(*db, args).await,
            "SELECT" => self.database.select(args, db).await,
            "DBSIZE" => self.database.dbsize(*db, args).await,
            "FLUSHDB" => self.database.flushdb(*db, args).await,
            "FLUSHALL" => self.database.flushall(args).await,
            "SWAPDB" => self.database.swapdb(args).await,
            "MOVE" => self.database.move_key(*db, args).await,
            "EXPIRE" => self.key_cmds.expire(*db, args).await,
            "EXPIREAT" => self.key_cmds.expireat(*db, args).await,
            "PEXPIRE" => self.key_cmds.pexpire(*db, args).await,
            "PEXPIREAT" => self.key_cmds.pexpireat(*db, args).await,
            "TTL" => self.key_cmds.ttl(*db, args).await,
            "PTTL" => self.key_cmds.pttl(*db, args).await,
            "PERSIST" => self.key_cmds.persist(*db, args).await,
            "KEYS" => self.key_cmds.keys(*db, args).await,
            "SCAN" => self.key_cmds.scan(*db, args).await,
            "RANDOMKEY" => self.key_cmds.randomkey(*db, args).await,
            "RENAME" => self.key_cmds.rename(*db, args).await,
            "RENAMENX" => self.key_cmds.renamenx(*db, args).await,
            "TYPE" => self.key_cmds.type_cmd(*db, args).await,
            "COPY" => self.key_cmds.copy(*db, args).await,
            "OBJECT" => self.dispatch_object(*db, args).await,
            "EXPIRETIME" => self.key_cmds.expiretime(*db, args).await,
            "PEXPIRETIME" => self.key_cmds.pexpiretime(*db, args).await,
            "DUMP" => self.key_cmds.dump(*db, args).await,
            "RESTORE" => self.key_cmds.restore(*db, args).await,
            "MIGRATE" => self.key_cmds.migrate(*db, args).await,
            "JSON.SET" => self.json.json_set(*db, args).await,
            "JSON.GET" => self.json.json_get(*db, args).await,
            "JSON.MGET" => self.json.json_mget(*db, args).await,
            "JSON.DEL" => self.json.json_del(*db, args).await,
            "JSON.TYPE" => self.json.json_type(*db, args).await,
            "JSON.STRLEN" => self.json.json_strlen(*db, args).await,
            "JSON.ARRLEN" => self.json.json_arrlen(*db, args).await,
            "JSON.OBJLEN" => self.json.json_objlen(*db, args).await,
            "JSON.NUMINCRBY" => self.json.json_numincrby(*db, args).await,
            "JSON.ARRAPPEND" => self.json.json_arrappend(*db, args).await,
            "JSON.UPDATE" => self.json.json_update(*db, args).await,
            "JSON.MSET" => self.json.json_mset(*db, args).await,
            "EVAL" => self.script.eval(*db, args).await,
            "EVALSHA" => self.script.evalsha(*db, args).await,
            "SCRIPT" => self.script.script(args).await,
            "INFO" => self.require_server()?.info(*db, args).await,
            "TIME" => self.require_server()?.time(args).await,
            "CONFIG" => self.dispatch_config(args).await,
            "CLIENT" => self.dispatch_client(client_id, args).await,
            "LATENCY" => self.require_server()?.latency(args, protocol_version).await,
            "SLOWLOG" => self.require_server()?.slowlog(args).await,
            "COMMAND" => self.require_server()?.command(args).await,
            #[cfg(feature = "cluster")]
            "CLUSTER" => {
                let sub = args.first().map(|a| std::str::from_utf8(a).unwrap_or(""));
                crate::cluster::dispatch_cluster(sub, args).await
            }
            "SAVE" => self.require_persistence()?.save().await,
            "BGSAVE" => self.require_persistence()?.bgsave().await,
            "LASTSAVE" => self.require_persistence()?.lastsave().await,
            "SHUTDOWN" => self.require_persistence()?.shutdown(args).await,
            _ => Err(Error::Command(format!("ERR unknown command '{cmd}'"))),
        }
    }

    fn require_server(&self) -> Result<&server::ServerCommands> {
        self.server
            .as_ref()
            .ok_or_else(|| Error::Command("ERR server commands unavailable".into()))
    }

    fn require_persistence(&self) -> Result<&persistence::PersistenceCommands> {
        self.persistence
            .as_ref()
            .ok_or_else(|| Error::Command("ERR persistence commands unavailable".into()))
    }

    async fn dispatch_object(&self, db: usize, args: &[Bytes]) -> Result<RespValue> {
        if args.is_empty() {
            return Err(wrong_args("OBJECT", ""));
        }
        let sub = String::from_utf8_lossy(&args[0]).to_ascii_uppercase();
        match sub.as_str() {
            "ENCODING" => {
                if args.len() < 2 {
                    return Err(wrong_args("OBJECT ENCODING", ""));
                }
                let key = &args[1];
                match self.key_cmds.storage().get_typed(db, key).await? {
                    None => Err(Error::Command("ERR no such key".into())),
                    Some(stored) => {
                        use crate::storage::ValueType;
                        let enc = match &stored.value {
                            ValueType::String(v) => {
                                if v.len() <= 44 {
                                    "embstr"
                                } else {
                                    "raw"
                                }
                            }
                            ValueType::Hash(_) => "listpack",
                            ValueType::List(_) => "listpack",
                            ValueType::Set(_) => "listpack",
                            ValueType::ZSet(_) => "listpack",
                        };
                        Ok(bulk(enc.as_bytes().to_vec()))
                    }
                }
            }
            "REFCOUNT" => {
                if args.len() < 2 {
                    return Err(wrong_args("OBJECT REFCOUNT", ""));
                }
                let exists = self.key_cmds.storage().exists(db, &args[1]).await?;
                if exists {
                    Ok(integer(1))
                } else {
                    Err(Error::Command("ERR no such key".into()))
                }
            }
            "IDLETIME" => {
                if args.len() < 2 {
                    return Err(wrong_args("OBJECT IDLETIME", ""));
                }
                let exists = self.key_cmds.storage().exists(db, &args[1]).await?;
                if exists {
                    Ok(integer(0))
                } else {
                    Err(Error::Command("ERR no such key".into()))
                }
            }
            "FREQ" => {
                if args.len() < 2 {
                    return Err(wrong_args("OBJECT FREQ", ""));
                }
                let exists = self.key_cmds.storage().exists(db, &args[1]).await?;
                if exists {
                    Ok(integer(0))
                } else {
                    Err(Error::Command("ERR no such key".into()))
                }
            }
            "HELP" => Ok(RespValue::Array(Some(vec![
                bulk(b"OBJECT ENCODING key - Return encoding of the object".to_vec()),
                bulk(b"OBJECT REFCOUNT key - Return reference count (stub: 1)".to_vec()),
                bulk(b"OBJECT IDLETIME key - Return idle time in seconds (stub: 0)".to_vec()),
                bulk(b"OBJECT FREQ key - Return access frequency (stub: 0)".to_vec()),
            ]))),
            _ => Err(Error::Command(format!(
                "ERR unknown OBJECT subcommand '{sub}'"
            ))),
        }
    }

    async fn dispatch_config(&self, args: &[Bytes]) -> Result<RespValue> {
        if args.is_empty() {
            return Err(wrong_args("CONFIG", ""));
        }
        let sub = String::from_utf8_lossy(&args[0]).to_ascii_uppercase();
        match sub.as_str() {
            "GET" => self.require_server()?.config_get(args).await,
            "SET" => self.require_server()?.config_set(args).await,
            "REWRITE" => Ok(ok()),
            "RESETSTAT" => Ok(ok()),
            _ => Err(Error::Command("ERR unknown CONFIG subcommand".into())),
        }
    }

    async fn dispatch_client(&self, client_id: Option<usize>, args: &[Bytes]) -> Result<RespValue> {
        if args.is_empty() {
            return Err(wrong_args("CLIENT", ""));
        }
        let sub = String::from_utf8_lossy(&args[0]).to_ascii_uppercase();
        match sub.as_str() {
            "LIST" => self.require_server()?.client_list(args).await,
            "SETNAME" => {
                let id = client_id.ok_or_else(|| {
                    Error::Command("ERR CLIENT SETNAME requires an active connection".into())
                })?;
                self.require_server()?.client_setname(id, args).await
            }
            "GETNAME" => {
                let id = client_id.ok_or_else(|| {
                    Error::Command("ERR CLIENT GETNAME requires an active connection".into())
                })?;
                self.require_server()?.client_getname(id, args).await
            }
            _ => Err(Error::Command("ERR unknown CLIENT subcommand".into())),
        }
    }
}

pub(crate) fn wrong_args(cmd: &str, expected: &str) -> Error {
    Error::Command(format!(
        "ERR wrong number of arguments for '{cmd}' command{expected}"
    ))
}

pub(crate) fn require_args(cmd: &str, args: &[Bytes], n: usize) -> Result<()> {
    if args.len() != n {
        return Err(wrong_args(cmd, ""));
    }
    Ok(())
}

pub(crate) fn require_min_args(cmd: &str, args: &[Bytes], n: usize) -> Result<()> {
    if args.len() < n {
        return Err(wrong_args(cmd, ""));
    }
    Ok(())
}

pub(crate) fn ok() -> RespValue {
    RespValue::SimpleString("OK".into())
}

pub(crate) fn nil_bulk() -> RespValue {
    RespValue::BulkString(None)
}

pub(crate) fn bulk(b: Vec<u8>) -> RespValue {
    RespValue::BulkString(Some(Bytes::from(b)))
}

pub(crate) fn integer(n: i64) -> RespValue {
    RespValue::Integer(n)
}

pub(crate) fn wrongtype() -> Error {
    Error::Command(crate::storage::types::WRONGTYPE.into())
}

fn record_keyspace_lookup(metrics: &Option<Arc<ServerMetrics>>, hit: bool) {
    let Some(metrics) = metrics else {
        return;
    };
    if hit {
        metrics.on_keyspace_hit();
    } else {
        metrics.on_keyspace_miss();
    }
}

fn record_command_outcome(
    metrics: &Option<Arc<ServerMetrics>>,
    cmd: &str,
    result: &Result<RespValue>,
) {
    let Some(metrics) = metrics else {
        return;
    };
    metrics.on_command(cmd, result.is_ok());
}

#[cfg(feature = "monitoring")]
fn record_command_span_status(result: &Result<RespValue>) {
    if result.is_ok() {
        return;
    }
    let span = tracing::Span::current();
    span.record("otel.status_code", "ERROR");
    if let Some(err) = result.as_ref().err() {
        span.record("otel.status_message", tracing::field::display(err));
        tracing::event!(
            parent: &span,
            tracing::Level::ERROR,
            exception.type = std::any::type_name::<Error>(),
            exception.message = %err,
            "command failed"
        );
    }
}

#[cfg(feature = "cluster")]
fn is_multi_key_cmd(cmd: &str) -> bool {
    matches!(
        cmd.to_ascii_lowercase().as_str(),
        "mget"
            | "mset"
            | "del"
            | "exists"
            | "unlink"
            | "touch"
            | "mexecute"
            | "rpop"
            | "blpop"
            | "brpop"
    )
}

#[cfg(feature = "cluster")]
fn classify_command(cmd: &str) -> crate::cluster::router::CommandType {
    match cmd.to_ascii_lowercase().as_str() {
        "get" | "exists" | "hget" | "hgetall" | "hkeys" | "hvals" | "hlen" | "hexists"
        | "lrange" | "lindex" | "llen" | "smembers" | "scard" | "sismember" | "zrange"
        | "zcard" | "zscore" | "zrank" | "type" | "ttl" | "pttl" | "strlen" | "getbit"
        | "getrange" | "mget" | "json.get" | "json.mget" => crate::cluster::router::CommandType::Read,
        _ => crate::cluster::router::CommandType::Write,
    }
}
