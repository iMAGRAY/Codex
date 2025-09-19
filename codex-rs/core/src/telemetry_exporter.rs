// Observability mesh adapter (REQ-OBS-01, REQ-OPS-01; MaxThink-Stellar.md).
use crate::telemetry::TelemetrySnapshot;
use bincode::Error as BincodeError;
use prometheus::Encoder;
use prometheus::Gauge;
use prometheus::IntGauge;
use prometheus::Opts;
use prometheus::Registry;
use prometheus::TextEncoder;
use reqwest::Client;
use serde::Deserialize;
use serde::Serialize;
use sled::Batch;
use sled::Db;
use sled::Tree;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use thiserror::Error;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time::interval;
use tracing::warn;

#[derive(Debug, Clone)]
pub struct TelemetryExporterConfig {
    pub sled_path: PathBuf,
    pub tree_name: String,
    pub otlp_endpoint: Option<String>,
    pub otlp_headers: Vec<(String, String)>,
    pub prometheus_bind: Option<SocketAddr>,
    pub flush_interval: Duration,
}

impl TelemetryExporterConfig {
    pub fn new(sled_path: PathBuf) -> Self {
        Self {
            sled_path,
            tree_name: "stellar-telemetry".to_string(),
            otlp_endpoint: None,
            otlp_headers: Vec::new(),
            prometheus_bind: None,
            flush_interval: Duration::from_secs(30),
        }
    }

    pub fn with_otlp_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.otlp_endpoint = Some(endpoint.into());
        self
    }

    pub fn with_otlp_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.otlp_headers.push((key.into(), value.into()));
        self
    }

    pub fn with_prometheus_bind(mut self, addr: SocketAddr) -> Self {
        self.prometheus_bind = Some(addr);
        self
    }

    pub fn with_flush_interval(mut self, interval: Duration) -> Self {
        self.flush_interval = interval;
        self
    }
}

#[derive(Debug)]
pub struct TelemetryExporter {
    #[allow(dead_code)]
    db: Db,
    tree: Tree,
    client: Client,
    otlp: Option<OtlpTask>,
    prom: Option<PrometheusEndpoint>,
}

impl TelemetryExporter {
    pub fn new(config: TelemetryExporterConfig) -> Result<Self, TelemetryExporterError> {
        let mut builder = sled::Config::new();
        builder = builder.path(&config.sled_path);
        let db = builder.open()?;
        let tree = db.open_tree(&config.tree_name)?;
        let client = Client::new();

        let prom = if let Some(addr) = config.prometheus_bind {
            Some(PrometheusEndpoint::new(addr)?)
        } else {
            None
        };

        let otlp = if let Some(endpoint) = config.otlp_endpoint.clone() {
            let headers = config.otlp_headers.clone();
            let (shutdown_tx, shutdown_rx) = oneshot::channel();
            let tree_clone = tree.clone();
            let client_clone = client.clone();
            let interval_duration = config.flush_interval;
            let endpoint_clone = endpoint.clone();
            let headers_clone = headers.clone();
            let handle = tokio::spawn(async move {
                run_flush_loop(
                    tree_clone,
                    client_clone,
                    endpoint_clone,
                    headers_clone,
                    interval_duration,
                    shutdown_rx,
                )
                .await;
            });
            Some(OtlpTask {
                endpoint,
                headers,
                handle,
                shutdown_tx,
            })
        } else {
            None
        };

        Ok(Self {
            db,
            tree,
            client,
            otlp,
            prom,
        })
    }

    pub fn record(&self, snapshot: TelemetrySnapshot) -> Result<(), TelemetryExporterError> {
        let record = TelemetryRecord::from(snapshot);
        self.tree.insert(record.key(), record.encode()?)?;
        if let Some(prom) = &self.prom {
            prom.update(snapshot);
        }
        Ok(())
    }

    pub async fn flush_once(&self) -> Result<(), TelemetryExporterError> {
        if let Some(task) = &self.otlp {
            flush_once_internal(&self.tree, &self.client, &task.endpoint, &task.headers).await
        } else {
            Ok(())
        }
    }
}

impl Drop for TelemetryExporter {
    fn drop(&mut self) {
        if let Some(prom) = self.prom.take() {
            prom.shutdown();
        }
        if let Some(task) = self.otlp.take() {
            task.shutdown();
        }
    }
}

