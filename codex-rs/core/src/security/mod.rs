//! Security primitives for Stellar: dynamic secrets, output scrubbing,
//! consent logging.
//!
//! Trace: REQ-SEC-03 (#27, #88, #92) — обеспечивает короткоживущие секреты,
//! безопасную очистку буфера и аудит согласий.

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::time::Duration;
use std::time::Instant;

use crate::config::find_codex_home;
use anyhow::Result;
use base64::Engine;
use chrono::DateTime;
use chrono::Duration as ChronoDuration;
use chrono::Utc;
use rand::RngCore;
use rand::rng;
use serde::Deserialize;
use serde::Serialize;
use uuid::Uuid;

use sha2::Digest;
use sha2::Sha256;
use sled::Batch;
use thiserror::Error;

const DEFAULT_SECRET_TTL: Duration = Duration::from_secs(600);
const GENERATED_SECRET_LEN: usize = 32;
const ENV_SECRET_KEY: &str = "CODEX_DYNAMIC_SECRET";

const DEFAULT_CPU_LIMIT_SECS: u64 = 120;
const DEFAULT_MEMORY_LIMIT_BYTES: u64 = 8 * 1024 * 1024 * 1024; // 8 GiB

const AUDIT_LEDGER_DIR: &str = "audit-ledger";
const AUDIT_ENTRIES_TREE: &[u8] = b"stellar-audit-entries";
const AUDIT_META_TREE: &[u8] = b"stellar-audit-meta";
const POLICY_EVIDENCE_TREE: &[u8] = b"stellar-policy-evidence";
const AUDIT_RECORD_VERSION: u32 = 1;
const AUDIT_GENESIS_HASH: &str = "GENESIS";
const POLICY_EVIDENCE_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const POLICY_EVIDENCE_TTL_SECS: i64 = 24 * 60 * 60;

/// CPU/memory guardrails applied before spawning sandboxed processes.
#[derive(Debug, Clone, Copy)]
pub struct ResourceLimits {
    pub cpu_time_seconds: Option<u64>,
    pub memory_bytes: Option<u64>,
}

impl ResourceLimits {
    /// Baseline shield aligned с REQ-SEC-03 (A9/D5 safeguards).
    #[must_use]
    pub fn standard() -> Self {
        let cpu_time_seconds =
            read_env_u64("CODEX_SANDBOX_CPU_SECS").or(Some(DEFAULT_CPU_LIMIT_SECS));
        let memory_bytes =
            read_env_u64("CODEX_SANDBOX_MEMORY_BYTES").or(Some(DEFAULT_MEMORY_LIMIT_BYTES));
        Self {
            cpu_time_seconds,
            memory_bytes,
        }
    }

    #[cfg(target_os = "linux")]
    pub fn apply(self) -> std::io::Result<()> {
        use libc::rlimit;
        if let Some(cpu_secs) = self.cpu_time_seconds {
            let limit = rlimit {
                rlim_cur: cpu_secs as libc::rlim_t,
                rlim_max: cpu_secs as libc::rlim_t,
            };
            unsafe {
                if libc::setrlimit(libc::RLIMIT_CPU, &limit) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
        }

        if let Some(memory_bytes) = self.memory_bytes {
            let limit = rlimit {
                rlim_cur: memory_bytes as libc::rlim_t,
                rlim_max: memory_bytes as libc::rlim_t,
            };
            unsafe {
                if libc::setrlimit(libc::RLIMIT_AS, &limit) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
        }
        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    pub fn apply(self) -> std::io::Result<()> {
        let _ = self;
        Ok(())
    }
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self::standard()
    }
}

static SECRET_BROKER: OnceLock<SecretBroker> = OnceLock::new();
static CONSENT_LOG: OnceLock<ConsentLog> = OnceLock::new();
static AUDIT_LEDGER: OnceLock<AuditLedger> = OnceLock::new();
static TEMP_AUDIT_LEDGER: OnceLock<AuditLedger> = OnceLock::new();

/// Persona-scoped secret allocation target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretScope {
    Env,
    Clipboard,
    Session,
}

#[derive(Debug, Clone)]
pub struct SecretLease {
    pub id: Uuid,
    pub value: String,
    pub scope: SecretScope,
    pub issued_at: Instant,
    pub expires_at: Instant,
}

impl SecretLease {
    fn is_expired(&self, now: Instant) -> bool {
        now >= self.expires_at
    }
}

#[derive(Debug, Default)]
struct SecretStore {
    leases: Vec<SecretLease>,
}

impl SecretStore {
    fn purge_expired(&mut self) {
        let now = Instant::now();
        self.leases.retain(|lease| !lease.is_expired(now));
    }

