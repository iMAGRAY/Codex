use serde::Deserialize;
use serde::Serialize;
use std::fmt;

/// Identifier for high-level Stellar actions. These strings are stable so that
/// keymaps, CLI invocations and telemetry can rely on them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StellarActionId {
    #[serde(rename = "core.navigate.next_pane")]
    NavigateNextPane,
    #[serde(rename = "core.navigate.prev_pane")]
    NavigatePrevPane,
    #[serde(rename = "core.palette.open")]
    OpenCommandPalette,
    #[serde(rename = "core.canvas.toggle")]
    ToggleCanvas,
    #[serde(rename = "core.overlay.telemetry")]
    ToggleTelemetryOverlay,
    #[serde(rename = "core.runbook.invoke")]
    RunbookInvoke,
    #[serde(rename = "core.input.undo")]
    InputUndo,
    #[serde(rename = "core.input.redo")]
    InputRedo,
    #[serde(rename = "core.input.field_lock")]
    FieldLockToggle,
    #[serde(rename = "core.input.confidence")]
    ToggleConfidencePanel,
    #[serde(rename = "core.input.submit")]
    SubmitInsight,
    #[serde(rename = "core.accessibility.toggle")]
    AccessibilityToggle,
    #[serde(rename = "core.conflict.open")]
    OpenConflictOverlay,
    #[serde(rename = "core.conflict.resolve")]
    ResolveConflict,
}

impl StellarActionId {
    pub const fn as_str(self) -> &'static str {
        match self {
            StellarActionId::NavigateNextPane => "core.navigate.next_pane",
            StellarActionId::NavigatePrevPane => "core.navigate.prev_pane",
            StellarActionId::OpenCommandPalette => "core.palette.open",
            StellarActionId::ToggleCanvas => "core.canvas.toggle",
            StellarActionId::ToggleTelemetryOverlay => "core.overlay.telemetry",
            StellarActionId::RunbookInvoke => "core.runbook.invoke",
            StellarActionId::InputUndo => "core.input.undo",
            StellarActionId::InputRedo => "core.input.redo",
            StellarActionId::FieldLockToggle => "core.input.field_lock",
            StellarActionId::ToggleConfidencePanel => "core.input.confidence",
            StellarActionId::SubmitInsight => "core.input.submit",
            StellarActionId::AccessibilityToggle => "core.accessibility.toggle",
            StellarActionId::OpenConflictOverlay => "core.conflict.open",
            StellarActionId::ResolveConflict => "core.conflict.resolve",
        }
    }
}

impl fmt::Display for StellarActionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Concrete action invocation possibly carrying additional payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "id", content = "payload", rename_all = "kebab-case")]
pub enum StellarAction {
    #[serde(rename = "core.navigate.next_pane")]
    NavigateNextPane,
    #[serde(rename = "core.navigate.prev_pane")]
    NavigatePrevPane,
    #[serde(rename = "core.palette.open")]
    OpenCommandPalette,
    #[serde(rename = "core.canvas.toggle")]
    ToggleCanvas,
    #[serde(rename = "core.overlay.telemetry")]
    ToggleTelemetryOverlay,
    #[serde(rename = "core.runbook.invoke")]
    RunbookInvoke { runbook_id: Option<String> },
    #[serde(rename = "core.input.undo")]
    InputUndo,
    #[serde(rename = "core.input.redo")]
    InputRedo,
    #[serde(rename = "core.input.field_lock")]
    FieldLockToggle,
    #[serde(rename = "core.input.confidence")]
    ToggleConfidencePanel,
    #[serde(rename = "core.input.submit")]
    SubmitInsight { text: Option<String> },
    #[serde(rename = "core.accessibility.toggle")]
    AccessibilityToggle,
    #[serde(rename = "core.conflict.open")]
    OpenConflictOverlay,
    #[serde(rename = "core.conflict.resolve")]
    ResolveConflict {
        conflict_id: Option<uuid::Uuid>,
        decision: crate::resilience::ConflictDecision,
    },
}

impl StellarAction {
    pub fn id(&self) -> StellarActionId {
        match self {
            StellarAction::NavigateNextPane => StellarActionId::NavigateNextPane,
            StellarAction::NavigatePrevPane => StellarActionId::NavigatePrevPane,
            StellarAction::OpenCommandPalette => StellarActionId::OpenCommandPalette,
            StellarAction::ToggleCanvas => StellarActionId::ToggleCanvas,
            StellarAction::ToggleTelemetryOverlay => StellarActionId::ToggleTelemetryOverlay,
            StellarAction::RunbookInvoke { .. } => StellarActionId::RunbookInvoke,
            StellarAction::InputUndo => StellarActionId::InputUndo,
            StellarAction::InputRedo => StellarActionId::InputRedo,
            StellarAction::FieldLockToggle => StellarActionId::FieldLockToggle,
            StellarAction::ToggleConfidencePanel => StellarActionId::ToggleConfidencePanel,
            StellarAction::SubmitInsight { .. } => StellarActionId::SubmitInsight,
            StellarAction::AccessibilityToggle => StellarActionId::AccessibilityToggle,
            StellarAction::OpenConflictOverlay => StellarActionId::OpenConflictOverlay,
            StellarAction::ResolveConflict { .. } => StellarActionId::ResolveConflict,
        }
    }
}
