//! Detector registry with optional hot-reload behaviour for MCP intake.
//!
//! Trace: REQ-DATA-01 (detector explainability) and REQ-REL-01 (resilient signal sourcing).

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::RwLock;
use std::time::SystemTime;

use anyhow::Context;
use anyhow::Result;

use super::parser::ParsedSource;
use super::state::InsightSuggestion;

/// Trait implemented by individual intake detectors.
pub trait IntakeDetector: Send + Sync {
    /// Stable identifier for tracing and debugging.
    fn id(&self) -> &str;
    /// Short human-readable description.
    fn description(&self) -> &str;
    /// Produce insight suggestions from the provided source.
    fn detect(&self, source: &ParsedSource) -> Result<Vec<InsightSuggestion>>;
}

/// Loader function used for hot-reloadable detectors.
type DetectorLoader = Arc<dyn Fn(&Path) -> Result<Box<dyn IntakeDetector>> + Send + Sync>;

struct ReloadableDetector {
    path: PathBuf,
    loader: DetectorLoader,
    last_seen: Option<SystemTime>,
}

struct RegistryEntry {
    detector: Arc<dyn IntakeDetector>,
    reloadable: Option<ReloadableDetector>,
}

/// Registry of available detectors with optional hot reload support.
#[derive(Default)]
pub struct DetectorRegistry {
    entries: RwLock<HashMap<String, RegistryEntry>>,
}

impl DetectorRegistry {
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
        }
    }

    /// Register a built-in detector that does not support hot reload.
    pub fn register_builtin<D>(&self, detector: D) -> Result<()>
    where
        D: IntakeDetector + 'static,
    {
        let id = detector.id().to_string();
        let entry = RegistryEntry {
            detector: Arc::new(detector),
            reloadable: None,
        };
        let mut guard = self.entries.write().expect("detector registry lock");
        guard.insert(id, entry);
        Ok(())
    }

    /// Register a detector loaded from a file path with reload support.
    pub fn register_hot_reload<P>(&self, path: P, loader: DetectorLoader) -> Result<()>
    where
        P: Into<PathBuf>,
    {
        let path = path.into();
        let detector = loader(&path)?;
        let id = detector.id().to_string();
        let metadata = fs::metadata(&path)
            .with_context(|| format!("Failed to read detector metadata for {}", path.display()))?;
        let last_seen = metadata.modified().ok();
        let entry = RegistryEntry {
            detector: Arc::from(detector),
            reloadable: Some(ReloadableDetector {
                path,
                loader,
                last_seen,
            }),
        };
        let mut guard = self.entries.write().expect("detector registry lock");
        guard.insert(id, entry);
        Ok(())
    }

    /// Reload detectors whose backing files changed.
    /// Returns the number of detectors reloaded.
    pub fn reload_stale(&self) -> Result<usize> {
        let mut reloaded = 0usize;
        let mut guard = self.entries.write().expect("detector registry lock");
        for entry in guard.values_mut() {
            let Some(reloadable) = entry.reloadable.as_mut() else {
                continue;
            };
            let metadata = match fs::metadata(&reloadable.path) {
                Ok(meta) => meta,
                Err(err) => {
                    tracing::warn!(
                        error = %err,
                        path = %reloadable.path.display(),
                        "Failed to stat detector file"
                    );
                    continue;
                }
            };
            let modified = metadata.modified().ok();
            if modified.is_some() && modified == reloadable.last_seen {
                continue;
            }
            match (reloadable.loader)(&reloadable.path) {
                Ok(detector) => {
                    entry.detector = Arc::from(detector);
                    reloadable.last_seen = modified;
                    reloaded += 1;
                }
                Err(err) => {
                    tracing::warn!(
                        error = %err,
                        path = %reloadable.path.display(),
                        "Failed to reload detector"
                    );
                }
            }
        }
        Ok(reloaded)
    }

    /// Execute all registered detectors and collect their suggestions.
    pub fn run_all(&self, source: &ParsedSource) -> Result<Vec<InsightSuggestion>> {
        let guard = self.entries.read().expect("detector registry lock");
        let mut aggregate = Vec::new();
        for entry in guard.values() {
            match entry.detector.detect(source) {
                Ok(mut suggestions) => aggregate.append(&mut suggestions),
                Err(err) => {
                    tracing::warn!(
                        error = %err,
                        detector = entry.detector.id(),
                        "Detector execution failed"
                    );
                }
            }
        }
        Ok(aggregate)
    }

    /// Returns the number of registered detectors.
    pub fn len(&self) -> usize {
        self.entries.read().expect("detector registry lock").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::intake::parser::SourceFingerprint;
    use crate::mcp::intake::parser::SourceKind;
    use std::path::PathBuf;

    struct ConstantDetector {
        id: &'static str,
        description: &'static str,
    }

    impl IntakeDetector for ConstantDetector {
        fn id(&self) -> &str {
            self.id
        }

        fn description(&self) -> &str {
            self.description
        }

        fn detect(&self, _source: &ParsedSource) -> Result<Vec<InsightSuggestion>> {
            Ok(vec![InsightSuggestion::new(
                "field",
                "value",
                super::super::state::ConfidenceLevel::High,
                super::super::reason_codes::ReasonCodeId::new("constant"),
            )])
        }
    }

    fn sample_source() -> ParsedSource {
        ParsedSource::new(
            PathBuf::from("/tmp"),
            SourceKind::Directory,
            SourceFingerprint {
                digest: "abc".into(),
                last_modified: None,
            },
        )
    }

    #[test]
    fn registers_and_runs_builtin_detector() {
        let registry = DetectorRegistry::new();
        registry
            .register_builtin(ConstantDetector {
                id: "constant",
                description: "Constant detector",
            })
            .expect("register");
        let suggestions = registry.run_all(&sample_source()).expect("run");
        assert_eq!(suggestions.len(), 1);
    }

    #[test]
    fn reloads_changed_detector() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("detector.json");
        std::fs::write(&file, "first").expect("write");

        let calls = Arc::new(std::sync::Mutex::new(0u32));
        let calls_clone = calls.clone();
        let loader: DetectorLoader = Arc::new(move |_path| {
            let mut guard = calls_clone.lock().unwrap();
            *guard += 1;
            Ok(Box::new(ConstantDetector {
                id: "dynamic",
                description: "Dynamic detector",
            }))
        });

        let registry = DetectorRegistry::new();
        registry
            .register_hot_reload(&file, loader.clone())
            .expect("register");
        assert_eq!(registry.reload_stale().expect("reload"), 0);

        std::thread::sleep(std::time::Duration::from_millis(10));
        std::fs::write(&file, "second").expect("write");
        assert_eq!(registry.reload_stale().expect("reload"), 1);
        assert!(*calls.lock().unwrap() >= 2);
    }
}
