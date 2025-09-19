use codex_core::orchestrator::FeedbackInput;
use codex_core::orchestrator::IncidentSeverity;
use codex_core::orchestrator::InvestigationInput;
use codex_core::orchestrator::QuickstartInput;
use codex_core::orchestrator::build_feedback;
use codex_core::orchestrator::build_investigation;
use codex_core::orchestrator::build_quickstart;
use codex_core::stellar::StellarPersona;
use codex_core::telemetry::TelemetrySnapshot;

#[test]
fn overlay_investigate_flow() {
    let playbook = build_investigation(InvestigationInput {
        title: Some("Latency spike".to_string()),
        severity: IncidentSeverity::Severity2,
        persona: StellarPersona::Operator,
        impact: Some("p95 latency > SLO".to_string()),
        hypothesis: Some("Warm cache miss".to_string()),
        requested_at: chrono::Utc::now(),
    });
    assert_eq!(playbook.phases.len(), 4);
    assert!(
        playbook
            .phases
            .iter()
            .any(|phase| phase.name.contains("Diagnose"))
    );

    let guide = build_quickstart(QuickstartInput {
        persona: StellarPersona::Operator,
    });
    assert!(
        guide
            .recommended_commands
            .iter()
            .any(|cmd| cmd.contains("codex orchestrator investigate"))
    );
    assert!(
        guide
            .recommended_commands
            .iter()
            .any(|cmd| cmd.contains("codex pipeline"))
    );

    let feedback = build_feedback(FeedbackInput {
        persona: StellarPersona::Operator,
        telemetry_override: Some(TelemetrySnapshot {
            latency_p95_ms: 180.0,
            audit_fallback_count: 0,
            cache_hit_ratio: 0.92,
            apdex: 0.96,
        }),
        review_effort_baseline_hours: 6.5,
    });
    assert_eq!(
        feedback.status.overall,
        codex_core::orchestrator::HealthState::Green
    );
}
