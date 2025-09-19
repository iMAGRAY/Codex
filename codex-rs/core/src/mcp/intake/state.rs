//! Intake state machine for MCP wizard recommendations.
//!
//! Trace: REQ-DATA-01 (data confidence) and REQ-REL-01 (resilience of wizard insights).

use std::time::SystemTime;

use super::parser::ParsedSource;
use super::reason_codes::ReasonCodeId;

/// Quick summary of the selected source that is shown before analysis.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SourcePreview {
    files: Vec<String>,
    warnings: Vec<String>,
}

impl SourcePreview {
    pub fn new(files: Vec<String>, warnings: Vec<String>) -> Self {
        Self { files, warnings }
    }

    pub fn files(&self) -> &[String] {
        &self.files
    }

    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }
}

/// Lifecycle phases that the intake engine traverses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IntakePhase {
    /// User enters or confirms the source path.
    SourceContext,
    /// Engine analyses the source structure and prepares signals.
    Analysis,
    /// User reviews generated recommendations with explanations.
    Insight,
    /// Final verification of configuration parameters before applying.
    Confirm,
    /// Optional follow-up actions (for example, sandbox health-check).
    Activation,
}

impl Default for IntakePhase {
    fn default() -> Self {
        IntakePhase::SourceContext
    }
}

/// Confidence levels assigned to generated recommendations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConfidenceLevel {
    High,
    Medium,
    Low,
    /// Used until scoring assigns a concrete level.
    Unknown,
}

impl Default for ConfidenceLevel {
    fn default() -> Self {
        ConfidenceLevel::Unknown
    }
}

/// Explainable recommendation for an MCP configuration field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InsightSuggestion {
    field: String,
    value: String,
    confidence: ConfidenceLevel,
    reason: ReasonCodeId,
    source_path: Option<String>,
    notes: Option<String>,
}

impl InsightSuggestion {
    pub fn new<S: Into<String>>(
        field: S,
        value: S,
        confidence: ConfidenceLevel,
        reason: ReasonCodeId,
    ) -> Self {
        Self {
            field: field.into(),
            value: value.into(),
            confidence,
            reason,
            source_path: None,
            notes: None,
        }
    }

    pub fn with_source_path(mut self, path: impl Into<String>) -> Self {
        self.source_path = Some(path.into());
        self
    }

    pub fn with_notes(mut self, notes: impl Into<String>) -> Self {
        self.notes = Some(notes.into());
        self
    }

    pub fn field(&self) -> &str {
        &self.field
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn confidence(&self) -> ConfidenceLevel {
        self.confidence
    }

    pub fn reason(&self) -> &ReasonCodeId {
        &self.reason
    }

    pub fn source_path(&self) -> Option<&str> {
        self.source_path.as_deref()
    }

    pub fn notes(&self) -> Option<&str> {
        self.notes.as_deref()
    }
}

/// Central intake state shared between wizard steps.
#[derive(Debug)]
pub struct IntakeState {
    phase: IntakePhase,
    source: Option<ParsedSource>,
    preview: Option<SourcePreview>,
    policy_warnings: Vec<String>,
    suggestions: Vec<InsightSuggestion>,
    last_updated: SystemTime,
}

impl IntakeState {
    pub fn new() -> Self {
        Self {
            phase: IntakePhase::SourceContext,
            source: None,
            preview: None,
            policy_warnings: Vec::new(),
            suggestions: Vec::new(),
            last_updated: SystemTime::now(),
        }
    }

    pub fn phase(&self) -> IntakePhase {
        self.phase
    }

    pub fn source(&self) -> Option<&ParsedSource> {
        self.source.as_ref()
    }

    pub fn preview(&self) -> Option<&SourcePreview> {
        self.preview.as_ref()
    }

    pub fn policy_warnings(&self) -> &[String] {
        &self.policy_warnings
    }

    pub fn suggestions(&self) -> &[InsightSuggestion] {
        &self.suggestions
    }

    pub fn last_updated(&self) -> SystemTime {
        self.last_updated
    }

    pub fn reset(&mut self) {
        self.phase = IntakePhase::SourceContext;
        self.source = None;
        self.preview = None;
        self.policy_warnings.clear();
        self.suggestions.clear();
        self.touch();
    }

    pub fn set_source(&mut self, source: ParsedSource) {
        self.source = Some(source);
        self.phase = IntakePhase::Analysis;
        self.touch();
    }

    pub fn set_preview(&mut self, preview: SourcePreview) {
        self.preview = Some(preview);
        self.touch();
    }

    pub fn set_policy_warnings(&mut self, warnings: Vec<String>) {
        self.policy_warnings = warnings;
        self.touch();
    }

    pub fn set_suggestions(&mut self, suggestions: Vec<InsightSuggestion>) {
        self.suggestions = suggestions;
        self.phase = IntakePhase::Insight;
        self.touch();
    }

    pub fn advance_to(&mut self, phase: IntakePhase) {
        if phase as u8 >= self.phase as u8 {
            self.phase = phase;
            self.touch();
        }
    }

    fn touch(&mut self) {
        self.last_updated = SystemTime::now();
    }
}

impl Default for IntakeState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::intake::parser::SourceFingerprint;
    use crate::mcp::intake::parser::SourceKind;
    use std::path::PathBuf;

    fn parsed_source() -> ParsedSource {
        ParsedSource::new(
            PathBuf::from("/tmp"),
            SourceKind::Directory,
            SourceFingerprint {
                digest: "abc".into(),
                last_modified: None,
            },
        )
    }

    fn suggestion(field: &str, value: &str) -> InsightSuggestion {
        InsightSuggestion::new(
            field,
            value,
            ConfidenceLevel::Medium,
            ReasonCodeId::new("reason"),
        )
    }

    #[test]
    fn transitions_between_phases() {
        let mut state = IntakeState::new();
        assert_eq!(state.phase(), IntakePhase::SourceContext);
        state.set_source(parsed_source());
        assert_eq!(state.phase(), IntakePhase::Analysis);
        state.set_suggestions(vec![suggestion("command", "run")]);
        assert_eq!(state.phase(), IntakePhase::Insight);
        state.advance_to(IntakePhase::Confirm);
        assert_eq!(state.phase(), IntakePhase::Confirm);
        state.set_policy_warnings(vec!["warning".into()]);
        assert_eq!(state.policy_warnings().len(), 1);
        state.set_preview(SourcePreview::new(vec!["file".into()], vec![]));
        assert!(state.preview().is_some());
        state.reset();
        assert_eq!(state.phase(), IntakePhase::SourceContext);
        assert!(state.source().is_none());
        assert!(state.preview().is_none());
    }
}
