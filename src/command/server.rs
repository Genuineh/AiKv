use crate::error::{AikvError, Result};
use crate::observability::{LogConfig, MemoryMetrics, Metrics, SlowQueryLog};
use crate::protocol::RespValue;
use crate::storage::StorageEngine;
use bytes::Bytes;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::Level;

/// AiKv version - the actual version of this server
const AIKV_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Redis-compatible version to report for client compatibility
/// We report a modern Redis version to ensure clients like StackExchange.Redis work correctly
const REDIS_COMPAT_VERSION: &str = "7.2.4";

/// Client info structure
#[derive(Clone, Debug)]
pub struct ClientInfo {
    pub id: usize,
    pub name: Option<String>,
    pub addr: String,
}

/// Command information structure for COMMAND command
#[derive(Clone, Debug)]
pub struct CommandInfo {
    /// Command name
    pub name: &'static str,
    /// Number of arguments (negative means variable)
    pub arity: i64,
    /// Command flags
    pub flags: &'static [&'static str],
    /// First key position
    pub first_key: i64,
    /// Last key position
    pub last_key: i64,
    /// Key step
    pub step: i64,
}

/// Server command handler
pub struct ServerCommands {
    storage: StorageEngine,
    clients: Arc<RwLock<HashMap<usize, ClientInfo>>>,
    config: Arc<RwLock<HashMap<String, String>>>,
    /// Server start time (Unix epoch seconds) for uptime calculation
    start_time_secs: u64,
    run_id: String,
    tcp_port: u16,
    current_log_level: Arc<RwLock<Level>>,
    slow_query_log: Arc<SlowQueryLog>,
    /// Last save timestamp (Unix epoch in seconds)
    last_save_time: Arc<AtomicU64>,
    /// Shutdown flag
    shutdown_requested: Arc<AtomicBool>,
    /// Whether cluster mode is enabled
    cluster_enabled: bool,
    /// Server metrics for INFO command
    metrics: Arc<Metrics>,
}

