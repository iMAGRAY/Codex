use chrono::DateTime;
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;

use crate::stellar::StellarPersona;
use crate::telemetry::TelemetryHub;
use crate::telemetry::TelemetrySnapshot;

/// Severity scale used for investigation orchestration (REQ-ACC-01).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IncidentSeverity {
    Severity0,
    Severity1,
    Severity2,
    Severity3,
}

impl IncidentSeverity {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            IncidentSeverity::Severity0 => "SEV0",
            IncidentSeverity::Severity1 => "SEV1",
            IncidentSeverity::Severity2 => "SEV2",
            IncidentSeverity::Severity3 => "SEV3",
        }
    }
}

impl Default for IncidentSeverity {
    fn default() -> Self {
        IncidentSeverity::Severity2
    }
}

/// Input for generating an investigation playbook.
#[derive(Debug, Clone)]
pub struct InvestigationInput {
    pub title: Option<String>,
    pub severity: IncidentSeverity,
    pub persona: StellarPersona,
    pub impact: Option<String>,
    pub hypothesis: Option<String>,
    pub requested_at: DateTime<Utc>,
}

impl Default for InvestigationInput {
    fn default() -> Self {
        Self {
            title: None,
            severity: IncidentSeverity::default(),
            persona: StellarPersona::Operator,
            impact: None,
            hypothesis: None,
            requested_at: Utc::now(),
        }
    }
}

