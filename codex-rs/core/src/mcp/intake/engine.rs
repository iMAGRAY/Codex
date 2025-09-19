//! High-level intake engine for MCP wizard.
//!
//! Trace: REQ-DATA-01 (explainable suggestions) and REQ-REL-01 (intake resilience).

use std::sync::Arc;

use anyhow::Context;
use anyhow::Result;

use super::cache::SignalCache;
use super::detectors::DetectorRegistry;
use super::fingerprint_store::FingerprintStatus;
use super::fingerprint_store::SharedFingerprintStore;
use super::parser::ParsedSource;
use super::parser::SourceParser;
use super::policy::evaluate_policy;
use super::preview::build_preview;
use super::reason_codes::ReasonCatalog;
use super::state::InsightSuggestion;
use super::state::IntakePhase;
use super::state::IntakeState;

/// High-level façade that orchestrates the intake process.
pub struct IntakeEngine {
    parser: SourceParser,
    cache: SignalCache,
    reasons: ReasonCatalog,
    fingerprints: Option<SharedFingerprintStore>,
    detectors: Arc<DetectorRegistry>,
}

impl IntakeEngine {
    pub fn new(
        parser: SourceParser,
        cache: SignalCache,
        reasons: ReasonCatalog,
        fingerprints: Option<SharedFingerprintStore>,
        detectors: Arc<DetectorRegistry>,
    ) -> Self {
        Self {
            parser,
            cache,
            reasons,
            fingerprints,
            detectors,
        }
    }

    pub fn parser(&self) -> &SourceParser {
        &self.parser
    }

    pub fn cache(&self) -> &SignalCache {
        &self.cache
    }

    pub fn reasons(&self) -> &ReasonCatalog {
        &self.reasons
    }

    pub fn detectors(&self) -> &DetectorRegistry {
        &self.detectors
    }

    pub fn parse_source(&self, raw_input: &str) -> Result<ParsedSource> {
        self.parser.parse(raw_input)
    }

    /// Populates state with the parsed source and reuses cached insights when available.
    pub fn begin_session(&self, state: &mut IntakeState, raw_input: &str) -> Result<()> {
        self.warmup();
        let parsed = self.parse_source(raw_input)?;
        let preview = build_preview(&parsed)?;
        state.set_preview(preview);

        let mut policy_warnings = evaluate_policy(&parsed, self.parser.cwd());
        if let Some(store) = &self.fingerprints {
            match store.0.lock() {
                Ok(mut guard) => match guard.record(&parsed) {
                    Ok(FingerprintStatus::Changed) => policy_warnings.push(
                        "Source fingerprint changed since the last analysis. Confirm that the differences are expected.".to_string(),
                    ),
                    Ok(_) => {}
                    Err(err) => {
                        tracing::warn!(error = %err, "Failed to update source fingerprint");
                    }
                },
                Err(err) => {
                    tracing::warn!(error = %err, "Failed to access fingerprint store");
                }
            }
        }
        state.set_policy_warnings(policy_warnings);

        if let Some(cached) = self.cache.get(&parsed.fingerprint().digest) {
            state.set_source(parsed);
            state.set_suggestions(cached.suggestions);
            state.advance_to(IntakePhase::Confirm);
            return Ok(());
        }
        state.set_source(parsed);
        Ok(())
    }

    /// Stores analysis results in state and persists them into cache.
    pub fn complete_analysis(&self, state: &mut IntakeState, suggestions: Vec<InsightSuggestion>) {
        if let Some(source) = state.source() {
            self.cache.put(source, suggestions.clone());
        }
        state.set_suggestions(suggestions);
    }

    pub fn warmup(&self) {
        self.cache.purge_expired();
        if let Some(store) = &self.fingerprints {
            match store.0.try_lock() {
                Ok(mut guard) => {
                    let _ = guard.purge_expired_and_save();
                }
                Err(_err) => {
                    // quietly skip if lock is held
                }
            }
        }
    }

    /// Execute all registered detectors, update the state, and cache results.
    pub fn analyze_with_detectors(
        &self,
        state: &mut IntakeState,
    ) -> Result<Vec<InsightSuggestion>> {
        let source = state
            .source()
            .context("intake source not initialised before detector run")?;
        let mut suggestions = self.detectors.run_all(source)?;
        if suggestions.is_empty() {
            return Ok(suggestions);
        }
        let path = source.path().to_string_lossy().to_string();
        suggestions = suggestions
            .into_iter()
            .map(|s| {
                if s.source_path().is_some() {
                    s
                } else {
                    s.with_source_path(path.clone())
                }
            })
            .collect();
        self.complete_analysis(state, suggestions.clone());
        Ok(suggestions)
    }

