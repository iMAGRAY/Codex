use crate::stellar::action::StellarActionId;
use crate::stellar::persona::StellarPersona;
use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LayoutMode {
    Wide,
    Compact,
}

impl Default for LayoutMode {
    fn default() -> Self {
        LayoutMode::Wide
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PaneFocus {
    InsightCanvas,
    Telemetry,
    CommandLog,
    Runbook,
    FooterSubmit,
    FooterOverlay,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoldenPathHint {
    pub label: String,
    pub description: String,
    pub action: StellarActionId,
    pub shortcut: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunbookShortcut {
    pub id: String,
    pub title: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfidenceSnapshot {
    pub score: f32,
    pub trend: String,
    pub reasons: Vec<String>,
    pub visible: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum RiskSeverity {
    Info,
    Warning,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskAlert {
    pub message: String,
    pub severity: RiskSeverity,
}

impl RiskAlert {
    pub fn info(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            severity: RiskSeverity::Info,
        }
    }

    pub fn warning(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            severity: RiskSeverity::Warning,
        }
    }

    pub fn critical(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            severity: RiskSeverity::Critical,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KernelSnapshot {
    pub persona: StellarPersona,
    pub assistive_mode: bool,
    pub layout_mode: LayoutMode,
    pub canvas_visible: bool,
    pub telemetry_visible: bool,
    pub focus: PaneFocus,
    pub field_text: String,
    pub field_locked: bool,
    pub suggestions: Vec<String>,
    pub command_log: Vec<String>,
    pub runbook_shortcuts: Vec<RunbookShortcut>,
    pub golden_path: Vec<GoldenPathHint>,
    pub confidence: Option<ConfidenceSnapshot>,
    pub status_messages: Vec<String>,
    pub risk_alerts: Vec<RiskAlert>,
}

impl KernelSnapshot {
    pub fn empty(persona: StellarPersona) -> Self {
        Self {
            persona,
            assistive_mode: false,
            layout_mode: LayoutMode::Wide,
            canvas_visible: false,
            telemetry_visible: false,
            focus: PaneFocus::InsightCanvas,
            field_text: String::new(),
            field_locked: false,
            suggestions: Vec::new(),
            command_log: Vec::new(),
            runbook_shortcuts: Vec::new(),
            golden_path: Vec::new(),
            confidence: None,
            status_messages: Vec::new(),
            risk_alerts: Vec::new(),
        }
    }
}