    fn register(&mut self, lease: SecretLease) {
        self.purge_expired();
        self.leases.push(lease);
    }

    fn values(&self) -> Vec<String> {
        self.leases
            .iter()
            .map(|lease| lease.value.clone())
            .collect()
    }
}

/// Manages short-lived secrets and performs content scrubbing.
pub struct SecretBroker {
    store: Mutex<SecretStore>,
}

impl SecretBroker {
    pub fn global() -> &'static Self {
        SECRET_BROKER.get_or_init(|| SecretBroker {
            store: Mutex::new(SecretStore::default()),
        })
    }

    /// Inject an ephemeral secret into environment variables if not already
    /// present. Returns the active secret value.
    pub fn ensure_env_secret(&self, env: &mut HashMap<String, String>) -> String {
        if let Some(existing) = env.get(ENV_SECRET_KEY) {
            self.register_secret(existing.clone(), SecretScope::Env);
            return existing.clone();
        }
        let lease = self.issue_secret(SecretScope::Env);
        env.insert(ENV_SECRET_KEY.to_string(), lease.value.clone());
        lease.value
    }

    /// Register existing secret material to guarantee redaction.
    pub fn register_secret(&self, value: String, scope: SecretScope) {
        if value.is_empty() {
            return;
        }
        let lease = SecretLease {
            id: Uuid::new_v4(),
            scope,
            issued_at: Instant::now(),
            expires_at: Instant::now() + DEFAULT_SECRET_TTL,
            value,
        };
        if let Ok(mut store) = self.store.lock() {
            store.register(lease);
        }
    }

    /// Create a fresh secret lease tracked for subsequent scrubbing.
    pub fn issue_secret(&self, scope: SecretScope) -> SecretLease {
        let mut buf = [0u8; GENERATED_SECRET_LEN];
        let mut rng = rng();
        rng.fill_bytes(&mut buf);
        let value = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf);
        let lease = SecretLease {
            id: Uuid::new_v4(),
            scope,
            issued_at: Instant::now(),
            expires_at: Instant::now() + DEFAULT_SECRET_TTL,
            value,
        };
        if let Ok(mut store) = self.store.lock() {
            store.register(lease.clone());
        }
        lease
    }

    /// Replace any registered secrets within `text` with `"***"`.
    pub fn scrub_text(&self, text: &str) -> String {
        let values = self
            .store
            .lock()
            .map(|store| store.values())
            .unwrap_or_default();
        if values.is_empty() {
            return text.to_string();
        }
        let mut scrubbed = text.to_string();
        for value in values {
            if !value.is_empty() {
                scrubbed = scrubbed.replace(&value, "***");
            }
        }
        scrubbed
    }

    /// Scrub in-place to avoid copies for large buffers.
    pub fn scrub_string(&self, text: &mut String) {
        let cleaned = self.scrub_text(text);
        if *text != cleaned {
            *text = cleaned;
        }
    }
}

#[derive(Debug, Clone)]
pub struct ConsentEvent {
    pub timestamp: DateTime<Utc>,
    pub actor: String,
    pub action: String,
    pub resource: String,
    pub metadata: HashMap<String, String>,
}

impl ConsentEvent {
    pub fn new(
        actor: impl Into<String>,
        action: impl Into<String>,
        resource: impl Into<String>,
    ) -> Self {
        Self {
            timestamp: Utc::now(),
            actor: actor.into(),
            action: action.into(),
            resource: resource.into(),
            metadata: HashMap::new(),
        }
    }

    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }
}

/// In-memory consent ledger (exportable later to immutable audit log).
pub struct ConsentLog {
    events: Mutex<Vec<ConsentEvent>>,
}

