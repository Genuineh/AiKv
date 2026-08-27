//! OTel 指标测试辅助 (InMemory exporter).

use std::cell::RefCell;
use std::sync::{Arc, Mutex};

use opentelemetry::global;
use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData};
use opentelemetry_sdk::metrics::{InMemoryMetricExporter, SdkMeterProvider};

use super::OtelMetrics;

static TEST_INIT_LOCK: Mutex<()> = Mutex::new(());

thread_local! {
    static TEST_PROVIDER: RefCell<Option<Arc<SdkMeterProvider>>> = const { RefCell::new(None) };
}

/// 每个测试独立 meter provider + 清空 sync 快照 (thread-local provider 避免并行 flush 串台).
pub fn init_in_memory() -> (InMemoryMetricExporter, Arc<OtelMetrics>) {
    let _guard = TEST_INIT_LOCK.lock();
    let exporter = InMemoryMetricExporter::default();
    let provider = SdkMeterProvider::builder()
        .with_periodic_exporter(exporter.clone())
        .build();
    global::set_meter_provider(provider.clone());
    let provider = Arc::new(provider);
    TEST_PROVIDER.with(|cell| *cell.borrow_mut() = Some(Arc::clone(&provider)));
    let otel = OtelMetrics::install_global(global::meter("aikv"));
    otel.reset_sync_snapshot_for_test();
    (exporter, otel)
}

fn flush() {
    TEST_PROVIDER.with(|cell| {
        if let Some(provider) = cell.borrow().as_ref() {
            provider.force_flush().unwrap();
        }
    });
}

pub fn counter_sum(exporter: &InMemoryMetricExporter, name: &str) -> u64 {
    flush();
    let metrics = exporter.get_finished_metrics().unwrap();
    use std::collections::HashMap;
    // Latest cumulative value per attribute set (handles periodic re-export).
    let mut by_attrs: HashMap<Vec<(String, String)>, u64> = HashMap::new();
    for rm in &metrics {
        for sm in rm.scope_metrics() {
            for m in sm.metrics() {
                if m.name() != name {
                    continue;
                }
                if let AggregatedMetrics::U64(MetricData::Sum(sum)) = m.data() {
                    for dp in sum.data_points() {
                        let key: Vec<(String, String)> = dp
                            .attributes()
                            .map(|kv| (kv.key.to_string(), kv.value.to_string()))
                            .collect();
                        by_attrs.insert(key, dp.value());
                    }
                }
            }
        }
    }
    by_attrs.values().sum()
}

pub fn gauge_value(exporter: &InMemoryMetricExporter, name: &str) -> f64 {
    observable_gauge_value(exporter, name)
}

pub fn observable_gauge_value(exporter: &InMemoryMetricExporter, name: &str) -> f64 {
    flush();
    let metrics = exporter.get_finished_metrics().unwrap();
    let mut best = 0.0f64;
    for rm in &metrics {
        for sm in rm.scope_metrics() {
            for m in sm.metrics() {
                if m.name() != name {
                    continue;
                }
                if let AggregatedMetrics::F64(MetricData::Gauge(g)) = m.data() {
                    for dp in g.data_points() {
                        best = best.max(dp.value());
                    }
                }
                if let AggregatedMetrics::I64(MetricData::Gauge(g)) = m.data() {
                    for dp in g.data_points() {
                        best = best.max(dp.value() as f64);
                    }
                }
            }
        }
    }
    best
}

pub fn metric_exists(exporter: &InMemoryMetricExporter, name: &str) -> bool {
    flush();
    let metrics = exporter.get_finished_metrics().unwrap();
    metrics.iter().any(|rm| {
        rm.scope_metrics()
            .any(|sm| sm.metrics().any(|m| m.name() == name))
    })
}
