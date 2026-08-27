//! CommandRouter 指标与 cluster 命令分类辅助.

use std::sync::Arc;

use crate::error::{Error, Result};
use crate::protocol::RespValue;
use crate::server::ServerMetrics;

pub(super) fn record_keyspace_lookup(metrics: &Option<Arc<ServerMetrics>>, hit: bool) {
    let Some(metrics) = metrics else {
        return;
    };
    if hit {
        metrics.on_keyspace_hit();
    } else {
        metrics.on_keyspace_miss();
    }
}

pub(super) fn record_command_outcome(
    metrics: &Option<Arc<ServerMetrics>>,
    cmd: &str,
    result: &Result<RespValue>,
) {
    let Some(metrics) = metrics else {
        return;
    };
    metrics.on_command(cmd, result.is_ok());
    if let Err(err) = result {
        if let Some(msg) = error_message_for_stats(err) {
            metrics.on_error_stat(msg);
        }
    }
}

pub(super) fn error_message_for_stats(err: &Error) -> Option<&str> {
    match err {
        Error::Command(msg) | Error::Protocol(msg) | Error::Storage(msg) | Error::Config(msg) => {
            Some(msg.as_str())
        }
        Error::Io(_) => None,
        #[cfg(feature = "cluster")]
        Error::Cluster(_) => None,
    }
}

#[cfg(feature = "monitoring")]
pub(super) fn record_command_span_status(result: &Result<RespValue>) {
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
pub(super) fn is_multi_key_cmd(cmd: &str) -> bool {
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
pub(super) fn classify_command(cmd: &str) -> crate::cluster::router::CommandType {
    match cmd.to_ascii_lowercase().as_str() {
        "get" | "exists" | "hget" | "hgetall" | "hkeys" | "hvals" | "hlen" | "hexists"
        | "lrange" | "lindex" | "llen" | "smembers" | "scard" | "sismember" | "zrange"
        | "zcard" | "zscore" | "zrank" | "type" | "ttl" | "pttl" | "strlen" | "getbit"
        | "getrange" | "mget" | "json.get" | "json.mget" => {
            crate::cluster::router::CommandType::Read
        }
        _ => crate::cluster::router::CommandType::Write,
    }
}
