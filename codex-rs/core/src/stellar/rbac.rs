//! RBAC matrices for Stellar personas.
//!
//! Trace: REQ-SEC-01 (#9, #11, #74) — persona-scoped guardrails preventing
//! unauthorized command execution across TUI/CLI surfaces.

use crate::stellar::action::StellarActionId;
use crate::stellar::persona::StellarPersona;

/// Returns `true` when the given persona is permitted to invoke the specified
/// Stellar action.
#[must_use]
pub fn is_action_allowed(persona: StellarPersona, action: StellarActionId) -> bool {
    match persona {
        StellarPersona::Sre | StellarPersona::SecOps | StellarPersona::PlatformEngineer => true,
        StellarPersona::Operator | StellarPersona::AssistiveBridge => matches!(
            action,
            StellarActionId::NavigateNextPane
                | StellarActionId::NavigatePrevPane
                | StellarActionId::OpenCommandPalette
                | StellarActionId::ToggleCanvas
                | StellarActionId::ToggleTelemetryOverlay
                | StellarActionId::RunbookInvoke
                | StellarActionId::InputUndo
                | StellarActionId::InputRedo
                | StellarActionId::FieldLockToggle
                | StellarActionId::ToggleConfidencePanel
                | StellarActionId::SubmitInsight
                | StellarActionId::AccessibilityToggle
                | StellarActionId::OpenConflictOverlay
        ),
        StellarPersona::PartnerDeveloper => matches!(
            action,
            StellarActionId::NavigateNextPane
                | StellarActionId::NavigatePrevPane
                | StellarActionId::OpenCommandPalette
                | StellarActionId::ToggleCanvas
                | StellarActionId::ToggleTelemetryOverlay
                | StellarActionId::InputUndo
                | StellarActionId::InputRedo
                | StellarActionId::FieldLockToggle
                | StellarActionId::ToggleConfidencePanel
                | StellarActionId::SubmitInsight
                | StellarActionId::AccessibilityToggle
                | StellarActionId::OpenConflictOverlay
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operator_cannot_resolve_conflicts() {
        assert!(!is_action_allowed(
            StellarPersona::Operator,
            StellarActionId::ResolveConflict
        ));
    }

    #[test]
    fn sre_can_resolve_conflicts() {
        assert!(is_action_allowed(
            StellarPersona::Sre,
            StellarActionId::ResolveConflict
        ));
    }

    #[test]
    fn partner_blocked_from_runbook() {
        assert!(!is_action_allowed(
            StellarPersona::PartnerDeveloper,
            StellarActionId::RunbookInvoke
        ));
    }
}