/// Structured investigation plan (REQ-ACC-01).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InvestigationPlaybook {
    pub metadata: InvestigationMetadata,
    pub phases: Vec<InvestigationPhase>,
    pub dry_run: DryRunPlan,
    pub audit: AuditPlan,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InvestigationMetadata {
    pub title: String,
    pub severity: String,
    pub persona: StellarPersona,
    pub impact: Option<String>,
    pub hypothesis: Option<String>,
    pub requested_at: DateTime<Utc>,
    pub telemetry: TelemetrySnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InvestigationPhase {
    pub name: String,
    pub checklist: Vec<ChecklistItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChecklistItem {
    pub id: String,
    pub description: String,
    pub owner: String,
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DryRunPlan {
    pub objective: String,
    pub steps: Vec<String>,
    pub rollback_conditions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AuditPlan {
    pub ledger_resource: String,
    pub tags: Vec<String>,
    pub evidence_requirements: Vec<String>,
}

/// Input for the quickstart helper.
#[derive(Debug, Clone)]
pub struct QuickstartInput {
    pub persona: StellarPersona,
}

impl Default for QuickstartInput {
    fn default() -> Self {
        Self {
            persona: StellarPersona::Operator,
        }
    }
}

/// Quickstart helper for onboarding (REQ-ACC-01).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QuickstartGuide {
    pub headline: String,
    pub sections: Vec<QuickstartSection>,
    pub recommended_commands: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QuickstartSection {
    pub title: String,
    pub bullets: Vec<String>,
}

/// Input for generating an automated feedback packet.
#[derive(Debug, Clone)]
pub struct FeedbackInput {
    pub persona: StellarPersona,
    pub telemetry_override: Option<TelemetrySnapshot>,
    pub review_effort_baseline_hours: f32,
}

impl Default for FeedbackInput {
    fn default() -> Self {
        Self {
            persona: StellarPersona::Operator,
            telemetry_override: None,
            review_effort_baseline_hours: 6.5,
        }
    }
}

/// Structured feedback packet summarising current SLO health (REQ-ACC-01).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FeedbackReport {
    pub captured_at: DateTime<Utc>,
    pub persona: StellarPersona,
    pub telemetry: TelemetrySnapshot,
    pub status: FeedbackStatus,
    pub metrics: FeedbackMetrics,
    pub recommendations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FeedbackStatus {
    pub overall: HealthState,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FeedbackMetrics {
    pub latency_p95_ms: f64,
    pub audit_fallback_count: u64,
    pub cache_hit_ratio: f64,
    pub review_effort_hours: f32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HealthState {
    Green,
    Yellow,
    Red,
}

/// Configuration for the weekly triage summary.
#[derive(Debug, Clone)]
pub struct TriageInput {
    pub persona: StellarPersona,
    pub apdex_target: f64,
    pub latency_target_ms: f64,
    pub audit_fallback_target: u64,
    pub review_effort_target_hours: f32,
    pub review_effort_hours: Option<f32>,
}

impl Default for TriageInput {
    fn default() -> Self {
        Self {
            persona: StellarPersona::Operator,
            apdex_target: 0.85,
            latency_target_ms: 200.0,
            audit_fallback_target: 0,
            review_effort_target_hours: 4.5,
            review_effort_hours: None,
        }
    }
}

/// Output of the triage generator (REQ-ACC-01 weekly safeguards).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TriageReport {
    pub captured_at: DateTime<Utc>,
    pub persona: StellarPersona,
    pub metrics: TriageMetrics,
    pub checklist_updates: Vec<String>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TriageMetrics {
    pub apdex: MetricDetail,
    pub latency_p95_ms: MetricDetail,
    pub audit_fallback_count: MetricDetail,
    pub review_effort_hours: MetricDetail,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MetricDetail {
    pub value: f64,
    pub target: f64,
    pub status: HealthState,
    pub summary: String,
}

/// Build an investigation plan tailored to persona & severity.
#[must_use]
pub fn build_investigation(input: InvestigationInput) -> InvestigationPlaybook {
    let telemetry = TelemetryHub::global().snapshot();
    let title = input
        .title
        .unwrap_or_else(|| "Unlabelled Incident".to_string());
    let impact = input.impact.clone();
    let hypothesis = input.hypothesis.clone();
    let severity_label = input.severity.label().to_string();

    let metadata = InvestigationMetadata {
        title: title.clone(),
        severity: severity_label,
        persona: input.persona,
        impact,
        hypothesis,
        requested_at: input.requested_at,
        telemetry,
    };

    let phases = investigation_phases(&metadata);
    let dry_run = DryRunPlan {
        objective: "Validate remediation steps without impacting live traffic".to_string(),
        steps: vec![
            "Replay failing request against staging with feature flags mirrored".to_string(),
            "Verify telemetry deltas (latency, error budget) stay within guardrails".to_string(),
            "Simulate rollback via `codex pipeline rollback <pack> <prev-version>`".to_string(),
        ],
        rollback_conditions: vec![
            "Latency p95 exceeds 200 мс for more than 3 consecutive minutes".to_string(),
            "Audit fallback counter increments during dry-run".to_string(),
            "Confidence score drops below 70%".to_string(),
        ],
    };

    let audit = AuditPlan {
        ledger_resource: format!("investigation:{}", title.to_lowercase().replace(' ', "-")),
        tags: vec![
            "orchestrator".to_string(),
            "investigation".to_string(),
            input.persona.to_string(),
        ],
        evidence_requirements: vec![
            "Attach dry-run output and telemetry deltas".to_string(),
            "Link signed pipeline bundle IDs impacting incident".to_string(),
            "Summarise customer impact + recovery timeline".to_string(),
        ],
    };

    InvestigationPlaybook {
        metadata,
        phases,
        dry_run,
        audit,
    }
}

/// Build the quickstart helper for the `/quickstart` surface.
#[must_use]
pub fn build_quickstart(input: QuickstartInput) -> QuickstartGuide {
    let persona = input.persona.to_string();
    QuickstartGuide {
        headline: format!("Welcome, {persona}!"),
        sections: vec![
            QuickstartSection {
                title: "Launch".to_string(),
                bullets: vec![
                    "Press Ctrl+O to open the Observability Overlay (latency p95 / cache hit).".to_string(),
                    "Use Ctrl+R to jump into persona-aligned runbooks via Investigate.".to_string(),
                ],
            },
            QuickstartSection {
                title: "Signed delivery".to_string(),
                bullets: vec![
                    "`codex pipeline sign --name insight --version <v>` to produce a signed bundle.".to_string(),
                    "`codex pipeline verify <bundle> --install` to promote into the workspace.".to_string(),
                    "`codex pipeline rollback <name> <version>` to revert when checks fail.".to_string(),
                ],
            },
            QuickstartSection {
                title: "Feedback".to_string(),
                bullets: vec![
                    "`codex orchestrator feedback` captures latency p95, audit fallback count, review effort.".to_string(),
                    "Attach the generated JSON to `/feedback` so the Governance Portal can track trends.".to_string(),
                ],
            },
        ],
        recommended_commands: vec![
            "codex pipeline sign --name insight --version 1.4.0 --source packs/insight --signer vault:pipeline/insight".to_string(),
            "codex orchestrator investigate --title \"Latency spike\" --severity sev2".to_string(),
            "codex orchestrator quickstart".to_string(),
            "codex orchestrator triage".to_string(),
            "codex orchestrator feedback".to_string(),
        ],
    }
}

/// Build an automated feedback packet summarising current health.
#[must_use]
pub fn build_feedback(input: FeedbackInput) -> FeedbackReport {
    let telemetry = input
        .telemetry_override
        .unwrap_or_else(|| TelemetryHub::global().snapshot());
    let (overall, summary) = classify_health(&telemetry);
    let feedback_metrics = FeedbackMetrics {
        latency_p95_ms: telemetry.latency_p95_ms,
        audit_fallback_count: telemetry.audit_fallback_count,
        cache_hit_ratio: telemetry.cache_hit_ratio,
        review_effort_hours: input.review_effort_baseline_hours,
    };
    let recommendations = build_recommendations(&feedback_metrics);

    FeedbackReport {
        captured_at: Utc::now(),
        persona: input.persona,
        telemetry,
        status: FeedbackStatus { overall, summary },
        metrics: feedback_metrics,
        recommendations,
    }
}

/// Build a weekly triage summary, comparing telemetry against targets.
#[must_use]
pub fn build_triage(input: TriageInput) -> TriageReport {
    let telemetry = TelemetryHub::global().snapshot();

    let apdex = metric_higher_is_better(telemetry.apdex, input.apdex_target, "APDEX", 0.05);
    let latency = metric_lower_is_better(
        telemetry.latency_p95_ms,
        input.latency_target_ms,
        "Latency p95 (ms)",
        0.2,
    );
    let audit = metric_lower_is_better(
        telemetry.audit_fallback_count as f64,
        input.audit_fallback_target as f64,
        "Audit fallbacks",
        1.0,
    );
    let review_value = input
        .review_effort_hours
        .unwrap_or(input.review_effort_target_hours) as f64;
    let review = metric_lower_is_better(
        review_value,
        input.review_effort_target_hours as f64,
        "Review effort (hours)",
        0.15,
    );

    let metrics = TriageMetrics {
        apdex,
        latency_p95_ms: latency,
        audit_fallback_count: audit,
        review_effort_hours: review,
    };

    let mut checklist_updates = Vec::new();
    if !matches!(metrics.apdex.status, HealthState::Green) {
        checklist_updates.push("Add customer journey replay to triage checklist".to_string());
    }
    if !matches!(metrics.latency_p95_ms.status, HealthState::Green) {
        checklist_updates.push("Schedule SRE latency drill for affected services".to_string());
    }
    if !matches!(metrics.audit_fallback_count.status, HealthState::Green) {
        checklist_updates.push("Review signed pipeline bundles for fallback events".to_string());
    }
    if !matches!(metrics.review_effort_hours.status, HealthState::Green) {
        checklist_updates.push("Rebalance reviewer rotations to hit effort target".to_string());
    }

    let mut notes = Vec::new();
    if matches!(metrics.apdex.status, HealthState::Green)
        && matches!(metrics.latency_p95_ms.status, HealthState::Green)
        && matches!(metrics.audit_fallback_count.status, HealthState::Green)
    {
        notes.push(
            "Metrics within SLO — document steady-state and close triage without escalation."
                .to_string(),
        );
    }

    TriageReport {
        captured_at: Utc::now(),
        persona: input.persona,
        metrics,
        checklist_updates,
        notes,
    }
}

fn metric_higher_is_better(
    value: f64,
    target: f64,
    label: &str,
    yellow_margin: f64,
) -> MetricDetail {
    let status = if value >= target {
        HealthState::Green
    } else if value + yellow_margin >= target {
        HealthState::Yellow
    } else {
        HealthState::Red
    };
    let summary = match status {
        HealthState::Green => format!("{label}: {value:.3} (meets target ≥{target:.3})"),
        HealthState::Yellow => format!(
            "{label}: {value:.3} (within {:.3} of target {target:.3})",
            (target - value).max(0.0)
        ),
        HealthState::Red => format!("{label}: {value:.3} (below target {target:.3})"),
    };
    MetricDetail {
        value,
        target,
        status,
        summary,
    }
}

fn metric_lower_is_better(value: f64, target: f64, label: &str, yellow_ratio: f64) -> MetricDetail {
    let status = if target <= f64::EPSILON {
        if value <= 0.0 {
            HealthState::Green
        } else if value <= 1.0 {
            HealthState::Yellow
        } else {
            HealthState::Red
        }
    } else if value <= target {
        HealthState::Green
    } else if value <= target * (1.0 + yellow_ratio) {
        HealthState::Yellow
    } else {
        HealthState::Red
    };

    let summary = match status {
        HealthState::Green => format!("{label}: {value:.3} (≤ target {target:.3})"),
        HealthState::Yellow => {
            if target <= f64::EPSILON {
                format!("{label}: {value:.3} (slightly above zero target)")
            } else {
                let percent = ((value / target) - 1.0) * 100.0;
                format!(
                    "{label}: {value:.3} ({:.0}% above target {target:.3})",
                    percent.abs().round()
                )
            }
        }
        HealthState::Red => {
            if target <= f64::EPSILON {
                format!("{label}: {value:.3} (requires immediate action)")
            } else {
                format!("{label}: {value:.3} (exceeds target {target:.3})")
            }
        }
    };

    MetricDetail {
        value,
        target,
        status,
        summary,
    }
}

fn investigation_phases(metadata: &InvestigationMetadata) -> Vec<InvestigationPhase> {
    vec![
        InvestigationPhase {
            name: "Stabilise".to_string(),
            checklist: vec![
                ChecklistItem {
                    id: "STB-1".to_string(),
                    description: format!(
                        "Acknowledge {} and announce expected recovery window",
                        metadata.severity
                    ),
                    owner: metadata.persona.to_string(),
                    required: true,
                },
                ChecklistItem {
                    id: "STB-2".to_string(),
                    description:
                        "Freeze risky deploys, pin affected services, enable adaptive throttling"
                            .to_string(),
                    owner: "Platform".to_string(),
                    required: true,
                },
                ChecklistItem {
                    id: "STB-3".to_string(),
                    description:
                        "Broadcast status to stakeholders (Slack #stellar-incident, StatusPage)"
                            .to_string(),
                    owner: "Comms".to_string(),
                    required: false,
                },
            ],
        },
        InvestigationPhase {
            name: "Diagnose".to_string(),
            checklist: vec![
                ChecklistItem {
                    id: "DIA-1".to_string(),
                    description:
                        "Pull telemetry overlay snapshot (latency p95, audit fallback count)"
                            .to_string(),
                    owner: metadata.persona.to_string(),
                    required: true,
                },
                ChecklistItem {
                    id: "DIA-2".to_string(),
                    description: "Correlate recent pipeline installs & feature flags".to_string(),
                    owner: "Delivery".to_string(),
                    required: true,
                },
                ChecklistItem {
                    id: "DIA-3".to_string(),
                    description: "Audit resilience cache hit/miss ratio to confirm data integrity"
                        .to_string(),
                    owner: "Reliability".to_string(),
                    required: false,
                },
            ],
        },
        InvestigationPhase {
            name: "Remediate".to_string(),
            checklist: vec![
                ChecklistItem {
                    id: "REM-1".to_string(),
                    description:
                        "Prepare mitigation runbook (rollback, traffic shift, guardrail tweak)"
                            .to_string(),
                    owner: "Platform".to_string(),
                    required: true,
                },
                ChecklistItem {
                    id: "REM-2".to_string(),
                    description:
                        "Run dry-run plan and capture evidence (logs, metrics, bundle fingerprints)"
                            .to_string(),
                    owner: metadata.persona.to_string(),
                    required: true,
                },
                ChecklistItem {
                    id: "REM-3".to_string(),
                    description:
                        "Trigger signed rollback via `codex pipeline rollback` if safeguards trip"
                            .to_string(),
                    owner: "Delivery".to_string(),
                    required: false,
                },
            ],
        },
        InvestigationPhase {
            name: "Close".to_string(),
            checklist: vec![
                ChecklistItem {
                    id: "CLS-1".to_string(),
                    description:
                        "Summarise incident in `/feedback` (latency, audit fallback, review effort)"
                            .to_string(),
                    owner: metadata.persona.to_string(),
                    required: true,
                },
                ChecklistItem {
                    id: "CLS-2".to_string(),
                    description: "Attach signed bundles & diff reports to audit trail".to_string(),
                    owner: "Governance".to_string(),
                    required: true,
                },
                ChecklistItem {
                    id: "CLS-3".to_string(),
                    description: "Schedule post-incident review & update runbooks".to_string(),
                    owner: "SRE".to_string(),
                    required: false,
                },
            ],
        },
    ]
}

fn classify_health(snapshot: &TelemetrySnapshot) -> (HealthState, String) {
    let mut deficits: Vec<&str> = Vec::new();
    if snapshot.latency_p95_ms > 220.0 {
        deficits.push("latency p95 > 220 мс");
    }
    if snapshot.audit_fallback_count > 0 {
        deficits.push("audit fallbacks detected");
    }
    if snapshot.cache_hit_ratio < 0.6 {
        deficits.push("cache hit ratio < 60%");
    }

    if deficits.is_empty() {
        (
            HealthState::Green,
            "Telemetry within targets; proceed with standard QA cadence.".to_string(),
        )
    } else if deficits.len() == 1 {
        (
            HealthState::Yellow,
            format!("Watchlist: {}", deficits.join(", ")),
        )
    } else {
        (
            HealthState::Red,
            format!("Escalate immediately: {}", deficits.join(", ")),
        )
    }
}

fn build_recommendations(metrics: &FeedbackMetrics) -> Vec<String> {
    let mut items = Vec::new();
    if metrics.latency_p95_ms > 220.0 {
        items.push(
            "Trigger `/investigate` to refresh mitigation checklist before next deploy."
                .to_string(),
        );
    }
    if metrics.audit_fallback_count > 0 {
        items.push(
            "Review audit ledger for fallback entries and reconcile signed bundle fingerprints."
                .to_string(),
        );
    }
    if metrics.cache_hit_ratio < 0.6 {
        items.push(
            "Warm resilience cache via `codex orchestrator quickstart` pre-flight routine."
                .to_string(),
        );
    }
    if items.is_empty() {
        items.push("All signals green — document the steady-state in `/feedback`.".to_string());
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telemetry::TelemetryHub;
    use crate::telemetry::TelemetrySnapshot;
    use pretty_assertions::assert_eq;
    use std::time::Duration;

    #[test]
    fn investigation_includes_expected_phases() {
        let input = InvestigationInput {
            title: Some("Search latency".to_string()),
            severity: IncidentSeverity::Severity1,
            persona: StellarPersona::Sre,
            impact: Some("95p latency above SLO".to_string()),
            hypothesis: Some("Cache eviction spike".to_string()),
            requested_at: Utc::now(),
        };
        let playbook = build_investigation(input);
        assert_eq!(playbook.phases.len(), 4);
        assert_eq!(playbook.phases[0].name, "Stabilise");
        assert!(
            playbook
                .phases
                .iter()
                .all(|phase| !phase.checklist.is_empty())
        );
        assert_eq!(playbook.metadata.persona, StellarPersona::Sre);
    }

    #[test]
    fn quickstart_contains_recommended_commands() {
        let guide = build_quickstart(QuickstartInput {
            persona: StellarPersona::PlatformEngineer,
        });
        assert!(!guide.sections.is_empty());
        assert!(
            guide
                .recommended_commands
                .iter()
                .any(|cmd| cmd.contains("orchestrator"))
        );
    }

    #[test]
    fn feedback_respects_override_snapshot() {
        let snapshot = TelemetrySnapshot {
            latency_p95_ms: 250.0,
            audit_fallback_count: 2,
            cache_hit_ratio: 0.4,
            apdex: 0.62,
        };
        let report = build_feedback(FeedbackInput {
            persona: StellarPersona::Operator,
            telemetry_override: Some(snapshot),
            review_effort_baseline_hours: 6.5,
        });
        assert_eq!(report.status.overall, HealthState::Red);
        assert_eq!(report.metrics.latency_p95_ms, 250.0);
        assert!(!report.recommendations.is_empty());
    }

    #[test]
    fn triage_flags_high_latency() {
        let hub = TelemetryHub::global();
        hub.record_exec_latency(Duration::from_millis(500));
        hub.record_exec_latency(Duration::from_millis(520));
        let report = build_triage(TriageInput {
            latency_target_ms: 150.0,
            review_effort_hours: Some(5.5),
            ..TriageInput::default()
        });
        assert!(
            report
                .checklist_updates
                .iter()
                .any(|item| item.contains("latency"))
        );
    }
}
