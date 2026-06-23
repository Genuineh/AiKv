//! 可观测性测试

use std::time::Duration;

use super::helpers::{connect, start_server, write_cmd};
#[cfg(feature = "monitoring")]
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn test_connection_metrics() {
    let storage = aikv::storage::MemoryEngine::new(16);
    let state = aikv::server::ServerSharedState::new(
        aikv::server::ConnectionConfig {
            read_timeout: None,
            idle_timeout: None,
            max_clients: 0,
        },
        storage,
        0,
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state_clone = std::sync::Arc::clone(&state);
    tokio::spawn(async move {
        let _ = aikv::server::Server::run_with_listener(listener, state_clone).await;
    });
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert_eq!(state.metrics().connections_total(), 0);
    let mut stream = connect(addr).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(state.metrics().connections_total(), 1);
    assert_eq!(state.metrics().connected_clients(), 1);

    write_cmd(&mut stream, &[b"QUIT"]).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    drop(stream);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(state.metrics().connections_total(), 1);
    assert_eq!(state.metrics().connected_clients(), 0);
}

#[tokio::test]
async fn test_connection_lifecycle_logs() {
    let (addr, _handle) = start_server().await;
    let mut stream = connect(addr).await;
    write_cmd(&mut stream, &[b"PING"]).await;
    let _ = super::helpers::read_response(&mut stream).await;
    drop(stream);
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn test_command_metrics_recorded() {
    use bytes::Bytes;
    use std::sync::Arc;

    use aikv::server::{ConnectionConfig, ServerSharedState};
    use aikv::storage::{KvStorage, MemoryEngine};

    fn b(s: &str) -> Bytes {
        Bytes::copy_from_slice(s.as_bytes())
    }

    let storage: Arc<dyn KvStorage> = MemoryEngine::new(16);
    let shared = ServerSharedState::new(ConnectionConfig::default(), storage, 6379);
    let router = shared.router();
    let mut db = 0;

    router
        .execute("GET", &[b("missing")], &mut db)
        .await
        .unwrap();
    router
        .execute("SET", &[b("k"), b("v")], &mut db)
        .await
        .unwrap();
    router.execute("GET", &[b("k")], &mut db).await.unwrap();
    router
        .execute("INFO", &[b("stats")], &mut db)
        .await
        .unwrap();

    assert_eq!(shared.metrics().keyspace_hits(), 1);
    assert_eq!(shared.metrics().keyspace_misses(), 1);
}

/// 验证 Metrics HTTP 端点 /metrics 返回有效的 Prometheus 文本格式。
#[cfg(feature = "monitoring")]
#[tokio::test]
async fn test_metrics_endpoint() {
    use aikv::server::metrics::Metrics;
    use aikv::server::MetricsServer;
    use std::sync::Arc;

    let metrics = Arc::new(Metrics::new().expect("metrics registration"));
    // 先触发一些指标数据
    metrics.kv_connections_total.inc();
    metrics
        .kv_commands_total
        .with_label_values(&["PING", "ok"])
        .inc();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind metrics server");
    let addr = listener.local_addr().unwrap();
    let svc = MetricsServer::from_listener(listener, Arc::clone(&metrics));
    tokio::spawn(async move {
        svc.run().await;
    });
    tokio::time::sleep(Duration::from_millis(50)).await;

    // 原始 HTTP GET /metrics
    let mut stream = tokio::net::TcpStream::connect(addr)
        .await
        .expect("connect metrics server");
    let request = b"GET /metrics HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
    stream.write_all(request).await.unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    let body = String::from_utf8_lossy(&response);

    // 验证 HTTP 状态行
    assert!(body.contains("200 OK"), "expected 200 OK, got: {body}");
    // 验证 Prometheus Content-Type
    assert!(
        body.contains("text/plain; version=0.0.4"),
        "expected prometheus content type, got: {body}"
    );
    // 验证指标存在
    assert!(
        body.contains("aikv_commands_total"),
        "expected aikv_commands_total in body: {body}"
    );
    assert!(
        body.contains("aikv_connections_total"),
        "expected aikv_connections_total in body: {body}"
    );
    // 验证指标值
    assert!(
        body.contains("aikv_connections_total 1"),
        "expected connections_total=1 in body: {body}"
    );

    // 验证 /health
    let mut stream = tokio::net::TcpStream::connect(addr)
        .await
        .expect("connect metrics server");
    let request = b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
    stream.write_all(request).await.unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    let body = String::from_utf8_lossy(&response);
    assert!(body.contains("200 OK"), "health endpoint failed: {body}");
    assert!(body.contains("OK"), "health body unexpected: {body}");

    // 验证 404
    let mut stream = tokio::net::TcpStream::connect(addr)
        .await
        .expect("connect metrics server");
    let request = b"GET /nonexistent HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
    stream.write_all(request).await.unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    let body = String::from_utf8_lossy(&response);
    assert!(body.contains("404 Not Found"), "expected 404, got: {body}");
}
/// 验证关键 tracing span 名称与文档规范一致。
#[test]
fn test_span_naming() {
    // 项目根目录 (cargo test 的 CARGO_MANIFEST_DIR 指向 AiKv)
    let kv_src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let db_src = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("AiDb")
        .join("src");

    // 期望存在的 span 名称: AiDb 引擎核心路径
    let expected_db_spans: &[&str] = &[
        "wal_open",
        "wal_write",
        "wal_flush",
        "wal_sync",
        "wal_rotate",
        "wal_replay",
        "wal_writer_open",
        "wal_writer_open_with_sync",
        "mem_delete",
        "mem_get",
        "mem_search",
        "sst_block_read",
        "sst_block_seek",
        "cmp_pick",
        "cmp_run",
        "cmp_merge",
        "cmp_apply",
    ];

    // 期望存在的 span 名称: AiKv 关键路径
    let expected_kv_spans: &[&str] = &[
        "kv_cluster_route",
        "cmd_cluster_failover",
        "cmd_cluster_info",
        "cmd_cluster_nodes",
        "cmd_cluster_slots",
    ];

    let mut missing = Vec::new();

    for name in expected_db_spans {
        if db_src.exists() {
            let pattern = format!("name = \"{}\"", name);
            let found = walk_grep(&db_src, &pattern).unwrap_or(false);
            if !found {
                missing.push(format!("DbSpan::{name}"));
            }
        }
    }

    for name in expected_kv_spans {
        let pattern = format!("name = \"{}\"", name);
        let found = walk_grep(&kv_src, &pattern).unwrap_or(false);
        if !found {
            missing.push(format!("KvSpan::{name}"));
        }
    }

    assert!(
        missing.is_empty(),
        "missing expected span names: {}",
        missing.join(", ")
    );
}

/// 递归搜索文件中是否包含指定字符串。
fn walk_grep(dir: &std::path::Path, pattern: &str) -> Result<bool, std::io::Error> {
    if !dir.is_dir() {
        return Ok(false);
    }
    let mut stack = vec![dir.to_path_buf()];
    while let Some(cur) = stack.pop() {
        for entry in std::fs::read_dir(&cur)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().map(|e| e == "rs").unwrap_or(false) {
                let content = std::fs::read_to_string(&path)?;
                if content.contains(pattern) {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

#[cfg(feature = "monitoring")]
#[tokio::test]
async fn test_prometheus_metrics_integration() {
    use std::sync::Arc;

    let prom = Arc::new(aikv::server::metrics::Metrics::new().expect("metrics registration"));
    let server_metrics = aikv::server::ServerMetrics::default().with_prometheus(prom.clone());

    server_metrics.on_connect();
    server_metrics.on_command("SET", true);
    server_metrics.on_command("GET", false);

    let families = prom.registry.gather();
    // 验证 aikv_connections_total 递增
    let conns = families
        .iter()
        .find(|f| f.get_name() == "aikv_connections_total")
        .expect("aikv_connections_total not found");
    assert_eq!(conns.get_metric()[0].get_counter().get_value() as u64, 1);

    // 验证 aikv_commands_total 包含 SET/ok 和 GET/error
    let cmds = families
        .iter()
        .find(|f| f.get_name() == "aikv_commands_total")
        .expect("aikv_commands_total not found");
    let set_ok = cmds.get_metric().iter().find(|m| {
        m.get_label().iter().any(|l| l.get_value() == "SET")
            && m.get_label().iter().any(|l| l.get_value() == "ok")
    });
    assert!(set_ok.is_some(), "SET/ok metric not found");
}

#[cfg(feature = "monitoring")]
fn prom_metric_exists(families: &[prometheus::proto::MetricFamily], name: &str) -> bool {
    families.iter().any(|f| f.get_name() == name)
}

#[cfg(feature = "monitoring")]
fn prom_counter(families: &[prometheus::proto::MetricFamily], name: &str) -> u64 {
    families
        .iter()
        .find(|f| f.get_name() == name)
        .and_then(|f| f.get_metric().first())
        .map(|m| m.get_counter().get_value() as u64)
        .unwrap_or(0)
}

#[cfg(feature = "monitoring")]
fn prom_gauge_with_label(
    families: &[prometheus::proto::MetricFamily],
    name: &str,
    label: &str,
    value: &str,
) -> i64 {
    families
        .iter()
        .find(|f| f.get_name() == name)
        .and_then(|f| {
            f.get_metric().iter().find(|m| {
                m.get_label()
                    .iter()
                    .any(|l| l.get_name() == label && l.get_value() == value)
            })
        })
        .map(|m| m.get_gauge().get_value() as i64)
        .unwrap_or(0)
}

#[cfg(feature = "monitoring")]
#[tokio::test]
async fn test_maxclients_rejection_metrics() {
    use std::sync::Arc;

    use aikv::server::{ConnectionConfig, ServerSharedState};
    use aikv::storage::{KvStorage, MemoryEngine};

    let storage: Arc<dyn KvStorage> = MemoryEngine::new(16);
    let state = ServerSharedState::new(
        ConnectionConfig {
            read_timeout: None,
            idle_timeout: None,
            max_clients: 1,
        },
        storage,
        6379,
    );

    assert!(state.try_register_connection());
    assert!(!state.try_register_connection());

    let families = state.prometheus_metrics.registry.gather();
    assert_eq!(
        prom_counter(&families, "aikv_rejected_connections_total"),
        1
    );
}

#[cfg(feature = "monitoring")]
#[tokio::test]
async fn test_runtime_metrics_refresh() {
    use std::sync::Arc;

    use aikv::server::{ConnectionConfig, ServerSharedState};
    use aikv::storage::{KvStorage, MemoryEngine, StorageEngineKind, StorageObservation};
    use bytes::Bytes;

    fn b(s: &str) -> Bytes {
        Bytes::copy_from_slice(s.as_bytes())
    }

    let observation = StorageObservation::new();
    let storage: Arc<dyn KvStorage> =
        MemoryEngine::with_observation(16, Some(Arc::clone(&observation)));
    let state = ServerSharedState::new_with_backup_dir(
        ConnectionConfig::default(),
        storage,
        6379,
        StorageEngineKind::Memory,
        None,
        None,
        9191,
        "127.0.0.1".into(),
        Some(observation),
    );

    let router = state.router();
    let mut db = 0;
    router
        .execute("SET", &[b("alive1"), b("v")], &mut db)
        .await
        .unwrap();
    router
        .execute("SET", &[b("alive2"), b("v")], &mut db)
        .await
        .unwrap();
    router
        .execute("SET", &[b("expired"), b("x"), b("PX"), b("1")], &mut db)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    router
        .execute("GET", &[b("expired")], &mut db)
        .await
        .unwrap();

    state.refresh_runtime_metrics().await;

    let families = state.prometheus_metrics.registry.gather();
    assert!(
        prom_counter(&families, "aikv_expired_keys_total") >= 1,
        "expected expired key counter"
    );
    assert_eq!(
        prom_gauge_with_label(&families, "aikv_db_keys", "db", "0"),
        2
    );
    assert!(
        state.prometheus_metrics.kv_used_memory_bytes.get() > 0,
        "expected non-zero memory gauge"
    );
    assert_eq!(
        state.prometheus_metrics.kv_blocked_clients.get(),
        0,
        "blocked_clients should default to 0"
    );
    assert!(
        prom_metric_exists(&families, "aikv_instantaneous_ops_per_sec"),
        "expected instantaneous_ops gauge registered"
    );
    assert!(
        prom_metric_exists(&families, "aikv_evicted_keys_total"),
        "expected evicted_keys counter registered"
    );
}

#[cfg(all(feature = "monitoring", target_os = "linux"))]
#[tokio::test]
async fn process_resident_memory_bytes_exposed_on_metrics_endpoint() {
    use std::sync::Arc;

    use aikv::server::metrics::Metrics;
    use aikv::server::{MetricsServer, ServerMetrics};

    let prom = Arc::new(Metrics::new().expect("metrics registration"));
    let server_metrics = ServerMetrics::default().with_prometheus(Arc::clone(&prom));
    server_metrics.refresh_process_metrics();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind metrics server");
    let addr = listener.local_addr().unwrap();
    let svc = MetricsServer::from_listener(listener, prom);
    tokio::spawn(async move {
        svc.run().await;
    });
    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut stream = tokio::net::TcpStream::connect(addr)
        .await
        .expect("connect metrics server");
    let request = b"GET /metrics HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
    stream.write_all(request).await.unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    let body = String::from_utf8_lossy(&response);

    assert!(
        body.contains("aikv_process_resident_memory_bytes"),
        "expected aikv_process_resident_memory_bytes in body: {body}"
    );

    let value = body
        .lines()
        .find(|line| line.starts_with("aikv_process_resident_memory_bytes "))
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);
    assert!(value > 0, "expected RSS gauge > 0, got {value}");
}

#[cfg(feature = "monitoring")]
#[tokio::test]
async fn test_net_bytes_metrics() {
    use std::sync::Arc;

    use aikv::server::{ConnectionConfig, Server, ServerSharedState};
    use aikv::storage::{KvStorage, MemoryEngine};

    let storage: Arc<dyn KvStorage> = MemoryEngine::new(16);
    let state = ServerSharedState::new(
        ConnectionConfig {
            read_timeout: None,
            idle_timeout: None,
            max_clients: 0,
        },
        storage,
        0,
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state_clone = Arc::clone(&state);
    tokio::spawn(async move {
        let _ = Server::run_with_listener(listener, state_clone).await;
    });
    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut stream = connect(addr).await;
    write_cmd(&mut stream, &[b"PING"]).await;
    let _ = super::helpers::read_response(&mut stream).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert!(
        state.metrics().net_input_bytes() > 0,
        "expected net input bytes on ServerMetrics"
    );
    assert!(
        state.metrics().net_output_bytes() > 0,
        "expected net output bytes on ServerMetrics"
    );

    write_cmd(&mut stream, &[b"INFO", b"stats"]).await;
    let info_resp = super::helpers::read_response(&mut stream).await;
    let info_text = String::from_utf8_lossy(&info_resp);
    assert!(
        info_text.contains("total_net_input_bytes:"),
        "INFO stats missing total_net_input_bytes: {info_text}"
    );
    assert!(
        info_text.contains("total_net_output_bytes:"),
        "INFO stats missing total_net_output_bytes: {info_text}"
    );

    let families = state.prometheus_metrics.registry.gather();
    assert!(
        prom_counter(&families, "aikv_net_input_bytes_total") > 0,
        "expected net input bytes"
    );
    assert!(
        prom_counter(&families, "aikv_net_output_bytes_total") > 0,
        "expected net output bytes"
    );
}

#[cfg(feature = "monitoring")]
fn prom_gauge(families: &[prometheus::proto::MetricFamily], name: &str) -> i64 {
    families
        .iter()
        .find(|f| f.get_name() == name)
        .and_then(|f| f.get_metric().first())
        .map(|m| m.get_gauge().get_value() as i64)
        .unwrap_or(0)
}

#[cfg(feature = "monitoring")]
fn info_field_u64(text: &str, key: &str) -> Option<u64> {
    text.lines().find_map(|line| {
        let (k, v) = line.split_once(':')?;
        (k == key).then(|| v.parse().ok())?
    })
}

/// 不变式 I1/I2: 同一进程 INFO 与 /metrics 数值一致.
#[cfg(feature = "monitoring")]
#[tokio::test]
async fn info_metrics_consistency_after_commands() {
    use std::sync::Arc;

    use aikv::server::{ConnectionConfig, ServerSharedState};
    use aikv::storage::{KvStorage, MemoryEngine, StorageEngineKind, StorageObservation};
    use bytes::Bytes;

    fn b(s: &str) -> Bytes {
        Bytes::copy_from_slice(s.as_bytes())
    }

    let observation = StorageObservation::new();
    let storage: Arc<dyn KvStorage> =
        MemoryEngine::with_observation(16, Some(Arc::clone(&observation)));
    let state = ServerSharedState::new_with_backup_dir(
        ConnectionConfig::default(),
        storage,
        6379,
        StorageEngineKind::Memory,
        None,
        None,
        9191,
        "127.0.0.1".into(),
        Some(observation),
    );

    let router = state.router();
    let mut db = 0;
    router
        .execute("GET", &[b("missing")], &mut db)
        .await
        .unwrap();
    router
        .execute("SET", &[b("k"), b("v")], &mut db)
        .await
        .unwrap();
    router.execute("GET", &[b("k")], &mut db).await.unwrap();

    state.refresh_runtime_metrics().await;

    let memory_text = {
        let resp = router
            .execute("INFO", &[b("memory")], &mut db)
            .await
            .unwrap();
        let aikv::protocol::RespValue::BulkString(Some(text)) = resp else {
            panic!("expected bulk string");
        };
        String::from_utf8_lossy(&text).into_owned()
    };
    let stats_text = {
        let resp = router
            .execute("INFO", &[b("stats")], &mut db)
            .await
            .unwrap();
        let aikv::protocol::RespValue::BulkString(Some(text)) = resp else {
            panic!("expected bulk string");
        };
        String::from_utf8_lossy(&text).into_owned()
    };
    let clients_text = {
        let resp = router
            .execute("INFO", &[b("clients")], &mut db)
            .await
            .unwrap();
        let aikv::protocol::RespValue::BulkString(Some(text)) = resp else {
            panic!("expected bulk string");
        };
        String::from_utf8_lossy(&text).into_owned()
    };

    let families = state.prometheus_metrics.registry.gather();
    let used_memory = state.metrics().used_memory_bytes();
    assert_eq!(
        info_field_u64(&memory_text, "used_memory"),
        Some(used_memory)
    );
    assert_eq!(
        prom_gauge(&families, "aikv_used_memory_bytes"),
        used_memory as i64
    );

    let hits = state.metrics().keyspace_hits();
    let misses = state.metrics().keyspace_misses();
    assert_eq!(info_field_u64(&stats_text, "keyspace_hits"), Some(hits));
    assert_eq!(info_field_u64(&stats_text, "keyspace_misses"), Some(misses));
    assert_eq!(prom_counter(&families, "aikv_keyspace_hits_total"), hits);
    assert_eq!(
        prom_counter(&families, "aikv_keyspace_misses_total"),
        misses
    );

    let blocked = state.metrics().blocked_clients() as u64;
    assert_eq!(
        info_field_u64(&clients_text, "blocked_clients"),
        Some(blocked)
    );
    assert_eq!(
        prom_gauge(&families, "aikv_blocked_clients"),
        blocked as i64
    );

    let ops = state.metrics().instantaneous_ops_per_sec();
    assert_eq!(
        info_field_u64(&stats_text, "instantaneous_ops_per_sec"),
        Some(ops)
    );
    assert_eq!(
        prom_gauge(&families, "aikv_instantaneous_ops_per_sec"),
        ops as i64
    );
}

#[tokio::test]
async fn blocked_clients_increments_during_blpop() {
    use bytes::Bytes;
    use std::sync::Arc;

    use aikv::server::{ConnectionConfig, ServerSharedState};
    use aikv::storage::{KvStorage, MemoryEngine};

    fn b(s: &str) -> Bytes {
        Bytes::copy_from_slice(s.as_bytes())
    }

    let storage: Arc<dyn KvStorage> = MemoryEngine::new(16);
    let shared = Arc::new(ServerSharedState::new(
        ConnectionConfig::default(),
        storage,
        6379,
    ));
    let metrics = shared.metrics();
    let router = shared.router();

    assert_eq!(metrics.blocked_clients(), 0);

    let router_wait = shared.router();
    let wait = tokio::spawn(async move {
        let mut db = 0;
        router_wait
            .execute("BLPOP", &[b("q"), b("5")], &mut db)
            .await
    });

    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(metrics.blocked_clients(), 1);

    let mut db = 0;
    router
        .execute("LPUSH", &[b("q"), b("v")], &mut db)
        .await
        .unwrap();

    wait.await.unwrap().unwrap();
    assert_eq!(metrics.blocked_clients(), 0);
}

#[tokio::test]
async fn blocked_clients_multi_key_blpop_counts_one_client() {
    use bytes::Bytes;
    use std::sync::Arc;

    use aikv::server::{ConnectionConfig, ServerSharedState};
    use aikv::storage::{KvStorage, MemoryEngine};

    fn b(s: &str) -> Bytes {
        Bytes::copy_from_slice(s.as_bytes())
    }

    let storage: Arc<dyn KvStorage> = MemoryEngine::new(16);
    let shared = Arc::new(ServerSharedState::new(
        ConnectionConfig::default(),
        storage,
        6379,
    ));
    let metrics = shared.metrics();
    let router_wait = shared.router();
    let wait = tokio::spawn(async move {
        let mut db = 0;
        router_wait
            .execute("BLPOP", &[b("a"), b("b"), b("c"), b("5")], &mut db)
            .await
    });

    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(metrics.blocked_clients(), 1);

    let router = shared.router();
    let mut db = 0;
    router
        .execute("LPUSH", &[b("b"), b("v")], &mut db)
        .await
        .unwrap();

    wait.await.unwrap().unwrap();
    assert_eq!(metrics.blocked_clients(), 0);
}

#[tokio::test]
async fn blocked_clients_zero_timeout_does_not_block() {
    use bytes::Bytes;
    use std::sync::Arc;

    use aikv::server::{ConnectionConfig, ServerSharedState};
    use aikv::storage::{KvStorage, MemoryEngine};

    fn b(s: &str) -> Bytes {
        Bytes::copy_from_slice(s.as_bytes())
    }

    let storage: Arc<dyn KvStorage> = MemoryEngine::new(16);
    let shared = ServerSharedState::new(ConnectionConfig::default(), storage, 6379);
    let router = shared.router();
    let mut db = 0;

    router
        .execute("BLPOP", &[b("missing"), b("0")], &mut db)
        .await
        .unwrap();
    assert_eq!(shared.metrics().blocked_clients(), 0);
}

/// 契约: 核心 aikv_* 指标名与 observability-reference 一致.
#[cfg(feature = "monitoring")]
#[test]
fn test_metric_catalog_contract() {
    use std::sync::Arc;

    use aikv::server::metrics::{Metrics, ServerMetrics};
    use aikv::server::otel_metrics::AIKV_METRIC_NAMES;

    let metrics = Arc::new(Metrics::new().expect("metrics registration"));
    let server_metrics = ServerMetrics::default().with_prometheus(metrics.clone());
    server_metrics.on_connect();
    server_metrics.on_command("PING", true);
    server_metrics.on_command_duration("PING", 1000, true);
    server_metrics.set_db_key_count(0, 0);

    let families = metrics.registry.gather();
    for name in AIKV_METRIC_NAMES {
        assert!(
            families.iter().any(|f| f.get_name() == *name),
            "missing prometheus metric: {name}"
        );
    }
}

/// 契约: OTel 双写后 counter 与 prometheus 对齐.
#[cfg(feature = "monitoring")]
#[test]
fn test_otel_prometheus_counter_parity() {
    use std::sync::Arc;

    use aikv::server::metrics::{Metrics, ServerMetrics};

    let prom = Arc::new(Metrics::new().expect("metrics registration"));
    let server_metrics = ServerMetrics::default().with_prometheus(prom.clone());
    server_metrics.on_connect();
    server_metrics.on_keyspace_hit();

    let families = prom.registry.gather();
    let conns = families
        .iter()
        .find(|f| f.get_name() == "aikv_connections_total")
        .expect("aikv_connections_total")
        .get_metric()[0]
        .get_counter()
        .get_value();
    assert_eq!(conns, 1.0);

    let hits = families
        .iter()
        .find(|f| f.get_name() == "aikv_keyspace_hits_total")
        .expect("aikv_keyspace_hits_total")
        .get_metric()[0]
        .get_counter()
        .get_value();
    assert_eq!(hits, 1.0);
}
