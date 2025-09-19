use crate::stellar::action::StellarAction;
use crate::stellar::action::StellarActionId;
use crate::stellar::persona::StellarPersona;
use serde::Deserialize;
use serde::Serialize;
use thiserror::Error;

/// Snapshot of mutable state relevant for guard decisions.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct GuardContext {
    pub persona: StellarPersona,
    pub assistive_mode: bool,
    pub field_locked: bool,
    pub field_empty: bool,
    pub undo_available: bool,
    pub redo_available: bool,
}

#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize)]
pub struct InputGuard {
    /// Allow submitting empty insights (disabled by default).
    pub allow_empty_submit: bool,
}

#[derive(Debug, Error, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "message")]
pub enum GuardError {
    #[error("action '{0}' blocked because the insight field is locked")]
    FieldLocked(StellarActionId),
    #[error("cannot submit an empty insight")]
    EmptySubmission,
    #[error("nothing to undo")]
    UndoUnavailable,
    #[error("nothing to redo")]
    RedoUnavailable,
    #[error("persona '{persona}' cannot invoke action '{action}' (REQ-SEC-01)")]
    PersonaDenied {
        persona: StellarPersona,
        action: StellarActionId,
    },
}

impl InputGuard {
    pub fn validate(&self, action: &StellarAction, ctx: GuardContext) -> Result<(), GuardError> {
        let action_id = action.id();
        match action {
            StellarAction::SubmitInsight { .. } => {
                if ctx.field_locked {
                    return Err(GuardError::FieldLocked(action.id()));
                }
                if !self.allow_empty_submit && ctx.field_empty {
                    return Err(GuardError::EmptySubmission);
                }
            }
            StellarAction::InputUndo => {
                if !ctx.undo_available {
                    return Err(GuardError::UndoUnavailable);
                }
            }
            StellarAction::InputRedo => {
                if !ctx.redo_available {
                    return Err(GuardError::RedoUnavailable);
                }
            }
            StellarAction::FieldLockToggle
            | StellarAction::ToggleTelemetryOverlay
            | StellarAction::ToggleCanvas
            | StellarAction::ToggleConfidencePanel
            | StellarAction::OpenCommandPalette
            | StellarAction::RunbookInvoke { .. }
            | StellarAction::NavigateNextPane
            | StellarAction::NavigatePrevPane
            | StellarAction::AccessibilityToggle
            | StellarAction::OpenConflictOverlay
            | StellarAction::ResolveConflict { .. } => {}
        }
        // Persona-scoped RBAC (REQ-SEC-01) blocks actions outside the matrix
        // defined in docs/future/stellar/alignment.md.
        if !crate::stellar::is_action_allowed(ctx.persona, action_id) {
            return Err(GuardError::PersonaDenied {
                persona: ctx.persona,
                action: action_id,
            });
        }
        Ok(())
    }
}
