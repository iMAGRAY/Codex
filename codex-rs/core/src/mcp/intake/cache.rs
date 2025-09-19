use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use std::time::SystemTime;

use super::parser::ParsedSource;
use super::parser::SourceFingerprint;
use super::state::InsightSuggestion;

#[derive(Debug, Clone)]
pub struct CachedSignals {
    pub fingerprint: SourceFingerprint,
    pub source_path: String,
    pub stored_at: SystemTime,
    pub suggestions: Vec<InsightSuggestion>,
}

#[derive(Debug, Clone)]
pub struct SignalCache {
    inner: Arc<Mutex<HashMap<String, CachedSignals>>>,
    ttl: Duration,
}

impl SignalCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            ttl,
        }
    }

    pub fn get(&self, digest: &str) -> Option<CachedSignals> {
        let mut guard = self.inner.lock().expect("signal cache poisoned");
        if let Some(entry) = guard.get(digest) {
            if self.is_expired(entry) {
                guard.remove(digest);
                return None;
            }
            return Some(entry.clone());
        }
        None
    }

    pub fn put(&self, source: &ParsedSource, suggestions: Vec<InsightSuggestion>) {
        let mut guard = self.inner.lock().expect("signal cache poisoned");
        let fingerprint = source.fingerprint().clone();
        let entry = CachedSignals {
            fingerprint: fingerprint.clone(),
            source_path: source.path().to_string_lossy().into_owned(),
            stored_at: SystemTime::now(),
            suggestions,
        };
        guard.insert(fingerprint.digest.clone(), entry);
        self.retain_recent_locked(&mut guard);
    }

    pub fn purge_expired(&self) {
        let mut guard = self.inner.lock().expect("signal cache poisoned");
        self.retain_recent_locked(&mut guard);
    }

    fn retain_recent_locked(&self, map: &mut HashMap<String, CachedSignals>) {
        let ttl = self.ttl;
        map.retain(|_, entry| !self.is_expired_with(entry, ttl));
    }

    fn is_expired(&self, entry: &CachedSignals) -> bool {
        self.is_expired_with(entry, self.ttl)
    }

    fn is_expired_with(&self, entry: &CachedSignals, ttl: Duration) -> bool {
        match entry.stored_at.elapsed() {
            Ok(elapsed) => elapsed > ttl,
            Err(_) => false,
        }
    }
}

impl Default for SignalCache {
    fn default() -> Self {
        Self::new(Duration::from_secs(15 * 60))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    fn dummy_fingerprint() -> SourceFingerprint {
        SourceFingerprint {
            digest: "abc".into(),
            last_modified: None,
        }
    }

    fn parsed_source() -> crate::mcp::intake::parser::ParsedSource {
        crate::mcp::intake::parser::ParsedSource::new(
            std::path::PathBuf::from("/tmp/source"),
            crate::mcp::intake::parser::SourceKind::Directory,
            dummy_fingerprint(),
        )
    }

    fn suggestion() -> InsightSuggestion {
        InsightSuggestion::new(
            "field",
            "value",
            crate::mcp::intake::state::ConfidenceLevel::High,
            crate::mcp::intake::reason_codes::ReasonCodeId::new("test"),
        )
    }

    #[test]
    fn stores_and_retrieves() {
        let cache = SignalCache::new(Duration::from_secs(60));
        cache.put(&parsed_source(), vec![suggestion()]);
        let entry = cache.get("abc").expect("cached");
        assert_eq!(entry.fingerprint.digest, "abc");
        assert_eq!(entry.suggestions.len(), 1);
        assert!(entry.source_path.ends_with("source"));
    }

    #[test]
    fn expires_entries() {
        let cache = SignalCache::new(Duration::from_millis(1));
        cache.put(&parsed_source(), vec![suggestion()]);
        thread::sleep(Duration::from_millis(5));
        assert!(cache.get("abc").is_none());
    }
}