impl ConsentLog {
    pub fn global() -> &'static Self {
        CONSENT_LOG.get_or_init(|| ConsentLog {
            events: Mutex::new(Vec::new()),
        })
    }

    pub fn record(&self, event: ConsentEvent) {
        if let Ok(mut events) = self.events.lock() {
            events.push(event);
        }
    }

    pub fn recent(&self, max: usize) -> Vec<ConsentEvent> {
        let Ok(events) = self.events.lock() else {
            return Vec::new();
        };
        let len = events.len();
        let start = len.saturating_sub(max);
        events[start..].to_vec()
    }
}

/// Deterministic metadata pair used for hashing and exports (REQ-SEC-02, #10, #57).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MetadataEntry {
    pub key: String,
    pub value: String,
}

/// Immutable audit event category (REQ-SEC-02, #10, #57).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuditEventKind {
    Consent,
    SandboxExec,
    SupplyChain,
}

impl AuditEventKind {
    fn label(self) -> &'static str {
        match self {
            AuditEventKind::Consent => "consent",
            AuditEventKind::SandboxExec => "sandbox_exec",
            AuditEventKind::SupplyChain => "supply_chain",
        }
    }
}

/// Immutable audit record persisted to the ledger (REQ-SEC-02, #10, #57).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuditRecord {
    pub id: Uuid,
    pub sequence: u64,
    pub version: u32,
    pub kind: AuditEventKind,
    pub timestamp: DateTime<Utc>,
    pub actor: String,
    pub action: String,
    pub resource: String,
    pub metadata: Vec<MetadataEntry>,
    pub prev_hash: String,
    pub hash: String,
}

/// Short-lived policy evidence derived from audit records (TTL 24h per REQ-SEC-02).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PolicyEvidenceRecord {
    pub id: Uuid,
    pub hash: String,
    pub kind: AuditEventKind,
    pub actor: String,
    pub action: String,
    pub resource: String,
    pub recorded_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub metadata: Vec<MetadataEntry>,
}

/// Audit event captured before committing to the immutable ledger.
#[derive(Debug, Clone)]
pub struct AuditEvent {
    pub kind: AuditEventKind,
    pub timestamp: DateTime<Utc>,
    pub actor: String,
    pub action: String,
    pub resource: String,
    pub metadata: HashMap<String, String>,
}

impl AuditEvent {
    #[must_use]
    pub fn new(
        kind: AuditEventKind,
        actor: impl Into<String>,
        action: impl Into<String>,
        resource: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            timestamp: Utc::now(),
            actor: actor.into(),
            action: action.into(),
            resource: resource.into(),
            metadata: HashMap::new(),
        }
    }

    #[must_use]
    pub fn with_timestamp(mut self, timestamp: DateTime<Utc>) -> Self {
        self.timestamp = timestamp;
        self
    }

    #[must_use]
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }
}

impl From<&ConsentEvent> for AuditEvent {
    fn from(event: &ConsentEvent) -> Self {
        let mut audit = AuditEvent::new(
            AuditEventKind::Consent,
            &event.actor,
            &event.action,
            &event.resource,
        )
        .with_timestamp(event.timestamp);
        for (key, value) in &event.metadata {
            audit.metadata.insert(key.clone(), value.clone());
        }
        audit
    }
}

#[derive(Debug, Clone)]
struct AuditLedgerConfig {
    path: PathBuf,
    policy_ttl: Duration,
    temporary: bool,
}

impl AuditLedgerConfig {
    fn with_default_path() -> Result<Self, LedgerError> {
        let mut path = find_codex_home()?;
        path.push(AUDIT_LEDGER_DIR);
        Ok(Self {
            path,
            policy_ttl: POLICY_EVIDENCE_TTL,
            temporary: false,
        })
    }

    fn temporary_for_process() -> Self {
        let mut path = env::temp_dir();
        path.push(format!(
            "codex-audit-ledger-{}-{}",
            process::id(),
            Uuid::new_v4()
        ));
        Self {
            path,
            policy_ttl: POLICY_EVIDENCE_TTL,
            temporary: true,
        }
    }

    #[cfg(test)]
    fn for_path(path: PathBuf) -> Self {
        Self {
            path,
            policy_ttl: POLICY_EVIDENCE_TTL,
            temporary: false,
        }
    }
}

