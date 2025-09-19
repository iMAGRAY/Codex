use serde::Deserialize;
use serde::Serialize;
use std::collections::VecDeque;
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::time::Duration;
use thiserror::Error;
use tracing::warn;

pub use crate::telemetry_exporter::TelemetryExporter;
pub use crate::telemetry_exporter::TelemetryExporterConfig;
pub use crate::telemetry_exporter::TelemetryExporterError;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct TelemetrySnapshot {
    pub latency_p95_ms: f64,
    pub audit_fallback_count: u64,
    pub cache_hit_ratio: f64,
    pub apdex: f64,
}

impl Default for TelemetrySnapshot {
    fn default() -> Self {
        Self {
            latency_p95_ms: 0.0,
            audit_fallback_count: 0,
            cache_hit_ratio: 1.0,
            apdex: 1.0,
        }
    }
}

#[derive(Default)]
struct TelemetryInner {
    latencies_ms: VecDeque<f64>,
    audit_fallback_count: u64,
    cache_hits: u64,
    cache_misses: u64,
}

impl TelemetryInner {
    fn push_latency(&mut self, value_ms: f64) {
        const MAX_SAMPLES: usize = 256;
        if self.latencies_ms.len() >= MAX_SAMPLES {
            self.latencies_ms.pop_front();
        }
        self.latencies_ms.push_back(value_ms);
    }

    fn latency_p95(&self) -> f64 {
        if self.latencies_ms.is_empty() {
            return 0.0;
        }
        let mut samples: Vec<_> = self.latencies_ms.iter().copied().collect();
        samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let idx = ((samples.len() as f64 * 0.95).ceil() as usize).saturating_sub(1);
        samples[idx]
    }

    fn cache_hit_ratio(&self) -> f64 {
        let total = self.cache_hits + self.cache_misses;
        if total == 0 {
            return 1.0;
        }
        self.cache_hits as f64 / total as f64
    }

    fn snapshot(&self) -> TelemetrySnapshot {
        TelemetrySnapshot {
            latency_p95_ms: self.latency_p95(),
            audit_fallback_count: self.audit_fallback_count,
            cache_hit_ratio: self.cache_hit_ratio(),
            apdex: self.apdex_score(),
        }
    }

    fn apdex_score(&self) -> f64 {
        if self.latencies_ms.is_empty() {
            return 1.0;
        }
        let threshold = 300.0; // milliseconds, aligns with SEARCH latency SLO
        let mut satisfied = 0usize;
        let mut tolerated = 0usize;
        for &lat in &self.latencies_ms {
            if lat <= threshold {
                satisfied += 1;
            } else if lat <= threshold * 4.0 {
                tolerated += 1;
            }
        }
        let total = self.latencies_ms.len() as f64;
        let score = (satisfied as f64 + tolerated as f64 / 2.0) / total;
        score.clamp(0.0, 1.0)
    }
}

pub struct TelemetryHub {
    inner: Mutex<TelemetryInner>,
}

static EXPORTER: OnceLock<Arc<TelemetryExporter>> = OnceLock::new();

impl TelemetryHub {
    fn new() -> Self {
        Self {
            inner: Mutex::new(TelemetryInner::default()),
        }
    }

    pub fn global() -> &'static TelemetryHub {
        static TELEMETRY: OnceLock<TelemetryHub> = OnceLock::new();
        TELEMETRY.get_or_init(TelemetryHub::new)
    }

    pub fn record_exec_latency(&self, duration: Duration) {
        self.update_and_publish(|inner| inner.push_latency(duration.as_secs_f64() * 1000.0));
    }

    pub fn record_audit_fallback(&self) {
        self.update_and_publish(|inner| {
            inner.audit_fallback_count = inner.audit_fallback_count.saturating_add(1);
        });
    }

    pub fn record_cache_hit(&self) {
        self.update_and_publish(|inner| {
            inner.cache_hits = inner.cache_hits.saturating_add(1);
        });
    }

    pub fn record_cache_miss(&self) {
        self.update_and_publish(|inner| {
            inner.cache_misses = inner.cache_misses.saturating_add(1);
        });
    }

    pub fn snapshot(&self) -> TelemetrySnapshot {
        if let Ok(inner) = self.inner.lock() {
            inner.snapshot()
        } else {
            TelemetrySnapshot::default()
        }
    }

    fn update_and_publish<F>(&self, update: F)
    where
        F: FnOnce(&mut TelemetryInner),
    {
        let snapshot = if let Ok(mut inner) = self.inner.lock() {
            update(&mut inner);
            inner.snapshot()
        } else {
            return;
        };
        publish_snapshot(snapshot);
    }
}

#[derive(Debug, Error)]
pub enum TelemetryInstallError {
    #[error("telemetry exporter already installed")]
    AlreadyInstalled,
}

pub fn install_exporter(exporter: TelemetryExporter) -> Result<(), TelemetryInstallError> {
    EXPORTER
        .set(Arc::new(exporter))
        .map_err(|_| TelemetryInstallError::AlreadyInstalled)
}

pub fn exporter_config(codex_home: &Path) -> TelemetryExporterConfig {
    TelemetryExporterConfig::new(codex_home.join("telemetry"))
}

fn publish_snapshot(snapshot: TelemetrySnapshot) {
    if let Some(exporter) = EXPORTER.get() {
        if let Err(err) = exporter.record(snapshot) {
            warn!("failed to persist telemetry snapshot: {err}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_latency_p95() {
        let hub = TelemetryHub::new();
        for ms in [10.0, 20.0, 30.0, 40.0, 50.0] {
            hub.record_exec_latency(Duration::from_millis(ms as u64));
        }
        let snap = hub.snapshot();
        assert_eq!(snap.latency_p95_ms, 50.0);
    }

    #[test]
    fn cache_hit_ratio_defaults_to_one() {
        let hub = TelemetryHub::new();
        let snap = hub.snapshot();
        assert_eq!(snap.cache_hit_ratio, 1.0);
    }

    #[test]
    fn cache_hit_ratio_updates() {
        let hub = TelemetryHub::new();
        hub.record_cache_hit();
        hub.record_cache_hit();
        hub.record_cache_miss();
        let snap = hub.snapshot();
        assert!((snap.cache_hit_ratio - 2.0 / 3.0).abs() < f64::EPSILON);
    }
}
