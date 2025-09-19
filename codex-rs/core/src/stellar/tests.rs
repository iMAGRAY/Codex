use super::*;
use crate::resilience::ConflictDecision;

#[test]
fn empty_submission_blocked() {
    let mut kernel = StellarKernel::new(StellarPersona::Operator);
    kernel.set_field_text("");
    let err = kernel
        .handle_action(StellarAction::SubmitInsight { text: None })
        .unwrap_err();
    assert!(matches!(err, GuardError::EmptySubmission));
}

#[test]
fn submit_records_command_log_and_event() {
    let mut kernel = StellarKernel::new(StellarPersona::Operator);
    kernel.set_field_text("Investigate cache latency");
    let applied = kernel
        .handle_action(StellarAction::SubmitInsight { text: None })
        .expect("guard should allow");
    assert!(matches!(applied, ActionApplied::StateChanged));
    let snapshot = kernel.snapshot();
    assert_eq!(snapshot.command_log.len(), 1);
    assert!(snapshot.command_log[0].contains("Investigate cache latency"));
    let events = kernel.take_events();
    assert_eq!(events.len(), 2);
    match &events[0] {
        KernelEvent::Submission { text } => assert_eq!(text, "Investigate cache latency"),
        other => panic!("unexpected event: {other:?}"),
    }
    assert!(matches!(events[1], KernelEvent::CacheStored { .. }));
}

#[test]
fn undo_and_redo_cycle_submission() {
    let mut kernel = StellarKernel::new(StellarPersona::Operator);
    kernel.set_field_text("Check APDEX drop");
    kernel
        .handle_action(StellarAction::SubmitInsight { text: None })
        .unwrap();
    kernel.clear_status_messages();

    let undo = kernel.handle_action(StellarAction::InputUndo).unwrap();
    assert!(matches!(undo, ActionApplied::StateChanged));
    assert_eq!(kernel.snapshot().command_log.len(), 0);
    assert_eq!(kernel.field_text(), "Check APDEX drop");

    kernel.clear_status_messages();
    let redo = kernel.handle_action(StellarAction::InputRedo).unwrap();
    assert!(matches!(redo, ActionApplied::StateChanged));
    assert_eq!(kernel.snapshot().command_log.len(), 1);
}

#[test]
fn toggle_confidence_updates_visibility() {
    let mut kernel = StellarKernel::new(StellarPersona::Operator);
    assert!(!kernel.snapshot().confidence.unwrap().visible);
    kernel
        .handle_action(StellarAction::ToggleConfidencePanel)
        .unwrap();
    assert!(kernel.snapshot().confidence.unwrap().visible);
}

#[test]
fn operator_resolve_conflict_is_denied_by_rbac() {
    let mut kernel = StellarKernel::new(StellarPersona::Operator);
    let err = kernel
        .handle_action(StellarAction::ResolveConflict {
            conflict_id: None,
            decision: ConflictDecision::Accept,
        })
        .unwrap_err();
    assert!(matches!(
        err,
        GuardError::PersonaDenied {
            persona: StellarPersona::Operator,
            action: StellarActionId::ResolveConflict
        }
    ));
}

#[test]
fn sre_resolve_conflict_is_allowed() {
    let mut kernel = StellarKernel::new(StellarPersona::Sre);
    let applied = kernel
        .handle_action(StellarAction::ResolveConflict {
            conflict_id: None,
            decision: ConflictDecision::Accept,
        })
        .expect("SRE persona should bypass RBAC restriction");
    assert!(matches!(
        applied,
        ActionApplied::StateChanged | ActionApplied::NoChange
    ));
}

#[test]
fn partner_cannot_invoke_runbook() {
    let mut kernel = StellarKernel::new(StellarPersona::PartnerDeveloper);
    let err = kernel
        .handle_action(StellarAction::RunbookInvoke { runbook_id: None })
        .unwrap_err();
    assert!(matches!(
        err,
        GuardError::PersonaDenied {
            persona: StellarPersona::PartnerDeveloper,
            action: StellarActionId::RunbookInvoke
        }
    ));
}
