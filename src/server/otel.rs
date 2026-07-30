//! OTel traces + metrics 初始化 (monitoring feature).

use std::sync::OnceLock;
use std::time::Duration;

use opentelemetry::global;
use opentelemetry::KeyValue;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};
use opentelemetry_sdk::trace::SdkTracerProvider;
use opentelemetry_sdk::Resource;

static TRACER_PROVIDER: OnceLock<SdkTracerProvider> = OnceLock::new();
static METER_PROVIDER: OnceLock<SdkMeterProvider> = OnceLock::new();

/// OTel 初始化参数 (Resource 标签).
#[derive(Debug, Clone)]
pub struct OtelInitConfig {
    pub endpoint: String,
    pub service_name: String,
    pub host_label: Option<String>,
    pub node_id: Option<u64>,
    pub tcp_port: u16,
    pub deployment_environment: Option<String>,
    pub extra_resource_attrs: Vec<KeyValue>,
}

fn env_nonempty(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

/// 解析 `OTEL_RESOURCE_ATTRIBUTES` (key=value,key2=value2).
fn parse_resource_attributes(raw: &str) -> Vec<KeyValue> {
    raw.split(',')
        .filter_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            let k = k.trim();
            let v = v.trim();
            if k.is_empty() {
                None
            } else {
                Some(KeyValue::new(k.to_string(), v.to_string()))
            }
        })
        .collect()
}

fn service_instance_id(config: &OtelInitConfig) -> String {
    if let Some(node_id) = config.node_id {
        return node_id.to_string();
    }
    let host = config
        .host_label
        .as_deref()
        .filter(|h| !h.is_empty())
        .unwrap_or("localhost");
    format!("{host}:{}", config.tcp_port)
}

fn build_resource(config: &OtelInitConfig) -> Resource {
    let instance_id = service_instance_id(config);
    let mut builder = Resource::builder()
        .with_service_name(config.service_name.clone())
        .with_attribute(KeyValue::new("service.version", env!("CARGO_PKG_VERSION")))
        .with_attribute(KeyValue::new("service.instance.id", instance_id.clone()));
    if let Some(host) = &config.host_label {
        builder = builder.with_attribute(KeyValue::new("host.name", host.clone()));
    }
    if let Some(node_id) = config.node_id {
        builder = builder.with_attribute(KeyValue::new("node_id", node_id.to_string()));
    }
    if let Some(env) = &config.deployment_environment {
        builder = builder.with_attribute(KeyValue::new("deployment.environment", env.clone()));
    }
    for kv in &config.extra_resource_attrs {
        builder = builder.with_attribute(kv.clone());
    }
    builder.build()
}

/// 从环境变量与 CLI 解析 OTel 配置.
pub fn otel_config_from_env(
    tcp_port: u16,
    #[cfg(feature = "cluster")] cluster_node_id: Option<u64>,
    #[cfg(not(feature = "cluster"))] cluster_node_id: Option<u64>,
) -> Option<OtelInitConfig> {
    let endpoint = env_nonempty("OTEL_EXPORTER_OTLP_ENDPOINT")
        .or_else(|| env_nonempty("AIKV_OTLP_ENDPOINT"))?;
    let service_name = env_nonempty("OTEL_SERVICE_NAME").unwrap_or_else(|| "aikv".to_string());
    let host_label = env_nonempty("AIKV_HOST_LABEL");
    let node_id = env_nonempty("AIKV_NODE_ID")
        .and_then(|v| v.parse::<u64>().ok())
        .or(cluster_node_id);
    let deployment_environment = env_nonempty("OTEL_DEPLOYMENT_ENVIRONMENT")
        .or_else(|| env_nonempty("AIKV_DEPLOYMENT_ENV"))
        .or_else(|| Some("dev".to_string()));
    let extra_resource_attrs = env_nonempty("OTEL_RESOURCE_ATTRIBUTES")
        .map(|raw| parse_resource_attributes(&raw))
        .unwrap_or_default();
    Some(OtelInitConfig {
        endpoint,
        service_name,
        host_label,
        node_id,
        tcp_port,
        deployment_environment,
        extra_resource_attrs,
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
    global::set_tracer_provider(tracer_provider.clone());
    let _ = TRACER_PROVIDER.set(tracer_provider);

    let metric_exporter = match opentelemetry_otlp::MetricExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint.clone())
        .build()
    {
        Ok(exporter) => exporter,
        Err(e) => {
            eprintln!("warn: OTel metrics exporter build failed: {e}");
            if let Some(tp) = TRACER_PROVIDER.get() {
                let _ = tp.shutdown();
            }
            return false;
        }
    };

    let reader = PeriodicReader::builder(metric_exporter)
        .with_interval(Duration::from_secs(15))
        .build();
    let meter_provider = SdkMeterProvider::builder()
        .with_reader(reader)
        .with_resource(resource)
        .build();
    global::set_meter_provider(meter_provider.clone());
    let _ = METER_PROVIDER.set(meter_provider);

    aidb::metrics::init();
    eprintln!("info: OTel traces+metrics initialized (endpoint={endpoint})");
    true
}

/// 进程退出前 flush traces/metrics (SHUTDOWN 或正常退出).
pub fn shutdown_otel() {
    if let Some(tp) = TRACER_PROVIDER.get() {
        let _ = tp.shutdown();
    }
    if let Some(mp) = METER_PROVIDER.get() {
        let _ = mp.shutdown();
    }
}
