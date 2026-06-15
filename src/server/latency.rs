//! 命令延迟统计 (Phase 10-ext Step 0, 内存版)

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

const MAX_LATENCY_HISTORY: usize = 128;

/// 直方图桶上界 (微秒), 与 Redis latency-tracking 对齐
pub const LATENCY_BUCKET_BOUNDS: &[u64] = &[
  1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1_024, 2_048, 4_096, 8_192, 16_384, 32_768, 65_536,
  131_072, 262_144, 524_288, 1_048_576,
];

#[derive(Debug)]
struct CommandLatency {
  calls: AtomicU64,
  total_us: AtomicU64,
  max_us: AtomicU64,
  buckets: Vec<AtomicU64>,
  /// 历史样本 (timestamp_s, duration_ms), 最多保留 MAX_LATENCY_HISTORY 条
  history: Mutex<VecDeque<(u64, u64)>>,
}

impl CommandLatency {
  fn new() -> Self {
    Self {
      calls: AtomicU64::new(0),
      total_us: AtomicU64::new(0),
      max_us: AtomicU64::new(0),
      buckets: LATENCY_BUCKET_BOUNDS
        .iter()
        .map(|_| AtomicU64::new(0))
        .collect(),
      history: Mutex::new(VecDeque::new()),
    }
  }

  fn record(&self, usec: u64) {
    self.calls.fetch_add(1, Ordering::Relaxed);
    self.total_us.fetch_add(usec, Ordering::Relaxed);
    loop {
      let current = self.max_us.load(Ordering::Relaxed);
      if usec <= current {
        break;
      }
      if self
        .max_us
        .compare_exchange(current, usec, Ordering::Relaxed, Ordering::Relaxed)
        .is_ok()
      {
        break;
      }
    }
    for (i, &bound) in LATENCY_BUCKET_BOUNDS.iter().enumerate() {
      if usec <= bound {
        self.buckets[i].fetch_add(1, Ordering::Relaxed);
      }
    }
    if let Some(&last_bound) = LATENCY_BUCKET_BOUNDS.last() {
      if usec > last_bound {
        let last_idx = LATENCY_BUCKET_BOUNDS.len() - 1;
        self.buckets[last_idx].fetch_add(1, Ordering::Relaxed);
      }
    }
    let ts_s = std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .unwrap_or_default()
      .as_secs();
    let mut hist = self.history.lock().unwrap();
    if hist.len() >= MAX_LATENCY_HISTORY {
      hist.pop_front();
    }
    hist.push_back((ts_s, usec / 1000));
  }

  fn snapshot(&self) -> CommandLatencySnapshot {
    let calls = self.calls.load(Ordering::Relaxed);
    let total_us = self.total_us.load(Ordering::Relaxed);
    let max_us = self.max_us.load(Ordering::Relaxed);
    let buckets = LATENCY_BUCKET_BOUNDS
      .iter()
      .zip(self.buckets.iter())
      .filter_map(|(&bound, counter)| {
        let count = counter.load(Ordering::Relaxed);
        if count > 0 {
          Some((bound, count))
        } else {
          None
        }
      })
      .collect();
    CommandLatencySnapshot {
      calls,
      total_us,
      max_us,
      buckets,
      p50_us: percentile_from_buckets(&self.buckets, 0.50),
      p95_us: percentile_from_buckets(&self.buckets, 0.95),
      p99_us: percentile_from_buckets(&self.buckets, 0.99),
      p999_us: percentile_from_buckets(&self.buckets, 0.999),
    }
  }

  fn history_snapshot(&self) -> Vec<(u64, u64)> {
    self.history.lock().unwrap().iter().cloned().collect()
  }