#[derive(Debug, Error)]
pub enum LedgerError {
    #[error("failed to prepare audit ledger directory: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to interact with audit ledger store: {0}")]
    Store(#[from] sled::Error),
    #[error("failed to (de)serialize audit ledger record: {0}")]
    Serialization(#[from] bincode::Error),
    #[error("corrupt audit ledger metadata for key {0}")]
    CorruptMetadata(&'static str),
}

struct AuditLedger {
    #[allow(dead_code)]
    db: sled::Db,
    entries: sled::Tree,
    metadata: sled::Tree,
    evidence: sled::Tree,
    policy_ttl: Duration,
}

impl AuditLedger {
    fn open(config: AuditLedgerConfig) -> Result<Self, LedgerError> {
        fs::create_dir_all(&config.path)?;
        let mut builder = sled::Config::new().path(&config.path);
        if config.temporary {
            builder = builder.temporary(true);
        }
        let db = builder.open()?;
        let entries = db.open_tree(AUDIT_ENTRIES_TREE)?;
        let metadata = db.open_tree(AUDIT_META_TREE)?;
        let evidence = db.open_tree(POLICY_EVIDENCE_TREE)?;
        Ok(Self {
            db,
            entries,
            metadata,
            evidence,
            policy_ttl: config.policy_ttl,
        })
    }

    fn append(&self, event: AuditEvent) -> Result<AuditRecord, LedgerError> {
        self.purge_expired_evidence(Utc::now())?;
        let sequence = self.db.generate_id()?;
        let prev_hash = match self.metadata.get(b"last_hash")? {
            Some(bytes) => String::from_utf8(bytes.to_vec())
                .map_err(|_| LedgerError::CorruptMetadata("last_hash"))?,
            None => AUDIT_GENESIS_HASH.to_string(),
        };

        let metadata_vec = sorted_metadata(&event.metadata);
        let timestamp_micros = event.timestamp.timestamp_micros();
        let id = Uuid::new_v4();

        let mut hasher = Sha256::new();
        hasher.update(id.as_bytes());
        hasher.update(sequence.to_be_bytes());
        hasher.update(timestamp_micros.to_be_bytes());
        hasher.update(event.kind.label().as_bytes());
        hasher.update(event.actor.as_bytes());
        hasher.update(event.action.as_bytes());
        hasher.update(event.resource.as_bytes());
        for entry in &metadata_vec {
            hasher.update(entry.key.as_bytes());
            hasher.update(entry.value.as_bytes());
        }
        hasher.update(prev_hash.as_bytes());
        let hash = hex::encode(hasher.finalize());

        let record = AuditRecord {
            id,
            sequence,
            version: AUDIT_RECORD_VERSION,
            kind: event.kind,
            timestamp: event.timestamp,
            actor: event.actor,
            action: event.action,
            resource: event.resource,
            metadata: metadata_vec,
            prev_hash,
            hash,
        };

        let encoded = bincode::serialize(&record)?;
        self.entries.insert(ledger_key(record.sequence), encoded)?;
        self.metadata.insert(b"last_hash", record.hash.as_bytes())?;
        self.metadata
            .insert(b"last_sequence", record.sequence.to_be_bytes().to_vec())?;
        self.metadata
            .insert(b"last_timestamp", timestamp_micros.to_be_bytes().to_vec())?;

        self.store_policy_evidence(&record)?;
        Ok(record)
    }

    fn export(&self) -> Result<Vec<AuditRecord>, LedgerError> {
        let mut records = Vec::new();
        for entry in self.entries.iter() {
            let (_, value) = entry?;
            records.push(bincode::deserialize(value.as_ref())?);
        }
        Ok(records)
    }

    fn export_since(&self, since: Option<DateTime<Utc>>) -> Result<Vec<AuditRecord>, LedgerError> {
        if since.is_none() {
            return self.export();
        }
        let threshold = since.unwrap().timestamp_micros();
        let mut records = Vec::new();
        for entry in self.entries.iter() {
            let (_, value) = entry?;
            let record: AuditRecord = bincode::deserialize(value.as_ref())?;
            if record.timestamp.timestamp_micros() >= threshold {
                records.push(record);
            }
        }
        Ok(records)
    }

    fn export_policy_evidence(&self) -> Result<Vec<PolicyEvidenceRecord>, LedgerError> {
        self.purge_expired_evidence(Utc::now())?;
        let mut records = Vec::new();
        for entry in self.evidence.iter() {
            let (_, value) = entry?;
            records.push(bincode::deserialize(value.as_ref())?);
        }
        Ok(records)
    }

    fn store_policy_evidence(&self, record: &AuditRecord) -> Result<(), LedgerError> {
        let ttl = ChronoDuration::from_std(self.policy_ttl)
            .unwrap_or_else(|_| ChronoDuration::seconds(POLICY_EVIDENCE_TTL_SECS));
        let expires_at = record.timestamp + ttl;
        let evidence = PolicyEvidenceRecord {
            id: record.id,
            hash: record.hash.clone(),
            kind: record.kind,
            actor: record.actor.clone(),
            action: record.action.clone(),
            resource: record.resource.clone(),
            recorded_at: record.timestamp,
            expires_at,
            metadata: record.metadata.clone(),
        };
        let encoded = bincode::serialize(&evidence)?;
        self.evidence
            .insert(evidence_key(record.timestamp, &record.id), encoded)?;
        Ok(())
    }

    fn purge_expired_evidence(&self, now: DateTime<Utc>) -> Result<(), LedgerError> {
        let mut batch = Batch::default();
        let mut removed = false;
        for entry in self.evidence.iter() {
            let (key, value) = entry?;
            let evidence: PolicyEvidenceRecord = bincode::deserialize(value.as_ref())?;
            if evidence.expires_at <= now {
                batch.remove(key);
                removed = true;
            }
        }
        if removed {
            self.evidence.apply_batch(batch)?;
        }
        Ok(())
    }
}

fn sorted_metadata(map: &HashMap<String, String>) -> Vec<MetadataEntry> {
    let mut entries: Vec<_> = map
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries
        .into_iter()
        .map(|(key, value)| MetadataEntry { key, value })
        .collect()
}

fn ledger_key(sequence: u64) -> [u8; 8] {
    sequence.to_be_bytes()
}

fn evidence_key(timestamp: DateTime<Utc>, id: &Uuid) -> [u8; 24] {
    let micros = timestamp.timestamp_micros();
    let mut key = [0u8; 24];
    key[..8].copy_from_slice(&micros.to_be_bytes());
    key[8..].copy_from_slice(id.as_bytes());
    key
}

fn audit_ledger() -> Result<&'static AuditLedger, LedgerError> {
    if let Some(ledger) = AUDIT_LEDGER.get() {
        return Ok(ledger);
    }
    let config = AuditLedgerConfig::with_default_path()?;
    let ledger = match AuditLedger::open(config) {
        Ok(ledger) => ledger,
        Err(LedgerError::Store(sled::Error::Io(io_err)))
            if io_err.kind() == std::io::ErrorKind::WouldBlock =>
        {
            tracing::warn!("audit ledger locked at CODEX_HOME; using temporary in-memory store");
            AuditLedger::open(AuditLedgerConfig::temporary_for_process())?
        }
        Err(err) => return Err(err),
    };
    let _ = AUDIT_LEDGER.set(ledger);
    Ok(AUDIT_LEDGER
        .get()
        .expect("audit ledger OnceLock should be initialized"))
}

fn audit_ledger_fallback() -> Result<&'static AuditLedger, LedgerError> {
    if let Some(ledger) = TEMP_AUDIT_LEDGER.get() {
        return Ok(ledger);
    }
    let ledger = AuditLedger::open(AuditLedgerConfig::temporary_for_process())?;
    let _ = TEMP_AUDIT_LEDGER.set(ledger);
    Ok(TEMP_AUDIT_LEDGER
        .get()
        .expect("temporary audit ledger OnceLock should be initialized"))
}

pub fn append_audit_event(event: AuditEvent) -> Result<AuditRecord, LedgerError> {
    match audit_ledger()?.append(event.clone()) {
        Ok(record) => Ok(record),
        Err(err) => {
            if let LedgerError::Store(sled::Error::Io(io_err)) = &err {
                if io_err.kind() == std::io::ErrorKind::WouldBlock {
                    tracing::warn!("audit ledger append blocked; routing entry to temporary store");
                    crate::telemetry::TelemetryHub::global().record_audit_fallback();
                    return audit_ledger_fallback()?.append(event);
                }
            }
            Err(err)
        }
    }
}

pub fn export_audit_records_since(
    since: Option<DateTime<Utc>>,
) -> Result<Vec<AuditRecord>, LedgerError> {
    audit_ledger()?.export_since(since)
}

pub fn export_audit_records() -> Result<Vec<AuditRecord>, LedgerError> {
    export_audit_records_since(None)
}

pub fn export_policy_evidence_snapshot() -> Result<Vec<PolicyEvidenceRecord>, LedgerError> {
    audit_ledger()?.export_policy_evidence()
}

/// Convenience helper: record consent and append to the immutable audit ledger.
pub fn record_consent(event: ConsentEvent) -> Result<()> {
    let audit_event = AuditEvent::from(&event);
    ConsentLog::global().record(event);
    append_audit_event(audit_event).map_err(|err| {
        tracing::error!(error = ?err, "failed to append consent audit entry (REQ-SEC-02)");
        err
    })?;
    Ok(())
}

/// Map OS signals to human-readable resource shield warnings.
#[must_use]
pub fn resource_signal_message(signal: i32) -> Option<&'static str> {
    #[cfg(target_os = "linux")]
    {
        return match signal {
            libc::SIGXCPU => Some("CPU time limit exceeded"),
            libc::SIGKILL => Some("Process terminated by kernel (possible memory exhaustion)"),
            _ => None,
        };
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = signal;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn env_secret_injected_once() {
        let broker = SecretBroker::global();
        let mut env = HashMap::new();
        let secret1 = broker.ensure_env_secret(&mut env);
        let secret2 = broker.ensure_env_secret(&mut env);
        assert_eq!(secret1, secret2);
        assert!(env.contains_key(ENV_SECRET_KEY));
    }

    #[test]
    fn scrubber_masks_registered_secret() {
        let broker = SecretBroker::global();
        let lease = broker.issue_secret(SecretScope::Session);
        let sensitive = format!("token={}", lease.value);
        let scrubbed = broker.scrub_text(&sensitive);
        assert!(scrubbed.contains("token=***"));
        assert!(!scrubbed.contains(&lease.value));
    }

    #[test]
    fn consent_log_retains_events() {
        let log = ConsentLog::global();
        log.record(ConsentEvent::new("actor", "approve", "resource"));
        let events = log.recent(10);
        assert!(!events.is_empty());
        let event = events.last().unwrap();
        assert_eq!(event.actor, "actor");
        assert_eq!(event.action, "approve");
        assert_eq!(event.resource, "resource");
    }

    #[test]
    fn standard_limits_populated() {
        let limits = ResourceLimits::standard();
        assert!(limits.cpu_time_seconds.unwrap() >= 1);
        assert!(limits.memory_bytes.unwrap() >= 1024 * 1024);
    }

    #[test]
    fn audit_records_linked_by_hash() {
        let temp = TempDir::new().expect("tempdir");
        let ledger = AuditLedger::open(AuditLedgerConfig::for_path(temp.path().to_path_buf()))
            .expect("open ledger");

        let first = ledger
            .append(AuditEvent::new(
                AuditEventKind::Consent,
                "actor",
                "approve",
                "resource",
            ))
            .expect("append first");

        let second = ledger
            .append(
                AuditEvent::new(
                    AuditEventKind::SandboxExec,
                    "core:exec",
                    "exec_succeeded",
                    "resource",
                )
                .with_metadata("command", "ls"),
            )
            .expect("append second");

        assert_eq!(second.prev_hash, first.hash);
        assert!(second.sequence > first.sequence);
    }

    #[test]
    fn policy_evidence_expires_after_ttl() {
        let temp = TempDir::new().expect("tempdir");
        let ledger = AuditLedger::open(AuditLedgerConfig::for_path(temp.path().to_path_buf()))
            .expect("open ledger");

        let expired_time = Utc::now() - ChronoDuration::hours(25);
        ledger
            .append(
                AuditEvent::new(
                    AuditEventKind::SandboxExec,
                    "core:exec",
                    "exec_timeout",
                    "resource",
                )
                .with_timestamp(expired_time),
            )
            .expect("append record");

        let evidence = ledger.export_policy_evidence().expect("export evidence");
        assert!(evidence.is_empty());
    }
}

fn read_env_u64(name: &str) -> Option<u64> {
    env::var(name).ok()?.parse().ok()
}
