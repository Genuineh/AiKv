//! OTel traces + metrics 初始化 (monitoring feature).

use std::time::Duration;

use opentelemetry::global;
use opentelemetry::KeyValue;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};
use opentelemetry_sdk::trace::SdkTracerProvider;
use opentelemetry_sdk::Resource;

/// OTel 初始化参数 (Resource 标签).
#[derive(Debug, Clone, Default)]
pub struct OtelInitConfig {
    pub endpoint: String,
    pub host_label: Option<String>,
    pub node_id: Option<u64>,
}

fn build_resource(config: &OtelInitConfig) -> Resource {
    let mut builder = Resource::builder()
        .with_service_name("aikv")
        .with_attribute(KeyValue::new(
            "service.version",
            env!("CARGO_PKG_VERSION"),
        ));
    if let Some(host) = &config.host_label {
        builder = builder.with_attribute(KeyValue::new("host.name", host.clone()));
    }
    if let Some(node_id) = config.node_id {
        builder = builder.with_attribute(KeyValue::new("node_id", node_id.to_string()));
    }
    builder.build()
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

/// 初始化 OTel Tracer + Meter (15s metrics export). 成功返回 true.
pub fn init_otel(config: &OtelInitConfig) -> bool {
    let resource = build_resource(config);
    let endpoint = config.endpoint.clone();

    let span_exporter = match opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint.clone())
        .build()
    {
        Ok(exporter) => exporter,
        Err(e) => {
            eprintln!("warn: OTel trace exporter build failed: {e}");
            return false;
        }
    };

    let tracer_provider = SdkTracerProvider::builder()
        .with_batch_exporter(span_exporter)
        .with_resource(resource.clone())
        .build();
    let _ = global::set_tracer_provider(tracer_provider.clone());
    let _ = Box::leak(Box::new(tracer_provider));

    let metric_exporter = match opentelemetry_otlp::MetricExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint.clone())
        .build()
    {
        Ok(exporter) => exporter,
        Err(e) => {
            eprintln!("warn: OTel metrics exporter build failed: {e}");
            return true;
        }
    };

    let reader = PeriodicReader::builder(metric_exporter)
        .with_interval(Duration::from_secs(15))
        .build();
    let meter_provider = SdkMeterProvider::builder()
        .with_reader(reader)
        .with_resource(resource)
        .build();
    let _ = global::set_meter_provider(meter_provider.clone());
    let _ = Box::leak(Box::new(meter_provider));

    aidb::metrics::init();
    eprintln!("info: OTel traces+metrics initialized (endpoint={endpoint})");
    true
}
