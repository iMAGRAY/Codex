use anyhow::Result;
use anyhow::bail;
use clap::Parser;
use clap::Subcommand;
use clap::ValueEnum;
use codex_core::resilience::ConflictDecision;
use codex_core::resilience::ConflictId;
use codex_core::stellar::StellarAction;
use codex_core::stellar::StellarCliEvent;
use codex_core::stellar::StellarPersona;
use codex_core::stellar::is_action_allowed;
use serde::Serialize;

#[derive(Debug, Parser)]
pub struct StellarCli {
    /// Persona context to scope role-based keymaps and guardrails.
    #[arg(
        long = "persona",
        value_name = "PERSONA",
        default_value = "operator",
        value_parser = parse_persona
    )]
    persona: StellarPersona,

    #[command(subcommand)]
    command: StellarSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum StellarSubcommand {
    /// Navigate between Insight Canvas panes.
    Focus {
        #[arg(value_enum, default_value_t = FocusDirection::Next)]
        direction: FocusDirection,
    },
    /// Open the command palette.
    Palette,
    /// Toggle the Insight Canvas visibility.
    Canvas,
    /// Toggle the telemetry overlay.
    Telemetry,
    /// Invoke a runbook by id (defaults to RB-01).
    Runbook {
        #[arg(value_name = "RUNBOOK_ID")]
        id: Option<String>,
    },
    /// Undo the last Insight action.
    Undo,
    /// Redo the last reverted Insight action.
    Redo,
    /// Toggle field lock for the Insight input.
    FieldLock,
    /// Toggle the Confidence panel visibility.
    Confidence,
    /// Submit the current insight text (optional override provided via argument).
    Submit {
        #[arg(value_name = "TEXT")]
        text: Option<String>,
    },
    /// Toggle accessibility/assistive mode.
    Accessibility,
    /// Open the conflict resolver overlay.
    #[clap(name = "conflicts-open")]
    ConflictsOpen,
    /// Resolve a pending conflict (defaults to the oldest pending entry).
    #[clap(name = "conflicts-resolve")]
    ConflictsResolve {
        #[arg(long = "decision", value_enum, default_value_t = ConflictDecisionArg::Accept)]
        decision: ConflictDecisionArg,
        #[arg(long = "id")]
        id: Option<ConflictId>,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum FocusDirection {
    Next,
    Prev,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ConflictDecisionArg {
    Accept,
    Reject,
    Auto,
}

pub fn run(cli: StellarCli) -> Result<()> {
    let action = cli.command.into_action();
    if !is_action_allowed(cli.persona, action.id()) {
        bail!(
            "persona '{}' cannot invoke '{}' (REQ-SEC-01)",
            cli.persona,
            action.id()
        );
    }
    let event = StellarCliEvent::new(cli.persona, action);
    print_json(event)?;
    Ok(())
}

impl StellarSubcommand {
    fn into_action(self) -> StellarAction {
        match self {
            StellarSubcommand::Focus { direction } => match direction {
                FocusDirection::Next => StellarAction::NavigateNextPane,
                FocusDirection::Prev => StellarAction::NavigatePrevPane,
            },
            StellarSubcommand::Palette => StellarAction::OpenCommandPalette,
            StellarSubcommand::Canvas => StellarAction::ToggleCanvas,
            StellarSubcommand::Telemetry => StellarAction::ToggleTelemetryOverlay,
            StellarSubcommand::Runbook { id } => StellarAction::RunbookInvoke { runbook_id: id },
            StellarSubcommand::Undo => StellarAction::InputUndo,
            StellarSubcommand::Redo => StellarAction::InputRedo,
            StellarSubcommand::FieldLock => StellarAction::FieldLockToggle,
            StellarSubcommand::Confidence => StellarAction::ToggleConfidencePanel,
            StellarSubcommand::Submit { text } => StellarAction::SubmitInsight { text },
            StellarSubcommand::Accessibility => StellarAction::AccessibilityToggle,
            StellarSubcommand::ConflictsOpen => StellarAction::OpenConflictOverlay,
            StellarSubcommand::ConflictsResolve { decision, id } => {
                StellarAction::ResolveConflict {
                    conflict_id: id,
                    decision: decision.into(),
                }
            }
        }
    }
}

fn print_json<T: Serialize>(value: T) -> Result<()> {
    let rendered = serde_json::to_string_pretty(&value)?;
    println!("{rendered}");
    Ok(())
}

fn parse_persona(value: &str) -> Result<StellarPersona, String> {
    match value.to_ascii_lowercase().as_str() {
        "operator" => Ok(StellarPersona::Operator),
        "sre" => Ok(StellarPersona::Sre),
        "secops" => Ok(StellarPersona::SecOps),
        "platform-engineer" | "platform" => Ok(StellarPersona::PlatformEngineer),
        "partner-developer" | "partner" => Ok(StellarPersona::PartnerDeveloper),
        "assistive-bridge" | "assistive" => Ok(StellarPersona::AssistiveBridge),
        other => Err(format!("unknown persona '{other}'")),
    }
}

impl From<ConflictDecisionArg> for ConflictDecision {
    fn from(value: ConflictDecisionArg) -> Self {
        match value {
            ConflictDecisionArg::Accept => ConflictDecision::Accept,
            ConflictDecisionArg::Reject => ConflictDecision::Reject,
            ConflictDecisionArg::Auto => ConflictDecision::Auto,
        }
    }
}