  fn reset(&self) {
    self.calls.store(0, Ordering::Relaxed);
    self.total_us.store(0, Ordering::Relaxed);
    self.max_us.store(0, Ordering::Relaxed);
    for bucket in &self.buckets {
      bucket.store(0, Ordering::Relaxed);
    }
    self.history.lock().unwrap().clear();
  }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandLatencySnapshot {
  pub calls: u64,
  pub total_us: u64,
  pub max_us: u64,
  pub buckets: Vec<(u64, u64)>,
  pub p50_us: u64,
  pub p95_us: u64,
  pub p99_us: u64,
  pub p999_us: u64,
}

#[derive(Debug, Default)]
pub struct LatencyStats {
  by_command: Mutex<HashMap<String, CommandLatency>>,
}

impl LatencyStats {
  pub fn record(&self, command: &str, duration_us: u64) {
    let key = command.to_ascii_uppercase();
    let mut map = self.by_command.lock().unwrap();
    map
      .entry(key)
      .or_insert_with(CommandLatency::new)
      .record(duration_us);
  }

  pub fn snapshot(&self, command: &str) -> Option<CommandLatencySnapshot> {
    let map = self.by_command.lock().unwrap();
    map.get(&command.to_ascii_uppercase()).map(|c| c.snapshot())
  }

  pub fn histogram_snapshots(
    &self,
    filter: Option<&[&str]>,
  ) -> Vec<(String, CommandLatencySnapshot)> {
    let map = self.by_command.lock().unwrap();
    let filter_upper: Option<Vec<String>> =
      filter.map(|names| names.iter().map(|n| n.to_ascii_uppercase()).collect());
    let mut out = Vec::new();
    for (cmd, stats) in map.iter() {
      if let Some(ref names) = filter_upper {
        if !names.contains(cmd) {
          continue;
        }
      }
      let snap = stats.snapshot();
      if snap.calls == 0 {
        continue;
      }
      out.push((cmd.to_ascii_lowercase(), snap));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
  }

  pub fn history(&self, command: &str) -> Vec<(u64, u64)> {
    let map = self.by_command.lock().unwrap();
    match map.get(&command.to_ascii_uppercase()) {
      None => vec![],
      Some(lat) => lat.history_snapshot(),
    }
  }

  pub fn reset(&self, filter: Option<&[&str]>) -> u64 {
    let mut map = self.by_command.lock().unwrap();
    match filter {
      None => {
        let count = map.len() as u64;
        map.clear();
        count
      }
      Some(names) => {
        let mut count = 0u64;
        for name in names {
          let key = name.to_ascii_uppercase();
          if let Some(stats) = map.get(&key) {
            stats.reset();
            count += 1;
          }
        }
        count
      }
    }
  }
}

fn percentile_from_buckets(buckets: &[AtomicU64], quantile: f64) -> u64 {
  let total = buckets
    .last()
    .map(|b| b.load(Ordering::Relaxed))
    .unwrap_or(0);
  if total == 0 {
    return 0;
  }
  let target = ((total as f64) * quantile).ceil() as u64;
  for (i, counter) in buckets.iter().enumerate() {
    let cumulative = counter.load(Ordering::Relaxed);
    if cumulative >= target {
      return LATENCY_BUCKET_BOUNDS[i];
    }
  }
  LATENCY_BUCKET_BOUNDS.last().copied().unwrap_or(0)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_latency_record_and_histogram() {
    let stats = LatencyStats::default();
    stats.record("GET", 10);
    stats.record("GET", 100);
    stats.record("GET", 1000);

    let snap = stats.snapshot("GET").unwrap();
    assert_eq!(snap.calls, 3);
    assert_eq!(snap.total_us, 1110);
    assert_eq!(snap.max_us, 1000);
    assert!(!snap.buckets.is_empty());
    assert!(snap.p50_us >= 10);

    let hists = stats.histogram_snapshots(None);
    assert_eq!(hists.len(), 1);
    assert_eq!(hists[0].0, "get");

    assert_eq!(stats.reset(None), 1);
    assert!(stats.snapshot("GET").is_none());
  }
}