#[derive(Debug)]
struct OtlpTask {
    endpoint: String,
    headers: Vec<(String, String)>,
    handle: JoinHandle<()>,
    shutdown_tx: oneshot::Sender<()>,
}

impl OtlpTask {
    fn shutdown(self) {
        let _ = self.shutdown_tx.send(());
        self.handle.abort();
    }
}

#[derive(Debug, Error)]
pub enum TelemetryExporterError {
    #[error("sled error: {0}")]
    Store(#[from] sled::Error),
    #[error("bincode error: {0}")]
    Serialize(#[from] BincodeError),
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("prometheus error: {0}")]
    Prometheus(#[from] prometheus::Error),
    #[error("otlp status {0}")]
    OtlpStatus(u16),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TelemetryRecord {
    timestamp_ms: i64,
    latency_p95_ms: f64,
    audit_fallback_count: u64,
    cache_hit_ratio: f64,
    apdex: f64,
}

impl TelemetryRecord {
    fn from(snapshot: TelemetrySnapshot) -> Self {
        let timestamp_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        Self {
            timestamp_ms,
            latency_p95_ms: snapshot.latency_p95_ms,
            audit_fallback_count: snapshot.audit_fallback_count,
            cache_hit_ratio: snapshot.cache_hit_ratio,
            apdex: snapshot.apdex,
        }
    }

    fn key(&self) -> [u8; 8] {
        self.timestamp_ms.to_be_bytes()
    }

    fn encode(&self) -> Result<Vec<u8>, BincodeError> {
        bincode::serialize(self)
    }

    fn time_unix_nano(&self) -> u64 {
        (self.timestamp_ms as i128 * 1_000_000)
            .max(0)
            .min(i64::MAX as i128) as u64
    }
}

#[derive(Debug)]
struct PrometheusEndpoint {
    _registry: Registry,
    latency_gauge: Gauge,
    audit_gauge: IntGauge,
    cache_gauge: Gauge,
    apdex_gauge: Gauge,
    shutdown_tx: Option<oneshot::Sender<()>>,
    handle: JoinHandle<()>,
}

impl PrometheusEndpoint {
    fn new(addr: SocketAddr) -> Result<Self, TelemetryExporterError> {
        let registry = Registry::new_custom(Some("stellar".to_string()), None)?;
        let latency_opts = Opts::new(
            "stellar_latency_p95_ms",
            "p95 latency in milliseconds (REQ-OBS-01)",
        );
        let latency_gauge = Gauge::with_opts(latency_opts)?;
        registry.register(Box::new(latency_gauge.clone()))?;

        let audit_opts = Opts::new(
            "stellar_audit_fallback_count",
            "Audit fallback occurrences (REQ-OPS-01)",
        );
        let audit_gauge = IntGauge::with_opts(audit_opts)?;
        registry.register(Box::new(audit_gauge.clone()))?;

        let cache_opts = Opts::new("stellar_cache_hit_pct", "Cache hit percentage (REQ-REL-01)");
        let cache_gauge = Gauge::with_opts(cache_opts)?;
        registry.register(Box::new(cache_gauge.clone()))?;

        let apdex_opts = Opts::new("stellar_apdex", "APDEX score (REQ-ACC-01)");
        let apdex_gauge = Gauge::with_opts(apdex_opts)?;
        registry.register(Box::new(apdex_gauge.clone()))?;

        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let registry_clone = registry.clone();
        let handle = tokio::spawn(async move {
            if let Err(err) = run_prometheus_server(addr, registry_clone, shutdown_rx).await {
                warn!("prometheus endpoint failed: {err}");
            }
        });

        Ok(Self {
            _registry: registry,
            latency_gauge,
            audit_gauge,
            cache_gauge,
            apdex_gauge,
            shutdown_tx: Some(shutdown_tx),
            handle,
        })
    }

    fn update(&self, snapshot: TelemetrySnapshot) {
        self.latency_gauge.set(snapshot.latency_p95_ms);
        self.audit_gauge.set(snapshot.audit_fallback_count as i64);
        self.cache_gauge.set(snapshot.cache_hit_ratio * 100.0);
        self.apdex_gauge.set(snapshot.apdex);
    }

    fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        self.handle.abort();
    }
}

async fn run_prometheus_server(
    addr: SocketAddr,
    registry: Registry,
    shutdown: oneshot::Receiver<()>,
) -> Result<(), hyper::Error> {
    use hyper::Body;
    use hyper::Request;
    use hyper::Response;
    use hyper::Server;
    use hyper::service::make_service_fn;
    use hyper::service::service_fn;
    use std::convert::Infallible;

    let make_svc = make_service_fn(move |_conn| {
        let registry = registry.clone();
        async move {
            Ok::<_, Infallible>(service_fn(move |_req: Request<Body>| {
                let registry = registry.clone();
                async move {
                    let encoder = TextEncoder::new();
                    let families = registry.gather();
                    let mut buffer = Vec::new();
                    if let Err(err) = encoder.encode(&families, &mut buffer) {
                        warn!("failed to encode prometheus metrics: {err}");
                    }
                    Ok::<_, Infallible>(
                        Response::builder()
                            .status(200)
                            .header("content-type", encoder.format_type())
                            .body(Body::from(buffer))
                            .unwrap(),
                    )
                }
            }))
        }
    });

    let server = Server::bind(&addr).serve(make_svc);
    server
        .with_graceful_shutdown(async move {
            let _ = shutdown.await;
        })
        .await
}

async fn run_flush_loop(
    tree: Tree,
    client: Client,
    endpoint: String,
    headers: Vec<(String, String)>,
    interval_duration: Duration,
    shutdown: oneshot::Receiver<()>,
) {
    let mut ticker = interval(interval_duration);
    tokio::pin!(shutdown);
    // Skip the immediate tick emitted by `interval` so the first flush happens after `interval_duration`.
    let mut skipped_first_tick = false;
    loop {
        tokio::select! {
            _ = &mut shutdown => {
                if let Err(err) = flush_once_internal(&tree, &client, &endpoint, &headers).await {
                    warn!("telemetry flush failed during shutdown: {err}");
                }
                break;
            }
            _ = ticker.tick() => {
                if !skipped_first_tick {
                    skipped_first_tick = true;
                    continue;
                }
                if let Err(err) = flush_once_internal(&tree, &client, &endpoint, &headers).await {
                    warn!("telemetry flush failed: {err}");
                }
            }
        }
    }
}

async fn flush_once_internal(
    tree: &Tree,
    client: &Client,
    endpoint: &str,
    headers: &[(String, String)],
) -> Result<(), TelemetryExporterError> {
    let mut entries = Vec::new();
    for item in tree.iter() {
        let (key, value) = item?;
        let record: TelemetryRecord = bincode::deserialize(&value)?;
        entries.push((key, record));
    }
    if entries.is_empty() {
        return Ok(());
    }

    let payload =
        ExportMetricsServiceRequest::from_records(entries.iter().map(|(_, record)| record));
    let mut request = client.post(endpoint);
    for (key, value) in headers {
        request = request.header(key, value);
    }
    let response = request.json(&payload).send().await?;
    if !response.status().is_success() {
        return Err(TelemetryExporterError::OtlpStatus(
            response.status().as_u16(),
        ));
    }

    let mut batch = Batch::default();
    for (key, _) in entries {
        batch.remove(key);
    }
    tree.apply_batch(batch)?;
    Ok(())
}

#[derive(Serialize)]
struct ExportMetricsServiceRequest {
    #[serde(rename = "resourceMetrics")]
    resource_metrics: Vec<ResourceMetrics>,
}

impl ExportMetricsServiceRequest {
    fn from_records<'a, I>(records: I) -> Self
    where
        I: Iterator<Item = &'a TelemetryRecord>,
    {
        let mut latency = Vec::new();
        let mut audit = Vec::new();
        let mut cache = Vec::new();
        let mut apdex = Vec::new();
        for record in records {
            let ts = record.time_unix_nano();
            latency.push(NumberDataPoint::new(ts, record.latency_p95_ms));
            audit.push(NumberDataPoint::new(ts, record.audit_fallback_count as f64));
            cache.push(NumberDataPoint::new(ts, record.cache_hit_ratio * 100.0));
            apdex.push(NumberDataPoint::new(ts, record.apdex));
        }
        Self {
            resource_metrics: vec![ResourceMetrics {
                scope_metrics: vec![ScopeMetrics {
                    metrics: vec![
                        Metric::gauge("stellar.latency_p95_ms", latency),
                        Metric::gauge("stellar.audit_fallback_count", audit),
                        Metric::gauge("stellar.cache_hit_pct", cache),
                        Metric::gauge("stellar.apdex", apdex),
                    ],
                }],
            }],
        }
    }
}

#[derive(Serialize)]
struct ResourceMetrics {
    #[serde(rename = "scopeMetrics")]
    scope_metrics: Vec<ScopeMetrics>,
}

#[derive(Serialize)]
struct ScopeMetrics {
    metrics: Vec<Metric>,
}

#[derive(Serialize)]
struct Metric {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    gauge: Option<GaugeMetric>,
}

impl Metric {
    fn gauge(name: &str, data_points: Vec<NumberDataPoint>) -> Self {
        Self {
            name: name.to_string(),
            gauge: Some(GaugeMetric { data_points }),
        }
    }
}

#[derive(Serialize)]
struct GaugeMetric {
    #[serde(rename = "dataPoints")]
    data_points: Vec<NumberDataPoint>,
}

#[derive(Serialize)]
struct NumberDataPoint {
    #[serde(rename = "timeUnixNano")]
    time_unix_nano: u64,
    #[serde(rename = "asDouble")]
    as_double: f64,
}

impl NumberDataPoint {
    fn new(time_unix_nano: u64, as_double: f64) -> Self {
        Self {
            time_unix_nano,
            as_double,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use tokio::time::Duration as TokioDuration;
    use tokio::time::sleep;
    use wiremock::Mock;
    use wiremock::MockServer;
    use wiremock::ResponseTemplate;
    use wiremock::matchers::body_string_contains;
    use wiremock::matchers::method;
    use wiremock::matchers::path;

    fn temp_config() -> (TelemetryExporterConfig, TempDir) {
        let dir = TempDir::new().expect("temp dir");
        let cfg = TelemetryExporterConfig::new(dir.path().join("sled"))
            .with_flush_interval(Duration::from_secs(1));
        (cfg, dir)
    }

    fn sample_snapshot() -> TelemetrySnapshot {
        TelemetrySnapshot {
            latency_p95_ms: 123.0,
            audit_fallback_count: 2,
            cache_hit_ratio: 0.88,
            apdex: 0.91,
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn flush_once_sends_payload_and_clears_sled() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/"))
            .and(body_string_contains("stellar.latency_p95_ms"))
            .and(body_string_contains("stellar.audit_fallback_count"))
            .and(body_string_contains("stellar.cache_hit_pct"))
            .and(body_string_contains("stellar.apdex"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let (cfg, _dir) = temp_config();
        let cfg = cfg
            .with_otlp_endpoint(server.uri())
            .with_flush_interval(Duration::from_secs(60));
        let exporter = TelemetryExporter::new(cfg).expect("exporter");

        exporter.record(sample_snapshot()).expect("record");
        exporter.flush_once().await.expect("flush");

        assert!(
            exporter.tree.iter().next().is_none(),
            "sled tree should be empty after flush"
        );

        server.verify().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn prometheus_endpoint_exposes_latest_snapshot() {
        let (cfg, _dir) = temp_config();

        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("listener");
        let addr = listener.local_addr().expect("addr");
        drop(listener);

        let cfg = cfg.with_prometheus_bind(addr);
        let exporter = TelemetryExporter::new(cfg).expect("exporter");

        exporter.record(sample_snapshot()).expect("record");

        // Give the HTTP server a brief moment to bind before polling it.
        sleep(TokioDuration::from_millis(50)).await;

        let resp = reqwest::get(format!("http://{}", addr))
            .await
            .expect("request");
        assert!(resp.status().is_success());
        let body = resp.text().await.expect("text");
        assert!(body.contains("stellar_latency_p95_ms"));
        assert!(body.contains("stellar_audit_fallback_count"));
        assert!(body.contains("stellar_cache_hit_pct"));
        assert!(body.contains("stellar_apdex"));

        drop(exporter);
    }
}
