//! OTel traces + metrics 初始化 (monitoring feature).

use std::time::Duration;

use opentelemetry::KeyValue;
use opentelemetry_sdk::Resource;

/// OTel 初始化参数 (Resource 标签).
#[derive(Debug, Clone, Default)]
pub struct OtelInitConfig {
    pub endpoint: String,
    pub host_label: Option<String>,
    pub node_id: Option<u64>,
}

fn build_resource(config: &OtelInitConfig) -> Resource {
    let mut kvs = vec![
        KeyValue::new("service.name", "aikv"),
        KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
    ];
    if let Some(host) = &config.host_label {
        kvs.push(KeyValue::new("host.name", host.clone()));
    }
    if let Some(node_id) = config.node_id {
        kvs.push(KeyValue::new("node_id", node_id.to_string()));
    }
    Resource::new(kvs)
}

/// 从环境变量与 CLI 解析 OTel 配置.
pub fn otel_config_from_env(
    #[cfg(feature = "cluster")] cluster_node_id: Option<u64>,
    #[cfg(not(feature = "cluster"))] cluster_node_id: Option<u64>,
) -> Option<OtelInitConfig> {
    let endpoint = std::env::var("AIKV_OTLP_ENDPOINT")
        .ok()
        .filter(|v| !v.is_empty())?;
    let host_label = std::env::var("AIKV_HOST_LABEL")
        .ok()
        .filter(|v| !v.is_empty());
    let node_id = std::env::var("AIKV_NODE_ID")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .or(cluster_node_id);
    Some(OtelInitConfig {
        endpoint,
        host_label,
        node_id,
    })
}

/// 将 OTel trace/span id 写入 tracing span 字段, JSON 日志可提取.
pub fn record_trace_ids_on_span(span: &tracing::Span) {
    use opentelemetry::trace::TraceContextExt;
    use tracing_opentelemetry::OpenTelemetrySpanExt;

    let cx = span.context();
    let otel_span = cx.span();
    let sc = otel_span.span_context();
    if sc.is_valid() {
        span.record("trace_id", tracing::field::display(sc.trace_id()));
        span.record("span_id", tracing::field::display(sc.span_id()));
    }
}

/// 初始化 OTel Tracer + Meter (15s metrics export). 失败返回 None.
pub fn init_otel(config: &OtelInitConfig) -> Option<opentelemetry_sdk::trace::Tracer> {
    use opentelemetry_otlp::WithExportConfig;
    use opentelemetry_sdk::trace;

    let resource = build_resource(config);
    let endpoint = config.endpoint.clone();

    let trace_exporter = opentelemetry_otlp::new_exporter()
        .tonic()
        .with_endpoint(endpoint.clone());
    let tracer = match opentelemetry_otlp::new_pipeline()
        .tracing()
        .with_exporter(trace_exporter)
        .with_trace_config(trace::config().with_resource(resource.clone()))
        .install_batch(opentelemetry_sdk::runtime::Tokio)
    {
        Ok(tracer) => tracer,
        Err(e) => {
            eprintln!("warn: OTel trace initialization failed: {e}");
            return None;
        }
    };

    let meter_exporter = opentelemetry_otlp::new_exporter()
        .tonic()
        .with_endpoint(endpoint.clone());
    match opentelemetry_otlp::new_pipeline()
        .metrics(opentelemetry_sdk::runtime::Tokio)
        .with_exporter(meter_exporter)
        .with_resource(resource)
        .with_period(Duration::from_secs(15))
        .build()
    {
        Ok(provider) => {
            let _ = Box::leak(Box::new(provider));
            eprintln!("info: OTel traces+metrics initialized (endpoint={endpoint})");
            Some(tracer)
        }
        Err(e) => {
            eprintln!("warn: OTel metrics initialization failed: {e}");
            Some(tracer)
        }
    }
}
