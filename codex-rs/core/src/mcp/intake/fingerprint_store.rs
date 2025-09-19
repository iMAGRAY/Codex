//! Fingerprint persistence for MCP intake sources.
//!
//! Trace: REQ-REL-01 (resilience for wizard offline cache).

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use std::time::SystemTime;

use anyhow::Context;
use anyhow::Result;
use serde::Deserialize;
use serde::Serialize;

use super::parser::ParsedSource;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredFingerprint {
    digest: String,
    last_seen: SystemTime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FingerprintStatus {
    Fresh,
    Unchanged,
    Changed,
}

#[derive(Debug)]
pub struct FingerprintStore {
    path: PathBuf,
    ttl: Duration,
    entries: HashMap<String, StoredFingerprint>,
}

impl FingerprintStore {
    pub fn load(path: PathBuf, ttl: Duration) -> Result<Self> {
        let entries = if path.exists() {
            let data = fs::read(&path)
                .with_context(|| format!("Failed to read fingerprint file: {}", path.display()))?;
            if data.is_empty() {
                HashMap::new()
            } else {
                serde_json::from_slice::<HashMap<String, StoredFingerprint>>(&data).with_context(
                    || format!("Invalid fingerprint file format: {}", path.display()),
                )?
            }
        } else {
            if let Some(dir) = path.parent() {
                fs::create_dir_all(dir).with_context(|| {
                    format!("Failed to create fingerprint directory: {}", dir.display())
                })?;
            }
            HashMap::new()
        };
        Ok(Self { path, ttl, entries })
    }

    pub fn record(&mut self, parsed: &ParsedSource) -> Result<FingerprintStatus> {
        self.purge_expired();
        let canonical = parsed
            .path()
            .canonicalize()
            .unwrap_or_else(|_| parsed.path().to_path_buf());
        let key = canonical.to_string_lossy().to_string();
        let now = SystemTime::now();
        let digest = parsed.fingerprint().digest.clone();

        let status = match self.entries.get(&key) {
            Some(existing) if existing.digest == digest => FingerprintStatus::Unchanged,
            Some(_) => FingerprintStatus::Changed,
            None => FingerprintStatus::Fresh,
        };

        self.entries.insert(
            key,
            StoredFingerprint {
                digest,
                last_seen: now,
            },
        );
        self.save()?;
        Ok(status)
    }

    pub fn purge_expired(&mut self) {
        let ttl = self.ttl;
        self.entries.retain(|_, entry| {
            entry
                .last_seen
                .elapsed()
                .map(|elapsed| elapsed <= ttl)
                .unwrap_or(true)
        });
    }

    pub fn purge_expired_and_save(&mut self) -> Result<()> {
        self.purge_expired();
        self.save()
    }

    fn save(&self) -> Result<()> {
        let data = serde_json::to_vec_pretty(&self.entries)
            .context("Failed to serialise source fingerprints")?;
        fs::write(&self.path, data)
            .with_context(|| format!("Failed to write fingerprint file: {}", self.path.display()))
    }
}

#[derive(Debug, Clone)]
pub struct SharedFingerprintStore(pub Arc<Mutex<FingerprintStore>>);

impl SharedFingerprintStore {
    pub fn new(inner: FingerprintStore) -> Self {
        Self(Arc::new(Mutex::new(inner)))
    }
}

#[cfg(test)]
mod tests {
    use super::super::parser::SourceFingerprint;
    use super::super::parser::SourceKind;
    use super::*;
    use std::path::Path;
    use tempfile::tempdir;

    fn parsed(path: &Path) -> ParsedSource {
        ParsedSource::new(
            path.to_path_buf(),
            SourceKind::Directory,
            SourceFingerprint {
                digest: "abc".to_string(),
                last_modified: None,
            },
        )
    }

    #[test]
    fn records_and_detects_changes() {
        let dir = tempdir().expect("tmp");
        let file = dir.path().join("store.json");
        let mut store = FingerprintStore::load(file, Duration::from_secs(60)).expect("store");
        let source = parsed(dir.path());
        let status = store.record(&source).expect("record");
        assert!(matches!(status, FingerprintStatus::Fresh));
        let status = store.record(&source).expect("record");
        assert!(matches!(status, FingerprintStatus::Unchanged));
    }

    #[test]
    fn detects_change() {
        let dir = tempdir().expect("tmp");
        let file = dir.path().join("store.json");
        let mut store = FingerprintStore::load(file, Duration::from_secs(60)).expect("store");
        let mut source = parsed(dir.path());
        store.record(&source).expect("record");
        source = ParsedSource::new(
            dir.path().to_path_buf(),
            SourceKind::Directory,
            SourceFingerprint {
                digest: "changed".to_string(),
                last_modified: None,
            },
        );
        let status = store.record(&source).expect("record");
        assert!(matches!(status, FingerprintStatus::Changed));
    }

    #[test]
    fn purges_expired_entries() {
        let dir = tempdir().expect("tmp");
        let file = dir.path().join("store.json");
        let mut store = FingerprintStore::load(file, Duration::from_millis(1)).expect("store");
        let source = parsed(dir.path());
        store.record(&source).expect("record");
        std::thread::sleep(Duration::from_millis(5));
        store.purge_expired();
        assert!(store.entries.is_empty());
    }
}
