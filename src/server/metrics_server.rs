//! Metrics HTTP 服务器: 在独立端口暴露 /metrics 端点。
//! 仅在 `--features monitoring` 下编译。

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use http_body_util::Full;
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use prometheus::{Encoder, TextEncoder};
use tokio::net::TcpListener;

use super::metrics::Metrics;

pub struct MetricsServer {
    addr: SocketAddr,
    metrics: Arc<Metrics>,
    listener: Option<tokio::net::TcpListener>,
}

impl MetricsServer {
    pub fn new(addr: SocketAddr, metrics: Arc<Metrics>) -> Self {
        Self {
            addr,
            metrics,
            listener: None,
        }
    }

    /// 使用已绑定的 TcpListener 创建 MetricsServer（用于测试，避免端口冲突）。
    pub fn from_listener(listener: tokio::net::TcpListener, metrics: Arc<Metrics>) -> Self {
        let addr = listener.local_addr().unwrap();
        Self {
            addr,
            metrics,
            listener: Some(listener),
        }
    }

    pub async fn run(self) {
        let listener = match self.listener {
            Some(l) => l,
            None => match TcpListener::bind(self.addr).await {
                Ok(l) => l,
                Err(e) => {
                    tracing::error!(error = %e, addr = %self.addr, "failed to bind metrics server");
                    return;
                }
            },
        };
        tracing::info!("Metrics server listening on http://{}", self.addr);

        loop {
            let (stream, _) = match listener.accept().await {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(error = %e, "metrics server accept error");
                    continue;
                }
            };
            let io = TokioIo::new(stream);
            let m = Arc::clone(&self.metrics);
            let svc = service_fn(move |req| handle_request(req, m.clone()));
            if let Err(e) = http1::Builder::new().serve_connection(io, svc).await {
                tracing::warn!(error = %e, "metrics server connection error");
            }
        }
    }
}

async fn handle_request(
    req: Request<hyper::body::Incoming>,
    metrics: Arc<Metrics>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    match req.uri().path() {
        "/metrics" => {
            let encoder = TextEncoder::new();
            let metric_families = metrics.registry.gather();
            let mut buffer = vec![];
            if encoder.encode(&metric_families, &mut buffer).is_err() {
                return Ok(Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(Full::new(Bytes::from("encoding error")))
                    .unwrap());
            }
            Ok(Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "text/plain; version=0.0.4")
                .body(Full::new(Bytes::from(buffer)))
                .unwrap())
        }
        "/health" => Ok(Response::builder()
            .status(StatusCode::OK)
            .body(Full::new(Bytes::from("OK")))
            .unwrap()),
        "/" => {
            let body = r#"<!DOCTYPE html>
<html>
<head><title>AiKv Metrics</title></head>
<body>
<h1>AiKv Metrics Server</h1>
<ul>
<li><a href="/metrics">Metrics</a> - Prometheus metrics endpoint</li>
<li><a href="/health">Health</a> - Health check endpoint</li>
</ul>
</body>
</html>"#;
            Ok(Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "text/html")
                .body(Full::new(Bytes::from(body)))
                .unwrap())
        }
        _ => Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Full::new(Bytes::from("Not Found")))
            .unwrap()),
    }
}