    /// Reload detectors whose backing artifacts changed.
    pub fn reload_detectors(&self) -> Result<usize> {
        self.detectors.reload_stale()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::intake::IntakeDetector;
    use crate::mcp::intake::reason_codes::ReasonCodeId;
    use crate::mcp::intake::state::ConfidenceLevel;
    use std::time::Duration;
    use tempfile::tempdir;

    fn reason_catalog() -> ReasonCatalog {
        let json = r#"{"codes":[{"id":"dummy","title":"Dummy","description":"test"}]}"#;
        ReasonCatalog::load_from_reader(json.as_bytes()).expect("catalog")
    }

    fn suggestion() -> InsightSuggestion {
        InsightSuggestion::new(
            "command",
            "npm run dev",
            ConfidenceLevel::High,
            ReasonCodeId::new("dummy"),
        )
        .with_notes("found in package.json")
    }

    #[test]
    fn begins_session_with_parsed_source() {
        let dir = tempdir().expect("tmp");
        let engine = IntakeEngine::new(
            SourceParser::new(None),
            SignalCache::new(Duration::from_secs(60)),
            reason_catalog(),
            None,
            Arc::new(DetectorRegistry::new()),
        );
        let mut state = IntakeState::new();
        engine
            .begin_session(&mut state, dir.path().to_str().unwrap())
            .expect("parsed");
        assert!(matches!(state.phase(), IntakePhase::Analysis));
        assert!(state.preview().is_some());
        assert_eq!(state.source().unwrap().path(), dir.path());
    }

    #[test]
    fn resumes_from_cache() {
        let dir = tempdir().expect("tmp");
        let parser = SourceParser::new(None);
        let source = parser.parse(dir.path().to_str().unwrap()).expect("parsed");
        let cache = SignalCache::new(Duration::from_secs(60));
        cache.put(&source, vec![suggestion()]);
        let engine = IntakeEngine::new(
            parser,
            cache,
            reason_catalog(),
            None,
            Arc::new(DetectorRegistry::new()),
        );
        let mut state = IntakeState::new();
        engine
            .begin_session(&mut state, dir.path().to_str().unwrap())
            .expect("cached");
        assert!(matches!(state.phase(), IntakePhase::Confirm));
        assert_eq!(state.suggestions().len(), 1);
    }

    #[test]
    fn stores_suggestions_in_cache() {
        let dir = tempdir().expect("tmp");
        let parser = SourceParser::new(None);
        let cache = SignalCache::new(Duration::from_secs(60));
        let engine = IntakeEngine::new(
            parser,
            cache.clone(),
            reason_catalog(),
            None,
            Arc::new(DetectorRegistry::new()),
        );
        let mut state = IntakeState::new();
        engine
            .begin_session(&mut state, dir.path().to_str().unwrap())
            .expect("start");
        engine.complete_analysis(&mut state, vec![suggestion()]);
        let digest = state.source().unwrap().fingerprint().digest.clone();
        assert!(cache.get(&digest).is_some());
    }

    #[test]
    fn analyze_with_detectors_populates_state_and_cache() {
        struct DemoDetector;

        impl IntakeDetector for DemoDetector {
            fn id(&self) -> &str {
                "demo.detector"
            }

            fn description(&self) -> &str {
                "Demo detector"
            }

            fn detect(&self, _source: &ParsedSource) -> Result<Vec<InsightSuggestion>> {
                Ok(vec![InsightSuggestion::new(
                    "command",
                    "npm run demo",
                    ConfidenceLevel::Medium,
                    ReasonCodeId::new("dummy"),
                )])
            }
        }

        let dir = tempdir().expect("tmp");
        let parser = SourceParser::new(None);
        let cache = SignalCache::new(Duration::from_secs(60));
        let detectors = Arc::new(DetectorRegistry::new());
        detectors
            .register_builtin(DemoDetector)
            .expect("register detector");
        let engine = IntakeEngine::new(parser, cache.clone(), reason_catalog(), None, detectors);
        let mut state = IntakeState::new();
        engine
            .begin_session(&mut state, dir.path().to_str().unwrap())
            .expect("session");
        let suggestions = engine.analyze_with_detectors(&mut state).expect("analyze");
        assert_eq!(suggestions.len(), 1);
        assert_eq!(state.suggestions().len(), 1);
        let digest = state.source().unwrap().fingerprint().digest.clone();
        let cached = cache.get(&digest).expect("cached suggestions");
        assert_eq!(cached.suggestions.len(), 1);
        assert!(cached.source_path.contains(dir.path().to_str().unwrap()));
    }
}