/// All supported commands with their metadata
fn get_command_table() -> Vec<CommandInfo> {
    vec![
        // String commands
        CommandInfo {
            name: "GET",
            arity: 2,
            flags: &["readonly", "fast"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "SET",
            arity: -3,
            flags: &["write", "denyoom"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "DEL",
            arity: -2,
            flags: &["write"],
            first_key: 1,
            last_key: -1,
            step: 1,
        },
        CommandInfo {
            name: "EXISTS",
            arity: -2,
            flags: &["readonly", "fast"],
            first_key: 1,
            last_key: -1,
            step: 1,
        },
        CommandInfo {
            name: "MGET",
            arity: -2,
            flags: &["readonly", "fast"],
            first_key: 1,
            last_key: -1,
            step: 1,
        },
        CommandInfo {
            name: "MSET",
            arity: -3,
            flags: &["write", "denyoom"],
            first_key: 1,
            last_key: -1,
            step: 2,
        },
        CommandInfo {
            name: "STRLEN",
            arity: 2,
            flags: &["readonly", "fast"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "APPEND",
            arity: 3,
            flags: &["write", "denyoom", "fast"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        // JSON commands
        CommandInfo {
            name: "JSON.GET",
            arity: -2,
            flags: &["readonly"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "JSON.SET",
            arity: -4,
            flags: &["write", "denyoom"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "JSON.DEL",
            arity: -2,
            flags: &["write"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "JSON.TYPE",
            arity: -2,
            flags: &["readonly"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "JSON.STRLEN",
            arity: -2,
            flags: &["readonly"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "JSON.ARRLEN",
            arity: -2,
            flags: &["readonly"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "JSON.OBJLEN",
            arity: -2,
            flags: &["readonly"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        // List commands
        CommandInfo {
            name: "LPUSH",
            arity: -3,
            flags: &["write", "denyoom", "fast"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "RPUSH",
            arity: -3,
            flags: &["write", "denyoom", "fast"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "LPOP",
            arity: -2,
            flags: &["write", "fast"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "RPOP",
            arity: -2,
            flags: &["write", "fast"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "LLEN",
            arity: 2,
            flags: &["readonly", "fast"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "LRANGE",
            arity: 4,
            flags: &["readonly"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "LINDEX",
            arity: 3,
            flags: &["readonly"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "LSET",
            arity: 4,
            flags: &["write", "denyoom"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "LREM",
            arity: 4,
            flags: &["write"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "LTRIM",
            arity: 4,
            flags: &["write"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "LINSERT",
            arity: 5,
            flags: &["write", "denyoom"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "LMOVE",
            arity: 5,
            flags: &["write", "denyoom"],
            first_key: 1,
            last_key: 2,
            step: 1,
        },
        // Hash commands
        CommandInfo {
            name: "HSET",
            arity: -4,
            flags: &["write", "denyoom", "fast"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "HSETNX",
            arity: 4,
            flags: &["write", "denyoom", "fast"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "HGET",
            arity: 3,
            flags: &["readonly", "fast"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "HMGET",
            arity: -3,
            flags: &["readonly", "fast"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "HMSET",
            arity: -4,
            flags: &["write", "denyoom", "fast"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "HDEL",
            arity: -3,
            flags: &["write", "fast"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "HEXISTS",
            arity: 3,
            flags: &["readonly", "fast"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "HLEN",
            arity: 2,
            flags: &["readonly", "fast"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "HKEYS",
            arity: 2,
            flags: &["readonly"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "HVALS",
            arity: 2,
            flags: &["readonly"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "HGETALL",
            arity: 2,
            flags: &["readonly"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "HINCRBY",
            arity: 4,
            flags: &["write", "denyoom", "fast"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "HINCRBYFLOAT",
            arity: 4,
            flags: &["write", "denyoom", "fast"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "HSCAN",
            arity: -3,
            flags: &["readonly"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        // Set commands
        CommandInfo {
            name: "SADD",
            arity: -3,
            flags: &["write", "denyoom", "fast"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "SREM",
            arity: -3,
            flags: &["write", "fast"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "SISMEMBER",
            arity: 3,
            flags: &["readonly", "fast"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "SMEMBERS",
            arity: 2,
            flags: &["readonly"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "SCARD",
            arity: 2,
            flags: &["readonly", "fast"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "SPOP",
            arity: -2,
            flags: &["write", "fast"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "SRANDMEMBER",
            arity: -2,
            flags: &["readonly"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "SUNION",
            arity: -2,
            flags: &["readonly"],
            first_key: 1,
            last_key: -1,
            step: 1,
        },
        CommandInfo {
            name: "SINTER",
            arity: -2,
            flags: &["readonly"],
            first_key: 1,
            last_key: -1,
            step: 1,
        },
        CommandInfo {
            name: "SDIFF",
            arity: -2,
            flags: &["readonly"],
            first_key: 1,
            last_key: -1,
            step: 1,
        },
        CommandInfo {
            name: "SUNIONSTORE",
            arity: -3,
            flags: &["write", "denyoom"],
            first_key: 1,
            last_key: -1,
            step: 1,
        },
        CommandInfo {
            name: "SINTERSTORE",
            arity: -3,
            flags: &["write", "denyoom"],
            first_key: 1,
            last_key: -1,
            step: 1,
        },
        CommandInfo {
            name: "SDIFFSTORE",
            arity: -3,
            flags: &["write", "denyoom"],
            first_key: 1,
            last_key: -1,
            step: 1,
        },
        // Sorted Set commands
        CommandInfo {
            name: "ZADD",
            arity: -4,
            flags: &["write", "denyoom", "fast"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "ZREM",
            arity: -3,
            flags: &["write", "fast"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "ZSCORE",
            arity: 3,
            flags: &["readonly", "fast"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "ZRANK",
            arity: 3,
            flags: &["readonly", "fast"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "ZREVRANK",
            arity: 3,
            flags: &["readonly", "fast"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "ZRANGE",
            arity: -4,
            flags: &["readonly"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "ZREVRANGE",
            arity: -4,
            flags: &["readonly"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "ZRANGEBYSCORE",
            arity: -4,
            flags: &["readonly"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "ZREVRANGEBYSCORE",
            arity: -4,
            flags: &["readonly"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "ZCARD",
            arity: 2,
            flags: &["readonly", "fast"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "ZCOUNT",
            arity: 4,
            flags: &["readonly", "fast"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "ZINCRBY",
            arity: 4,
            flags: &["write", "denyoom", "fast"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        // Database commands
        CommandInfo {
            name: "SELECT",
            arity: 2,
            flags: &["fast"],
            first_key: 0,
            last_key: 0,
            step: 0,
        },
        CommandInfo {
            name: "DBSIZE",
            arity: 1,
            flags: &["readonly", "fast"],
            first_key: 0,
            last_key: 0,
            step: 0,
        },
        CommandInfo {
            name: "FLUSHDB",
            arity: -1,
            flags: &["write"],
            first_key: 0,
            last_key: 0,
            step: 0,
        },
        CommandInfo {
            name: "FLUSHALL",
            arity: -1,
            flags: &["write"],
            first_key: 0,
            last_key: 0,
            step: 0,
        },
        CommandInfo {
            name: "SWAPDB",
            arity: 3,
            flags: &["write", "fast"],
            first_key: 0,
            last_key: 0,
            step: 0,
        },
        CommandInfo {
            name: "MOVE",
            arity: 3,
            flags: &["write", "fast"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        // Key commands
        CommandInfo {
            name: "KEYS",
            arity: 2,
            flags: &["readonly"],
            first_key: 0,
            last_key: 0,
            step: 0,
        },
        CommandInfo {
            name: "SCAN",
            arity: -2,
            flags: &["readonly"],
            first_key: 0,
            last_key: 0,
            step: 0,
        },
        CommandInfo {
            name: "RANDOMKEY",
            arity: 1,
            flags: &["readonly"],
            first_key: 0,
            last_key: 0,
            step: 0,
        },
        CommandInfo {
            name: "RENAME",
            arity: 3,
            flags: &["write"],
            first_key: 1,
            last_key: 2,
            step: 1,
        },
        CommandInfo {
            name: "RENAMENX",
            arity: 3,
            flags: &["write", "fast"],
            first_key: 1,
            last_key: 2,
            step: 1,
        },
        CommandInfo {
            name: "TYPE",
            arity: 2,
            flags: &["readonly", "fast"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "COPY",
            arity: -3,
            flags: &["write", "denyoom"],
            first_key: 1,
            last_key: 2,
            step: 1,
        },
        CommandInfo {
            name: "DUMP",
            arity: 2,
            flags: &["readonly"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "RESTORE",
            arity: -4,
            flags: &["write", "denyoom"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "MIGRATE",
            arity: -6,
            flags: &["write"],
            first_key: 3,
            last_key: 3,
            step: 1,
        },
        CommandInfo {
            name: "EXPIRE",
            arity: -3,
            flags: &["write", "fast"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "EXPIREAT",
            arity: -3,
            flags: &["write", "fast"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "PEXPIRE",
            arity: -3,
            flags: &["write", "fast"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "PEXPIREAT",
            arity: -3,
            flags: &["write", "fast"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "TTL",
            arity: 2,
            flags: &["readonly", "fast"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "PTTL",
            arity: 2,
            flags: &["readonly", "fast"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "PERSIST",
            arity: 2,
            flags: &["write", "fast"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "EXPIRETIME",
            arity: 2,
            flags: &["readonly", "fast"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        CommandInfo {
            name: "PEXPIRETIME",
            arity: 2,
            flags: &["readonly", "fast"],
            first_key: 1,
            last_key: 1,
            step: 1,
        },
        // Server commands
        CommandInfo {
            name: "PING",
            arity: -1,
            flags: &["fast", "stale"],
            first_key: 0,
            last_key: 0,
            step: 0,
        },
        CommandInfo {
            name: "ECHO",
            arity: 2,
            flags: &["fast"],
            first_key: 0,
            last_key: 0,
            step: 0,
        },
        CommandInfo {
            name: "INFO",
            arity: -1,
            flags: &["stale", "fast"],
            first_key: 0,
            last_key: 0,
            step: 0,
        },
        CommandInfo {
            name: "CONFIG",
            arity: -2,
            flags: &["admin", "stale"],
            first_key: 0,
            last_key: 0,
            step: 0,
        },
        CommandInfo {
            name: "SLOWLOG",
            arity: -2,
            flags: &["admin", "stale"],
            first_key: 0,
            last_key: 0,
            step: 0,
        },
        CommandInfo {
            name: "TIME",
            arity: 1,
            flags: &["fast", "stale"],
            first_key: 0,
            last_key: 0,
            step: 0,
        },
        CommandInfo {
            name: "CLIENT",
            arity: -2,
            flags: &["admin", "stale"],
            first_key: 0,
            last_key: 0,
            step: 0,
        },
        CommandInfo {
            name: "COMMAND",
            arity: -1,
            flags: &["stale"],
            first_key: 0,
            last_key: 0,
            step: 0,
        },
        CommandInfo {
            name: "SAVE",
            arity: 1,
            flags: &["admin"],
            first_key: 0,
            last_key: 0,
            step: 0,
        },
        CommandInfo {
            name: "BGSAVE",
            arity: -1,
            flags: &["admin"],
            first_key: 0,
            last_key: 0,
            step: 0,
        },
        CommandInfo {
            name: "LASTSAVE",
            arity: 1,
            flags: &["fast", "stale"],
            first_key: 0,
            last_key: 0,
            step: 0,
        },
        CommandInfo {
            name: "SHUTDOWN",
            arity: -1,
            flags: &["admin"],
            first_key: 0,
            last_key: 0,
            step: 0,
        },
        CommandInfo {
            name: "MONITOR",
            arity: 1,
            flags: &["admin"],
            first_key: 0,
            last_key: 0,
            step: 0,
        },
        // Script commands
        CommandInfo {
            name: "EVAL",
            arity: -3,
            flags: &["write", "denyoom"],
            first_key: 0,
            last_key: 0,
            step: 0,
        },
        CommandInfo {
            name: "EVALSHA",
            arity: -3,
            flags: &["write", "denyoom"],
            first_key: 0,
            last_key: 0,
            step: 0,
        },
        CommandInfo {
            name: "SCRIPT",
            arity: -2,
            flags: &["admin"],
            first_key: 0,
            last_key: 0,
            step: 0,
        },
        // Connection commands
        CommandInfo {
            name: "HELLO",
            arity: -1,
            flags: &["fast", "stale"],
            first_key: 0,
            last_key: 0,
            step: 0,
        },
    ]
}

/// Generate a random 40-character hex string for run_id (similar to Redis)
fn generate_run_id() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};

    let mut result = String::with_capacity(40);
    let hasher_builder = RandomState::new();

    // Generate enough random data for a 40-char hex string
    // Include loop index to ensure uniqueness even when called rapidly
    for i in 0..5 {
        let mut hasher = hasher_builder.build_hasher();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        hasher.write_u64(nanos.wrapping_add(i as u64));
        result.push_str(&format!("{:016x}", hasher.finish()));
    }

    result.truncate(40);
    result
}

impl ServerCommands {
    pub fn new() -> Self {
        Self::with_port_and_cluster(6379, false)
    }

    pub fn with_port(port: u16) -> Self {
        Self::with_port_and_cluster(port, false)
    }

    pub fn with_port_and_cluster(port: u16, cluster_enabled: bool) -> Self {
        Self::with_storage_port_and_cluster(
            StorageEngine::new_memory(16),
            port,
            cluster_enabled,
            Arc::new(Metrics::new()),
        )
    }

    /// Build the default server configuration map for the given port.
    pub fn default_config(port: u16) -> HashMap<String, String> {
        let mut cfg = HashMap::new();
        cfg.insert("server".to_string(), "aikv".to_string());
        cfg.insert("version".to_string(), AIKV_VERSION.to_string());
        cfg.insert("port".to_string(), port.to_string());
        cfg.insert("databases".to_string(), "16".to_string());
        cfg.insert("loglevel".to_string(), "info".to_string());
        cfg.insert("slowlog-log-slower-than".to_string(), "10000".to_string());
        cfg.insert("slowlog-max-len".to_string(), "128".to_string());
        cfg.insert("maxclients".to_string(), "10000".to_string());
        cfg
    }

    pub fn with_storage_port_and_cluster(
        storage: StorageEngine,
        port: u16,
        cluster_enabled: bool,
        metrics: Arc<Metrics>,
    ) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        Self {
            storage,
            clients: Arc::new(RwLock::new(HashMap::new())),
            config: Arc::new(RwLock::new(Self::default_config(port))),
            start_time_secs: now,
            run_id: generate_run_id(),
            tcp_port: port,
            current_log_level: Arc::new(RwLock::new(Level::INFO)),
            slow_query_log: Arc::new(SlowQueryLog::new()),
            last_save_time: Arc::new(AtomicU64::new(now)),
            shutdown_requested: Arc::new(AtomicBool::new(false)),
            cluster_enabled,
            metrics,
        }
    }

    /// Create a `ServerCommands` that uses pre-existing shared state `Arc`s.
    ///
    /// This is the constructor used in production: all connections share the same
    /// `config`, `slow_query_log`, `clients`, and `current_log_level` so that
    /// changes made by one connection (e.g. CONFIG SET) are visible to others.
    pub fn with_shared_state(
        storage: StorageEngine,
        port: u16,
        cluster_enabled: bool,
        metrics: Arc<Metrics>,
        config: Arc<RwLock<HashMap<String, String>>>,
        slow_query_log: Arc<SlowQueryLog>,
        clients: Arc<RwLock<HashMap<usize, ClientInfo>>>,
        current_log_level: Arc<RwLock<Level>>,
    ) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        Self {
            storage,
            clients,
            config,
            slow_query_log,
            current_log_level,
            start_time_secs: now,
            run_id: generate_run_id(),
            tcp_port: port,
            last_save_time: Arc::new(AtomicU64::new(now)),
            shutdown_requested: Arc::new(AtomicBool::new(false)),
            cluster_enabled,
            metrics,
        }
    }

    /// Get the slow query log
    pub fn slow_query_log(&self) -> Arc<SlowQueryLog> {
        Arc::clone(&self.slow_query_log)
    }

    /// Get server metrics
    pub fn metrics(&self) -> Arc<Metrics> {
        Arc::clone(&self.metrics)
    }

    /// Get current log level
    pub fn log_level(&self) -> Level {
        *self
            .current_log_level
            .read()
            .unwrap_or_else(|e| e.into_inner())
    }

    /// Get server uptime in seconds (uses SystemTime for Docker/container compatibility)
    fn uptime_seconds(&self) -> u64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        now.saturating_sub(self.start_time_secs)
    }

    /// Build the Server section info lines
    fn build_server_info(&self) -> Vec<String> {
        let uptime_secs = self.uptime_seconds();
        let uptime_days = uptime_secs / 86400;
        let pid = std::process::id();

        vec![
            "# Server".to_string(),
            format!("redis_version:{}", REDIS_COMPAT_VERSION),
            "redis_git_sha1:00000000".to_string(),
            "redis_git_dirty:0".to_string(),
            format!("redis_build_id:aikv{}", AIKV_VERSION.replace('.', "")),
            format!(
                "redis_mode:{}",
                if self.cluster_enabled {
                    "cluster"
                } else {
                    "standalone"
                }
            ),
            format!(
                "os:{} {} {}",
                std::env::consts::OS,
                std::env::consts::FAMILY,
                std::env::consts::ARCH
            ),
            format!(
                "arch_bits:{}",
                if cfg!(target_pointer_width = "64") {
                    64
                } else {
                    32
                }
            ),
            "multiplexing_api:tokio".to_string(),
            format!("process_id:{}", pid),
            format!("run_id:{}", self.run_id),
            format!("tcp_port:{}", self.tcp_port),
            format!("uptime_in_seconds:{}", uptime_secs),
            format!("uptime_in_days:{}", uptime_days),
            "hz:10".to_string(),
            "configured_hz:10".to_string(),
            "lru_clock:0".to_string(),
            "executable:aikv".to_string(),
            "config_file:".to_string(),
            "io_threads_active:0".to_string(),
            format!("aikv_version:{}", AIKV_VERSION),
        ]
    }

    /// Build the Clients section info lines
    fn build_clients_info(&self) -> Result<Vec<String>> {
        let clients = self
            .clients
            .read()
            .map_err(|e| AikvError::Storage(format!("Lock error: {}", e)))?;

        let maxclients = self
            .config
            .read()
            .map_err(|e| AikvError::Storage(format!("Lock error: {}", e)))?
            .get("maxclients")
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(10000);

        Ok(vec![
            "# Clients".to_string(),
            format!("connected_clients:{}", clients.len()),
            "cluster_connections:0".to_string(),
            format!("maxclients:{}", maxclients),
            "client_recent_max_input_buffer:0".to_string(),
            "client_recent_max_output_buffer:0".to_string(),
            "blocked_clients:0".to_string(),
            "tracking_clients:0".to_string(),
            "clients_in_timeout_table:0".to_string(),
        "pubsub_clients:0".to_string(),
            "watching_clients:0".to_string(),
        ])
    }

    /// Build the Memory section info lines
    fn build_memory_info(&self) -> Vec<String> {
        let memory = &self.metrics.memory;
        let rss = Self::get_process_rss();

        // used_memory: 若 storage 未调用 set_used_memory，则用 RSS 作为 fallback
        let used = memory.used_memory();
        let effective_used = if used > 0 { used } else { rss.unwrap_or(0) };

        // used_memory_peak: 同上，未设置时用 effective_used
        let used_peak = memory.used_memory_peak();
        let effective_peak = if used_peak > 0 {
            used_peak
        } else {
            effective_used
        };

        let format_bytes = MemoryMetrics::format_bytes(effective_used);
        let format_peak = MemoryMetrics::format_bytes(effective_peak);

        // 计算内存碎片率
        let fragmentation = if effective_used > 0 {
            rss.map(|r| r as f64 / effective_used as f64).unwrap_or(1.0)
        } else {
            0.0
        };

        let rss = rss.unwrap_or_else(|| effective_used.saturating_mul(2));
        let rss_human = MemoryMetrics::format_bytes(rss);

        let total_sys_bytes = Self::get_system_total_memory().unwrap_or(0);
        let total_sys_human = MemoryMetrics::format_bytes(total_sys_bytes);

        vec![
            "# Memory".to_string(),
            format!("used_memory:{}", effective_used),
            format!("used_memory_human:{}", format_bytes),
            format!("used_memory_rss:{}", rss),
            format!("used_memory_rss_human:{}", rss_human),
            format!("used_memory_peak:{}", effective_peak),
            format!("used_memory_peak_human:{}", format_peak),
            format!(
                "used_memory_peak_perc:{:.2}%",
                if effective_peak > 0 {
                    (effective_used as f64 / effective_peak as f64) * 100.0
                } else {
                    0.0
                }
            ),
            format!("used_memory_overhead:{}", effective_used),
            format!("used_memory_startup:{}", effective_used),
            "used_memory_dataset:0".to_string(),
            "used_memory_dataset_perc:0.00%".to_string(),
            format!("allocator_allocated:{}", effective_used),
            format!("allocator_active:{}", rss),
            format!("allocator_resident:{}", rss),
            format!("total_system_memory:{}", total_sys_bytes),
            format!("total_system_memory_human:{}", total_sys_human),
            "used_memory_lua:0".to_string(),
            "used_memory_lua_human:0B".to_string(),
            "used_memory_scripts:0".to_string(),
            "used_memory_scripts_human:0B".to_string(),
            "maxmemory:0".to_string(),
            "maxmemory_human:0B".to_string(),
            "maxmemory_policy:noeviction".to_string(),
            format!("allocator_frag_ratio:{:.2}", fragmentation),
            "allocator_frag_bytes:0".to_string(),
            format!("allocator_rss_ratio:{:.2}", fragmentation),
            "allocator_rss_bytes:0".to_string(),
            format!("rss_overhead_ratio:{:.2}", fragmentation),
            "rss_overhead_bytes:0".to_string(),
            format!("mem_fragmentation_ratio:{:.2}", fragmentation),
            "mem_fragmentation_bytes:0".to_string(),
            "mem_not_counted_for_evict:0".to_string(),
            "mem_replication_backlog:0".to_string(),
            "mem_clients_slaves:0".to_string(),
            "mem_clients_normal:0".to_string(),
            "mem_aof_buffer:0".to_string(),
            "mem_allocator:jemalloc-5.3.0".to_string(),
            "active_defrag_running:0".to_string(),
            "lazyfree_pending_objects:0".to_string(),
            "lazyfreed_objects:0".to_string(),
        ]
    }

    /// 获取当前进程的 RSS（Resident Set Size）内存
    fn get_process_rss() -> Option<u64> {
        std::fs::read_to_string("/proc/self/status")
            .ok()?
            .lines()
            .find(|line| line.starts_with("VmRSS:"))
            .and_then(|line| {
                line.split_whitespace()
                    .nth(1)?
                    .parse::<u64>()
                    .ok()
                    .map(|kb| kb * 1024)
            })
    }

    /// 获取系统总内存（字节），从 /proc/meminfo MemTotal 读取，非 Linux 或解析失败返回 None
    fn get_system_total_memory() -> Option<u64> {
        std::fs::read_to_string("/proc/meminfo")
            .ok()?
            .lines()
            .find(|line| line.starts_with("MemTotal:"))
            .and_then(|line| {
                line.split_whitespace()
                    .nth(1)?
                    .parse::<u64>()
                    .ok()
                    .map(|kb| kb * 1024)
            })
    }

    /// Build the Stats section info lines
    fn build_stats_info(&self) -> Vec<String> {
        let mut info_lines = vec!["# Stats".to_string()];

        // Get stats from Metrics
        let stats = self.metrics.get_stats_info();
        for (key, value) in stats {
            info_lines.push(format!("{}:{}", key, value));
        }

        // Add other stats that are not in Metrics yet
        info_lines.push("sync_full:0".to_string());
        info_lines.push("sync_partial_ok:0".to_string());
        info_lines.push("sync_partial_err:0".to_string());
        info_lines.push("expired_stale_perc:0.00".to_string());
        info_lines.push("expired_time_cap_reached_count:0".to_string());
        info_lines.push("expire_cycle_cpu_milliseconds:0".to_string());
        info_lines.push("pubsub_channels:0".to_string());
        info_lines.push("pubsub_patterns:0".to_string());
        info_lines.push("latest_fork_usec:0".to_string());
        info_lines.push("total_forks:0".to_string());
        info_lines.push("migrate_cached_sockets:0".to_string());
        info_lines.push("slave_expires_tracked_keys:0".to_string());
        info_lines.push("active_defrag_hits:0".to_string());
        info_lines.push("active_defrag_misses:0".to_string());
        info_lines.push("active_defrag_key_hits:0".to_string());
        info_lines.push("active_defrag_key_misses:0".to_string());
        info_lines.push("tracking_total_keys:0".to_string());
        info_lines.push("tracking_total_items:0".to_string());
        info_lines.push("tracking_total_prefixes:0".to_string());
        info_lines.push(format!(
            "unexpected_error_replies:{}",
            self.metrics.commands.total_errors()
        ));
        info_lines.push(format!(
            "total_error_replies:{}",
            self.metrics.commands.total_errors()
        ));
        info_lines.push("dump_payload_sanitizations:0".to_string());
        // 无 read/write 拆分时，按 1:1 近似（reads + writes = total_commands）
        let total_cmds = self.metrics.commands.total_commands();
        let half = total_cmds / 2;
        info_lines.push(format!("total_reads_processed:{}", half));
        info_lines.push(format!("total_writes_processed:{}", total_cmds - half));
        info_lines.push("io_threaded_reads_processed:0".to_string());
        info_lines.push("io_threaded_writes_processed:0".to_string());
        // Redis 7.0+ eventloop (redis_exporter metricMapCounters)
        info_lines.push("eventloop_cycles:0".to_string());
        info_lines.push("eventloop_duration_sum:0".to_string());
        info_lines.push("eventloop_duration_cmd_sum:0".to_string());
        info_lines.push("reply_buffer_shrinks:0".to_string());
        info_lines.push("reply_buffer_expands:0".to_string());
        info_lines.push("client_query_buffer_limit_disconnections:0".to_string());
        info_lines.push("client_output_buffer_limit_disconnections:0".to_string());
        info_lines.push("acl_access_denied_auth:0".to_string());
        info_lines.push("acl_access_denied_cmd:0".to_string());
        info_lines.push("acl_access_denied_key:0".to_string());
        info_lines.push("acl_access_denied_channel:0".to_string());
        // Gauges (Redis 7.0+)
        info_lines.push("current_eviction_exceeded_time:0".to_string());
        info_lines.push("instantaneous_eventloop_cycles_per_sec:0".to_string());
        info_lines.push("instantaneous_eventloop_duration_usec:0".to_string());

        // AiKv-specific lock contention metrics (aikv_ prefix avoids collision with Redis fields).
        // redis-exporter converts every "key:value" line in INFO stats to a Prometheus metric,
        // so these automatically appear as redis_aikv_* gauges without any exporter changes.
        let locks = &self.metrics.locks;
        info_lines.push(format!(
            "aikv_storage_write_contentions:{}",
            locks.storage_write_contentions.get()
        ));
        info_lines.push(format!(
            "aikv_storage_write_wait_us_total:{}",
            locks.storage_write_wait_us.load(Ordering::Relaxed)
        ));
        info_lines.push(format!(
            "aikv_script_lock_wait_us_total:{}",
            locks.script_lock_wait_us.load(Ordering::Relaxed)
        ));
        info_lines.push(format!(
            "aikv_script_lock_timeouts:{}",
            locks.script_lock_timeouts.get()
        ));

        info_lines
    }

    /// Build the Replication section info lines
    fn build_replication_info(&self) -> Vec<String> {
        vec![
            "# Replication".to_string(),
            "role:master".to_string(),
            "connected_slaves:0".to_string(),
            "master_failover_state:no-failover".to_string(),
            "master_replid:0000000000000000000000000000000000000000".to_string(),
            "master_replid2:0000000000000000000000000000000000000000".to_string(),
            "master_repl_offset:0".to_string(),
            "second_repl_offset:-1".to_string(),
            "repl_backlog_active:0".to_string(),
            "repl_backlog_size:1048576".to_string(),
            "repl_backlog_first_byte_offset:0".to_string(),
            "repl_backlog_histlen:0".to_string(),
        ]
    }

    /// Build the CPU section info lines
    fn build_cpu_info(&self) -> Vec<String> {
        let (utime, stime) = Self::get_process_cpu_time().unwrap_or((0.0, 0.0));

        vec![
            "# CPU".to_string(),
            format!("used_cpu_sys:{:.6}", stime),
            format!("used_cpu_user:{:.6}", utime),
            "used_cpu_sys_children:0.000000".to_string(),
            "used_cpu_user_children:0.000000".to_string(),
            format!("used_cpu_sys_main_thread:{:.6}", stime),
            format!("used_cpu_user_main_thread:{:.6}", utime),
        ]
    }

    /// 获取进程 CPU 时间（用户态 + 系统态），单位秒
    fn get_process_cpu_time() -> Option<(f64, f64)> {
        // 读取 /proc/self/stat 获取进程的 CPU 时间
        // 字段 14 是 utime (用户态), 字段 15 是 stime (系统态)
        let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
        let parts: Vec<&str> = stat.split(' ').collect();

        if parts.len() < 15 {
            return None;
        }

        let utime: u64 = parts[13].parse().ok()?;
        let stime: u64 = parts[14].parse().ok()?;

        // 转换为秒 (Linux 通常 100 ticks/秒，可用 sysconf(_SC_CLK_TCK) 获取)
        const CLK_TCK: f64 = 100.0;
        Some((utime as f64 / CLK_TCK, stime as f64 / CLK_TCK))
    }

    /// Build the Commandstats section info lines (Redis compatible)
    /// Format: cmdstat_xxx:calls=N,usec=M,usec_per_call=X.XX
    fn build_commandstats_info(&self) -> Vec<String> {
        let mut lines = vec!["# Commandstats".to_string()];

        for (cmd, calls, usec) in self.metrics.commands.commandstats() {
            let usec_per_call = if calls > 0 {
                usec as f64 / calls as f64
            } else {
                0.0
            };
            lines.push(format!(
                "cmdstat_{}:calls={},usec={},usec_per_call={:.2}",
                cmd.to_lowercase(),
                calls,
                usec,
                usec_per_call
            ));
        }

        lines
    }

    /// Build the Modules section info lines
    fn build_modules_info(&self) -> Vec<String> {
        vec!["# Modules".to_string()]
    }

    /// Build the Errorstats section info lines (Redis compatible)
    /// Format: errorstat_<type>:count=N (redis_exporter parseMetricsErrorStats expects this)
    /// AiKv keys by command name (GET, SET, etc.); Redis keys by error type (ERR, WRONGTYPE)
    fn build_errorstats_info(&self) -> Vec<String> {
        let mut lines = vec!["# Errorstats".to_string()];

        let mut errors: Vec<_> = self.metrics.commands.errors_by_type().into_iter().collect();
        errors.sort_by(|a, b| a.0.cmp(&b.0));

        for (key, count) in errors {
            if count > 0 {
                let key_normalized = key.to_lowercase().replace(' ', "_");
                lines.push(format!("errorstat_{}:count={}", key_normalized, count));
            }
        }

        lines
    }

    /// Build the Cluster section info lines
    fn build_cluster_info(&self) -> Vec<String> {
        #[cfg(feature = "cluster")]
        {
            vec![
                "# Cluster".to_string(),
                "cluster_enabled:1".to_string(),
                "cluster_stats_messages_sent:0".to_string(),
                "cluster_stats_messages_received:0".to_string(),
            ]
        }
        #[cfg(not(feature = "cluster"))]
        {
            vec![
                "# Cluster".to_string(),
                "cluster_enabled:0".to_string(),
                "cluster_stats_messages_sent:0".to_string(),
                "cluster_stats_messages_received:0".to_string(),
            ]
        }
    }

    /// Build the Keyspace section info lines
    /// Format: dbN:keys=X,expires=Y,avg_ttl=Z (Redis compatible)
    fn build_keyspace_info(&self) -> Vec<String> {
        // The keyspace scan (db.iter() across all AiDb MemTables) is too slow to
        // run inline — under heavy write load it takes 30-90+ seconds because it
        // must merge every unflushed MemTable.  The background task in
        // server::Server::spawn_keyspace_refresh_task() keeps the cache warm; we
        // just return whatever is there.  If the cache is empty (server just
        // started) we return an empty section — the exporter will get real data
        // after the first background scan completes (~3 s after startup).
        let cache = self.metrics.keyspace_cache.lock().unwrap_or_else(|e| e.into_inner());
        match &*cache {
            Some(snap) => Self::format_keyspace_stats(&snap.stats),
            None => vec!["# Keyspace".to_string()],
        }
    }

    fn format_keyspace_stats(stats: &[(usize, usize, u64)]) -> Vec<String> {
        let mut lines = vec!["# Keyspace".to_string()];
        for (db_index, &(keys, expires, avg_ttl)) in stats.iter().enumerate() {
            if keys > 0 {
                lines.push(format!(
                    "db{}:keys={},expires={},avg_ttl={}",
                    db_index, keys, expires, avg_ttl
                ));
            }
        }
        lines
    }

    /// Build the Persistence section info lines
    fn build_persistence_info(&self) -> Vec<String> {
        vec![
            "# Persistence".to_string(),
            "loading:0".to_string(),
            "current_cow_size:0".to_string(),
            "current_cow_size_age:0".to_string(),
            "current_fork_perc:0.00".to_string(),
            "current_save_keys_processed:0".to_string(),
            "current_save_keys_total:0".to_string(),
            "rdb_changes_since_last_save:0".to_string(),
            "rdb_bgsave_in_progress:0".to_string(),
            "rdb_last_save_time:0".to_string(),
            "rdb_last_bgsave_status:ok".to_string(),
            "rdb_last_bgsave_time_sec:-1".to_string(),
            "rdb_current_bgsave_time_sec:-1".to_string(),
            "rdb_last_cow_size:0".to_string(),
            "aof_enabled:0".to_string(),
            "aof_rewrite_in_progress:0".to_string(),
            "aof_rewrite_scheduled:0".to_string(),
            "aof_last_rewrite_time_sec:-1".to_string(),
            "aof_current_rewrite_time_sec:-1".to_string(),
            "aof_last_bgrewrite_status:ok".to_string(),
            "aof_last_write_status:ok".to_string(),
            "aof_last_cow_size:0".to_string(),
            "module_fork_in_progress:0".to_string(),
            "module_fork_last_cow_size:0".to_string(),
        ]
    }

    /// INFO \[section\] - Get server information
    pub fn info(&self, args: &[Bytes]) -> Result<RespValue> {
        let section = if args.is_empty() {
            "default"
        } else {
            &String::from_utf8_lossy(&args[0])
        };

        let mut info_lines = Vec::new();

        match section.to_lowercase().as_str() {
            // "default" returns the standard default sections (server, clients, memory,
            // persistence, stats, replication, cpu, cluster, keyspace)
            // This is required for redis-cli --cluster create to detect cluster_enabled:1
            "default" => {
                info_lines.extend(self.build_server_info());
                info_lines.push(String::new());
                info_lines.extend(self.build_clients_info()?);
                info_lines.push(String::new());
                info_lines.extend(self.build_memory_info());
                info_lines.push(String::new());
                info_lines.extend(self.build_persistence_info());
                info_lines.push(String::new());
                info_lines.extend(self.build_stats_info());
                info_lines.push(String::new());
                info_lines.extend(self.build_replication_info());
                info_lines.push(String::new());
                info_lines.extend(self.build_cpu_info());
                info_lines.push(String::new());
                info_lines.extend(self.build_cluster_info());
                info_lines.push(String::new());
                info_lines.extend(self.build_keyspace_info());
            }
            "server" => {
                info_lines.extend(self.build_server_info());
            }
            "clients" => {
                info_lines.extend(self.build_clients_info()?);
            }
            "memory" => {
                info_lines.extend(self.build_memory_info());
            }
            "stats" => {
                info_lines.extend(self.build_stats_info());
            }
            "replication" => {
                info_lines.extend(self.build_replication_info());
            }
            "cpu" => {
                info_lines.extend(self.build_cpu_info());
            }
            "commandstats" => {
                info_lines.extend(self.build_commandstats_info());
            }
            "modules" => {
                info_lines.extend(self.build_modules_info());
            }
            "errorstats" => {
                info_lines.extend(self.build_errorstats_info());
            }
            "cluster" => {
                info_lines.extend(self.build_cluster_info());
            }
            "keyspace" => {
                info_lines.extend(self.build_keyspace_info());
            }
            "persistence" => {
                info_lines.extend(self.build_persistence_info());
            }
            // "all" or "everything" returns all sections (including modules and errorstats)
            "all" | "everything" => {
                info_lines.extend(self.build_server_info());
                info_lines.push(String::new());
                info_lines.extend(self.build_clients_info()?);
                info_lines.push(String::new());
                info_lines.extend(self.build_memory_info());
                info_lines.push(String::new());
                info_lines.extend(self.build_persistence_info());
                info_lines.push(String::new());
                info_lines.extend(self.build_stats_info());
                info_lines.push(String::new());
                info_lines.extend(self.build_replication_info());
                info_lines.push(String::new());
                info_lines.extend(self.build_cpu_info());
                info_lines.push(String::new());
                info_lines.extend(self.build_commandstats_info());
                info_lines.push(String::new());
                info_lines.extend(self.build_modules_info());
                info_lines.push(String::new());
                info_lines.extend(self.build_errorstats_info());
                info_lines.push(String::new());
                info_lines.extend(self.build_cluster_info());
                info_lines.push(String::new());
                info_lines.extend(self.build_keyspace_info());
            }
            // For unknown sections, return empty result (Redis behavior)
            // This is important for compatibility - Redis returns empty for unknown sections
            _ => {
                // Return empty bulk string for unknown sections to maintain compatibility
            }
        }

        let info_str = info_lines.join("\r\n");
        Ok(RespValue::bulk_string(info_str))
    }

    /// CONFIG GET parameter - Get configuration value
    pub fn config_get(&self, args: &[Bytes]) -> Result<RespValue> {
        if args.len() != 1 {
            return Err(AikvError::WrongArgCount("CONFIG GET".to_string()));
        }

        let parameter = String::from_utf8_lossy(&args[0]).to_lowercase();
        let config = self
            .config
            .read()
            .map_err(|e| AikvError::Storage(format!("Lock error: {}", e)))?;

        let mut results = Vec::new();

        // Helper function to check if pattern matches key (supports * wildcard)
        let matches_pattern = |pattern: &str, key: &str| -> bool {
            if pattern == "*" {
                return true;
            }
            // Simple glob matching for patterns like "cluster*"
            if let Some(prefix) = pattern.strip_suffix('*') {
                key.starts_with(prefix)
            } else {
                pattern == key
            }
        };

        // Built-in cluster configuration values (read-only, derived from runtime state)
        let builtin_configs: Vec<(&str, String)> = vec![
            (
                "cluster-enabled",
                if self.cluster_enabled {
                    "yes".to_string()
                } else {
                    "no".to_string()
                },
            ),
            ("cluster-node-timeout", "15000".to_string()),
            ("cluster-announce-port", self.tcp_port.to_string()),
            (
                "cluster-announce-bus-port",
                (self.tcp_port + 10000).to_string(),
            ),
        ];

        // Support wildcard matching
        if parameter == "*" {
            // Add built-in configs first
            for (key, value) in &builtin_configs {
                results.push(RespValue::bulk_string(key.to_string()));
                results.push(RespValue::bulk_string(value.clone()));
            }
            // Add user-defined configs
            for (key, value) in config.iter() {
                results.push(RespValue::bulk_string(key.clone()));
                results.push(RespValue::bulk_string(value.clone()));
            }
        } else {
            // Check built-in configs first
            for (key, value) in &builtin_configs {
                if matches_pattern(&parameter, key) {
                    results.push(RespValue::bulk_string(key.to_string()));
                    results.push(RespValue::bulk_string(value.clone()));
                }
            }
            // Check user-defined configs
            for (key, value) in config.iter() {
                if matches_pattern(&parameter, key) {
                    results.push(RespValue::bulk_string(key.clone()));
                    results.push(RespValue::bulk_string(value.clone()));
                }
            }
        }

        Ok(RespValue::array(results))
    }

    /// CONFIG SET parameter value - Set configuration value
    pub fn config_set(&self, args: &[Bytes]) -> Result<RespValue> {
        if args.len() != 2 {
            return Err(AikvError::WrongArgCount("CONFIG SET".to_string()));
        }

        let parameter = String::from_utf8_lossy(&args[0]).to_string();
        let value = String::from_utf8_lossy(&args[1]).to_string();

        // Handle special parameters with side effects (case-insensitive comparison)
        let param_lower = parameter.to_lowercase();
        if param_lower == "server" || param_lower == "version" || param_lower == "port" {
            return Err(AikvError::InvalidArgument(
                "ERR configuration parameter is read-only".to_string(),
            ));
        } else if param_lower == "loglevel" {
            // Dynamic log level adjustment
            if let Some(level) = LogConfig::parse_level(&value) {
                if let Ok(mut current) = self.current_log_level.write() {
                    *current = level;
                }
            } else {
                return Err(AikvError::InvalidArgument(format!(
                    "ERR invalid log level: {}",
                    value
                )));
            }
        } else if param_lower == "slowlog-log-slower-than" {
            // Update slow query threshold
            match value.parse::<u64>() {
                Ok(threshold) => {
                    self.slow_query_log.set_threshold_us(threshold);
                }
                Err(_) => {
                    return Err(AikvError::InvalidArgument(
                        "ERR invalid slowlog threshold value".to_string(),
                    ));
                }
            }
        } else if param_lower == "slowlog-max-len" {
            // Update slow query max length
            match value.parse::<usize>() {
                Ok(max_len) => {
                    self.slow_query_log.set_max_len(max_len);
                }
                Err(_) => {
                    return Err(AikvError::InvalidArgument(
                        "ERR invalid slowlog max length value".to_string(),
                    ));
                }
            }
        }

        let mut config = self
            .config
            .write()
            .map_err(|e| AikvError::Storage(format!("Lock error: {}", e)))?;

        config.insert(parameter, value);
        Ok(RespValue::ok())
    }

    /// SLOWLOG subcommand - Manage the slow query log
    pub fn slowlog(&self, args: &[Bytes]) -> Result<RespValue> {
        if args.is_empty() {
            return Err(AikvError::WrongArgCount("SLOWLOG".to_string()));
        }

        let subcommand = String::from_utf8_lossy(&args[0]).to_uppercase();

        match subcommand.as_str() {
            "GET" => {
                // SLOWLOG GET [count]
                let count = if args.len() > 1 {
                    String::from_utf8_lossy(&args[1])
                        .parse::<usize>()
                        .unwrap_or(10)
                } else {
                    10
                };

                let entries = self.slow_query_log.get(count);
                let result: Vec<RespValue> = entries
                    .iter()
                    .map(|entry| {
                        RespValue::array(vec![
                            RespValue::integer(entry.id as i64),
                            RespValue::integer(entry.timestamp as i64),
                            RespValue::integer(entry.duration_us as i64),
                            RespValue::array(
                                std::iter::once(RespValue::bulk_string(entry.command.clone()))
                                    .chain(
                                        entry
                                            .args
                                            .iter()
                                            .map(|a| RespValue::bulk_string(a.clone())),
                                    )
                                    .collect(),
                            ),
                            RespValue::bulk_string(
                                entry.client_addr.clone().unwrap_or_else(|| "".to_string()),
                            ),
                            RespValue::bulk_string(""), // client name (not tracked)
                        ])
                    })
                    .collect();

                Ok(RespValue::array(result))
            }
            "LEN" => {
                // SLOWLOG LEN
                Ok(RespValue::integer(self.slow_query_log.len() as i64))
            }
            "RESET" => {
                // SLOWLOG RESET
                self.slow_query_log.reset();
                Ok(RespValue::ok())
            }
            "HELP" => {
                // SLOWLOG HELP
                Ok(RespValue::array(vec![
                    RespValue::bulk_string("SLOWLOG GET [count] - Get the slow log entries"),
                    RespValue::bulk_string("SLOWLOG LEN - Get the slow log length"),
                    RespValue::bulk_string("SLOWLOG RESET - Reset the slow log"),
                    RespValue::bulk_string("SLOWLOG HELP - Show this help"),
                ]))
            }
            _ => Err(AikvError::InvalidCommand(format!(
                "Unknown SLOWLOG subcommand: {}",
                subcommand
            ))),
        }
    }

    /// LATENCY subcommand - Latency monitoring (Redis 7.0+ compatible)
    pub fn latency(&self, args: &[Bytes]) -> Result<RespValue> {
        if args.is_empty() {
            return Err(AikvError::WrongArgCount("LATENCY".to_string()));
        }

        let subcommand = String::from_utf8_lossy(&args[0]).to_uppercase();

        match subcommand.as_str() {
            "HISTOGRAM" => {
                // LATENCY HISTOGRAM [command [command ...]]
                // Returns per-command latency histograms in Redis RESP2 flat-array format.
                // redis-exporter calls this to generate redis_commands_latencies_usec_bucket metrics.
                let filter: Option<Vec<&str>> = if args.len() > 1 {
                    Some(
                        args[1..]
                            .iter()
                            .map(|b| std::str::from_utf8(b).unwrap_or(""))
                            .collect(),
                    )
                } else {
                    None
                };

                let snapshots = self
                    .metrics
                    .commands
                    .get_latency_histogram_snapshot(filter.as_deref());

                // Build flat top-level array: [cmd1, details1, cmd2, details2, ...]
                let mut result_items: Vec<RespValue> = Vec::with_capacity(snapshots.len() * 2);

                for (cmd, total_calls, _total_usec, buckets) in snapshots {
                    // Build flat bucket array: [bound1, count1, bound2, count2, ...]
                    let mut bucket_items: Vec<RespValue> = Vec::with_capacity(buckets.len() * 2);
                    for (bound, count) in &buckets {
                        bucket_items.push(RespValue::integer(*bound as i64));
                        bucket_items.push(RespValue::integer(*count as i64));
                    }

                    // Detail array: ["calls", N, "histogram_usec", [bucket_flat_array]]
                    let detail = RespValue::array(vec![
                        RespValue::bulk_string("calls"),
                        RespValue::integer(total_calls as i64),
                        RespValue::bulk_string("histogram_usec"),
                        RespValue::array(bucket_items),
                    ]);

                    result_items.push(RespValue::bulk_string(cmd));
                    result_items.push(detail);
                }

                Ok(RespValue::array(result_items))
            }

            "LATEST" => {
                // LATENCY LATEST
                // Returns latest latency spikes per event. AiKv does not track spike events,
                // so return an empty array. redis-exporter handles this gracefully.
                Ok(RespValue::array(vec![]))
            }

            "RESET" => {
                // LATENCY RESET [event [event ...]]
                let filter: Option<Vec<&str>> = if args.len() > 1 {
                    Some(
                        args[1..]
                            .iter()
                            .map(|b| std::str::from_utf8(b).unwrap_or(""))
                            .collect(),
                    )
                } else {
                    None
                };

                let count = self
                    .metrics
                    .commands
                    .reset_latency_histograms(filter.as_deref());

                Ok(RespValue::integer(count as i64))
            }

            "HISTORY" => {
                // LATENCY HISTORY event
                // AiKv does not keep per-event time-series, return empty array.
                Ok(RespValue::array(vec![]))
            }

            "HELP" => Ok(RespValue::array(vec![
                RespValue::bulk_string(
                    "LATENCY HISTOGRAM [command ...] - Show latency histogram per command",
                ),
                RespValue::bulk_string(
                    "LATENCY LATEST - Show latest latency spike events",
                ),
                RespValue::bulk_string(
                    "LATENCY HISTORY event - Show latency time series for an event",
                ),
                RespValue::bulk_string(
                    "LATENCY RESET [event ...] - Reset latency data (all or specific commands)",
                ),
            ])),

            _ => Err(AikvError::InvalidCommand(format!(
                "Unknown LATENCY subcommand: {}",
                subcommand
            ))),
        }
    }

    /// TIME - Return the current server time
    pub fn time(&self, _args: &[Bytes]) -> Result<RespValue> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| AikvError::Storage(format!("Time error: {}", e)))?;

        let seconds = now.as_secs();
        let microseconds = now.subsec_micros();

        Ok(RespValue::array(vec![
            RespValue::bulk_string(seconds.to_string()),
            RespValue::bulk_string(microseconds.to_string()),
        ]))
    }

    /// CLIENT LIST - List all client connections
    pub fn client_list(&self, _args: &[Bytes]) -> Result<RespValue> {
        let clients = self
            .clients
            .read()
            .map_err(|e| AikvError::Storage(format!("Lock error: {}", e)))?;

        let mut client_lines = Vec::new();
        for (id, client) in clients.iter() {
            let name = client
                .name
                .as_ref()
                .map(|n| format!(" name={}", n))
                .unwrap_or_default();
            client_lines.push(format!("id={} addr={}{}", id, client.addr, name));
        }

        let client_str = client_lines.join("\n");
        Ok(RespValue::bulk_string(client_str))
    }

    /// CLIENT SETNAME name - Set client name
    pub fn client_setname(&self, args: &[Bytes], client_id: usize) -> Result<RespValue> {
        if args.len() != 1 {
            return Err(AikvError::WrongArgCount("CLIENT SETNAME".to_string()));
        }

        let name = String::from_utf8_lossy(&args[0]).to_string();

        let mut clients = self
            .clients
            .write()
            .map_err(|e| AikvError::Storage(format!("Lock error: {}", e)))?;

        if let Some(client) = clients.get_mut(&client_id) {
            client.name = Some(name);
        }

        Ok(RespValue::ok())
    }

    /// CLIENT GETNAME - Get client name
    pub fn client_getname(&self, _args: &[Bytes], client_id: usize) -> Result<RespValue> {
        let clients = self
            .clients
            .read()
            .map_err(|e| AikvError::Storage(format!("Lock error: {}", e)))?;

        if let Some(client) = clients.get(&client_id) {
            if let Some(name) = &client.name {
                return Ok(RespValue::bulk_string(name.clone()));
            }
        }

        Ok(RespValue::null_bulk_string())
    }

    /// Register a client
    pub fn register_client(&self, id: usize, addr: String) -> Result<()> {
        let mut clients = self
            .clients
            .write()
            .map_err(|e| AikvError::Storage(format!("Lock error: {}", e)))?;

        clients.insert(
            id,
            ClientInfo {
                id,
                name: None,
                addr,
            },
        );
        Ok(())
    }

    /// Unregister a client
    pub fn unregister_client(&self, id: usize) -> Result<()> {
        let mut clients = self
            .clients
            .write()
            .map_err(|e| AikvError::Storage(format!("Lock error: {}", e)))?;

        clients.remove(&id);
        Ok(())
    }

    /// COMMAND - Get array of all commands or specific command info
    pub fn command(&self, args: &[Bytes]) -> Result<RespValue> {
        if args.is_empty() {
            // COMMAND with no args returns all commands
            let commands = get_command_table();
            let result: Vec<RespValue> = commands
                .iter()
                .map(|cmd| self.format_command_info(cmd))
                .collect();
            return Ok(RespValue::array(result));
        }

        let subcommand = String::from_utf8_lossy(&args[0]).to_uppercase();
        match subcommand.as_str() {
            "COUNT" => self.command_count(),
            "INFO" => self.command_info(&args[1..]),
            "DOCS" => self.command_docs(&args[1..]),
            "GETKEYS" => self.command_getkeys(&args[1..]),
            "HELP" => self.command_help(),
            _ => Err(AikvError::InvalidCommand(format!(
                "Unknown COMMAND subcommand: {}",
                subcommand
            ))),
        }
    }

    /// Format a single command info for COMMAND response
    fn format_command_info(&self, cmd: &CommandInfo) -> RespValue {
        let flags: Vec<RespValue> = cmd
            .flags
            .iter()
            .map(|f| RespValue::simple_string(*f))
            .collect();

        RespValue::array(vec![
            RespValue::bulk_string(cmd.name.to_lowercase()),
            RespValue::integer(cmd.arity),
            RespValue::array(flags),
            RespValue::integer(cmd.first_key),
            RespValue::integer(cmd.last_key),
            RespValue::integer(cmd.step),
        ])
    }

    /// COMMAND COUNT - Get total number of commands
    fn command_count(&self) -> Result<RespValue> {
        let count = get_command_table().len();
        Ok(RespValue::integer(count as i64))
    }

    /// COMMAND INFO command [command ...] - Get info for specific commands
    fn command_info(&self, args: &[Bytes]) -> Result<RespValue> {
        if args.is_empty() {
            return Err(AikvError::WrongArgCount("COMMAND INFO".to_string()));
        }

        let commands = get_command_table();
        let command_map: HashMap<&str, &CommandInfo> =
            commands.iter().map(|c| (c.name, c)).collect();

        let result: Vec<RespValue> = args
            .iter()
            .map(|arg| {
                let name = String::from_utf8_lossy(arg).to_uppercase();
                match command_map.get(name.as_str()) {
                    Some(cmd) => self.format_command_info(cmd),
                    None => RespValue::null_bulk_string(),
                }
            })
            .collect();

        Ok(RespValue::array(result))
    }

    /// COMMAND DOCS - Get command documentation (simplified)
    fn command_docs(&self, args: &[Bytes]) -> Result<RespValue> {
        if args.is_empty() {
            // Return docs for all commands (simplified)
            let commands = get_command_table();
            let result: Vec<(RespValue, RespValue)> = commands
                .iter()
                .map(|cmd| {
                    (
                        RespValue::bulk_string(cmd.name.to_lowercase()),
                        RespValue::map(vec![
                            (
                                RespValue::bulk_string("summary"),
                                RespValue::bulk_string(format!("{} command", cmd.name)),
                            ),
                            (
                                RespValue::bulk_string("since"),
                                RespValue::bulk_string("1.0.0"),
                            ),
                            (
                                RespValue::bulk_string("group"),
                                RespValue::bulk_string("generic"),
                            ),
                        ]),
                    )
                })
                .collect();
            return Ok(RespValue::map(result));
        }

        // Return docs for specific commands
        let commands = get_command_table();
        let command_map: HashMap<&str, &CommandInfo> =
            commands.iter().map(|c| (c.name, c)).collect();

        let result: Vec<(RespValue, RespValue)> = args
            .iter()
            .filter_map(|arg| {
                let name = String::from_utf8_lossy(arg).to_uppercase();
                command_map.get(name.as_str()).map(|cmd| {
                    (
                        RespValue::bulk_string(cmd.name.to_lowercase()),
                        RespValue::map(vec![
                            (
                                RespValue::bulk_string("summary"),
                                RespValue::bulk_string(format!("{} command", cmd.name)),
                            ),
                            (
                                RespValue::bulk_string("since"),
                                RespValue::bulk_string("1.0.0"),
                            ),
                            (
                                RespValue::bulk_string("group"),
                                RespValue::bulk_string("generic"),
                            ),
                        ]),
                    )
                })
            })
            .collect();

        Ok(RespValue::map(result))
    }

    /// COMMAND GETKEYS command [arg ...] - Extract keys from a command
    fn command_getkeys(&self, args: &[Bytes]) -> Result<RespValue> {
        if args.is_empty() {
            return Err(AikvError::WrongArgCount("COMMAND GETKEYS".to_string()));
        }

        let cmd_name = String::from_utf8_lossy(&args[0]).to_uppercase();
        let commands = get_command_table();

        let cmd = commands.iter().find(|c| c.name == cmd_name.as_str());

        match cmd {
            Some(cmd_info) => {
                if cmd_info.first_key == 0 {
                    return Ok(RespValue::array(vec![]));
                }

                let cmd_args = &args[1..];
                let mut keys = Vec::new();

                if cmd_info.last_key == -1 {
                    // All remaining args from first_key are keys
                    let start = (cmd_info.first_key - 1) as usize;
                    for (i, arg) in cmd_args.iter().enumerate().skip(start) {
                        if cmd_info.step == 1 || (i - start) % cmd_info.step as usize == 0 {
                            keys.push(RespValue::bulk_string(arg.clone()));
                        }
                    }
                } else if cmd_info.last_key >= cmd_info.first_key {
                    let start = (cmd_info.first_key - 1) as usize;
                    let end = (cmd_info.last_key - 1) as usize;
                    for i in (start..=end.min(cmd_args.len().saturating_sub(1)))
                        .step_by(cmd_info.step.max(1) as usize)
                    {
                        if i < cmd_args.len() {
                            keys.push(RespValue::bulk_string(cmd_args[i].clone()));
                        }
                    }
                }

                Ok(RespValue::array(keys))
            }
            None => Err(AikvError::InvalidCommand(format!(
                "Invalid command specified: {}",
                cmd_name
            ))),
        }
    }

    /// COMMAND HELP - Show help for COMMAND subcommands
    fn command_help(&self) -> Result<RespValue> {
        Ok(RespValue::array(vec![
            RespValue::bulk_string("COMMAND - Return information about all commands"),
            RespValue::bulk_string("COMMAND COUNT - Return the total number of commands"),
            RespValue::bulk_string(
                "COMMAND INFO <command-name> [<command-name> ...] - Return details about command(s)",
            ),
            RespValue::bulk_string(
                "COMMAND DOCS [<command-name> ...] - Return documentation for command(s)",
            ),
            RespValue::bulk_string(
                "COMMAND GETKEYS <command> [<arg> ...] - Extract keys from a full command",
            ),
            RespValue::bulk_string("COMMAND HELP - Show this help"),
        ]))
    }

    /// CONFIG REWRITE - Rewrite the configuration file
    pub fn config_rewrite(&self, _args: &[Bytes]) -> Result<RespValue> {
        // In AiKv, we don't persist configuration changes to a file automatically
        // This is a stub that returns OK for compatibility
        // A real implementation would write the current config to the config file
        Ok(RespValue::ok())
    }

    /// SAVE - Synchronously save the dataset to disk
    pub fn save(&self, args: &[Bytes]) -> Result<RespValue> {
        if !args.is_empty() {
            return Err(AikvError::WrongArgCount("SAVE".to_string()));
        }

        // Export all databases from storage
        let databases = self.storage.export_all_databases()?;

        // Create a temporary file for the RDB dump
        let temp_file = tempfile::NamedTempFile::new()
            .map_err(|e| AikvError::Persistence(format!("Failed to create temp file: {}", e)))?;
        let temp_path = temp_file.path();

        // Save to RDB format
        crate::persistence::save_stored_value_rdb(temp_path, &databases)?;

        // Update last save time
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.last_save_time.store(now, Ordering::SeqCst);

        // For now, we save to a temporary file and don't persist it permanently
        // In a real implementation, this would save to a configured RDB file path
        Ok(RespValue::ok())
    }

    /// BGSAVE - Asynchronously save the dataset to disk
    pub fn bgsave(&self, args: &[Bytes]) -> Result<RespValue> {
        if !args.is_empty() {
            return Err(AikvError::WrongArgCount("BGSAVE".to_string()));
        }

        // For now, perform synchronous save (background save would require threading)
        // In a real implementation, this would spawn a background thread
        self.save(args)?;

        Ok(RespValue::simple_string("Background saving started"))
    }

    /// LASTSAVE - Get the Unix timestamp of the last successful save
    pub fn lastsave(&self, _args: &[Bytes]) -> Result<RespValue> {
        let last_save = self.last_save_time.load(Ordering::SeqCst);
        Ok(RespValue::integer(last_save as i64))
    }

    /// SHUTDOWN - Shut down the server
    /// Note: This sets a shutdown flag but doesn't actually terminate the process
    /// The actual shutdown should be handled by the server loop
    pub fn shutdown(&self, args: &[Bytes]) -> Result<RespValue> {
        // Parse optional arguments: NOSAVE, SAVE, NOW, FORCE, ABORT
        let mut _nosave = false;
        let mut _save = false;
        let mut _now = false;
        let mut _force = false;
        let mut abort = false;

        for arg in args {
            let arg_str = String::from_utf8_lossy(arg).to_uppercase();
            match arg_str.as_str() {
                "NOSAVE" => _nosave = true,
                "SAVE" => _save = true,
                "NOW" => _now = true,
                "FORCE" => _force = true,
                "ABORT" => abort = true,
                _ => {
                    return Err(AikvError::InvalidArgument(format!(
                        "Unknown SHUTDOWN option: {}",
                        arg_str
                    )));
                }
            }
        }

        if abort {
            // Abort a pending shutdown
            self.shutdown_requested.store(false, Ordering::SeqCst);
            return Ok(RespValue::ok());
        }

        // Set shutdown flag
        self.shutdown_requested.store(true, Ordering::SeqCst);

        // In a real implementation, the server would check this flag and exit gracefully
        // For now, we just return an error indicating shutdown was requested
        // The connection will be closed, and the client should reconnect
        Err(AikvError::Storage("Server is shutting down".to_string()))
    }

    /// Check if shutdown has been requested
    pub fn is_shutdown_requested(&self) -> bool {
        self.shutdown_requested.load(Ordering::SeqCst)
    }
}

impl Default for ServerCommands {
    fn default() -> Self {
        Self::new()
    }
}
