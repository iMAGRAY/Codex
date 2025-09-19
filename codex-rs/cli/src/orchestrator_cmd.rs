use anyhow::Result;
use chrono::Utc;
use clap::Parser;
use clap::Subcommand;
use clap::ValueEnum;
use codex_core::orchestrator::FeedbackInput;
use codex_core::orchestrator::FeedbackReport;
use codex_core::orchestrator::IncidentSeverity;
use codex_core::orchestrator::InvestigationInput;
use codex_core::orchestrator::InvestigationPlaybook;
use codex_core::orchestrator::QuickstartGuide;
use codex_core::orchestrator::QuickstartInput;
use codex_core::orchestrator::TriageInput;
use codex_core::orchestrator::TriageReport;
use codex_core::orchestrator::build_feedback;
use codex_core::orchestrator::build_investigation;
use codex_core::orchestrator::build_quickstart;
use codex_core::orchestrator::build_triage;
use codex_core::stellar::StellarPersona;
use serde_json::json;

#[derive(Debug, Parser)]
pub struct OrchestratorCli {
    #[command(subcommand)]
    command: OrchestratorCommand,
}

#[derive(Debug, Subcommand)]
enum OrchestratorCommand {
    /// Generate an investigation playbook (checklist, dry-run, audit guidance).
    Investigate(InvestigateArgs),
    /// Render the embedded quickstart guide (overlay/help surface).
    Quickstart(QuickstartArgs),
    /// Collect feedback metrics (latency p95, audit fallbacks, review effort).
    Feedback(FeedbackArgs),
    /// Produce weekly triage summary (APDEX, latency, audit fallbacks, review effort).
    Triage(TriageArgs),
}

#[derive(Debug, Parser)]
pub struct InvestigateArgs {
    /// Incident title for context (defaults to "Unlabelled Incident").
    #[arg(long = "title", value_name = "TEXT")]
    title: Option<String>,
    /// Incident severity (SEV0..SEV3).
    #[arg(long = "severity", value_enum, default_value_t = SeverityArg::Sev2)]
    severity: SeverityArg,
    /// Persona coordinating the investigation.
    #[arg(long = "persona", value_enum, default_value_t = PersonaArg::Operator)]
    persona: PersonaArg,
    /// Optional business/customer impact summary.
    #[arg(long = "impact", value_name = "TEXT")]
    impact: Option<String>,
    /// Optional working hypothesis.
    #[arg(long = "hypothesis", value_name = "TEXT")]
    hypothesis: Option<String>,
    /// Output format.
    #[arg(long = "format", value_enum, default_value_t = OutputFormat::Text)]
    format: OutputFormat,
}

#[derive(Debug, Parser)]
pub struct QuickstartArgs {
    /// Persona requesting quickstart guidance.
    #[arg(long = "persona", value_enum, default_value_t = PersonaArg::Operator)]
    persona: PersonaArg,
    /// Output format.
    #[arg(long = "format", value_enum, default_value_t = OutputFormat::Text)]
    format: OutputFormat,
}

#[derive(Debug, Parser)]
pub struct FeedbackArgs {
    /// Persona submitting feedback.
    #[arg(long = "persona", value_enum, default_value_t = PersonaArg::Operator)]
    persona: PersonaArg,
    /// Review effort baseline (hours) to include in the report.
    #[arg(long = "review-effort-hours", default_value_t = 6.5)]
    review_effort_hours: f32,
    /// Output format.
    #[arg(long = "format", value_enum, default_value_t = OutputFormat::Text)]
    format: OutputFormat,
}

#[derive(Debug, Parser)]
pub struct TriageArgs {
    /// Persona driving the triage review.
    #[arg(long = "persona", value_enum, default_value_t = PersonaArg::Operator)]
    persona: PersonaArg,
    /// Target APDEX threshold (green zone).
    #[arg(long = "apdex-target", default_value_t = 0.85)]
    apdex_target: f64,
    /// Target latency p95 in milliseconds.
    #[arg(long = "latency-target-ms", default_value_t = 200.0)]
    latency_target_ms: f64,
    /// Allowed audit fallback count before yellow/red.
    #[arg(long = "audit-target", default_value_t = 0)]
    audit_target: u64,
    /// Review effort target (hours).
    #[arg(long = "review-target-hours", default_value_t = 4.5)]
    review_target_hours: f32,
    /// Actual review effort captured for the week (hours).
    #[arg(long = "review-hours")]
    review_hours: Option<f32>,
    /// Output format.
    #[arg(long = "format", value_enum, default_value_t = OutputFormat::Text)]
    format: OutputFormat,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum OutputFormat {
    Text,
    Json,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum SeverityArg {
    Sev0,
    Sev1,
    Sev2,
    Sev3,
}

impl From<SeverityArg> for IncidentSeverity {
    fn from(value: SeverityArg) -> Self {
        match value {
            SeverityArg::Sev0 => IncidentSeverity::Severity0,
            SeverityArg::Sev1 => IncidentSeverity::Severity1,
            SeverityArg::Sev2 => IncidentSeverity::Severity2,
            SeverityArg::Sev3 => IncidentSeverity::Severity3,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum PersonaArg {
    Operator,
    Sre,
    Secops,
    Platform,
    Partner,
    Assistive,
}

impl From<PersonaArg> for StellarPersona {
    fn from(value: PersonaArg) -> Self {
        match value {
            PersonaArg::Operator => StellarPersona::Operator,
            PersonaArg::Sre => StellarPersona::Sre,
            PersonaArg::Secops => StellarPersona::SecOps,
            PersonaArg::Platform => StellarPersona::PlatformEngineer,
            PersonaArg::Partner => StellarPersona::PartnerDeveloper,
            PersonaArg::Assistive => StellarPersona::AssistiveBridge,
        }
    }
}

pub fn run(cli: OrchestratorCli) -> Result<()> {
    match cli.command {
        OrchestratorCommand::Investigate(args) => run_investigate(args),
        OrchestratorCommand::Quickstart(args) => run_quickstart(args),
        OrchestratorCommand::Feedback(args) => run_feedback(args),
        OrchestratorCommand::Triage(args) => run_triage(args),
    }
}

fn run_investigate(args: InvestigateArgs) -> Result<()> {
    let input = InvestigationInput {
        title: args.title,
        severity: args.severity.into(),
        persona: args.persona.into(),
        impact: args.impact,
        hypothesis: args.hypothesis,
        requested_at: Utc::now(),
    };
    let plan = build_investigation(input);
    match args.format {
        OutputFormat::Text => print_investigation_text(&plan),
        OutputFormat::Json => print_json(&json!(plan)),
    }
}

fn run_quickstart(args: QuickstartArgs) -> Result<()> {
    let guide = build_quickstart(QuickstartInput {
        persona: args.persona.into(),
    });
    match args.format {
        OutputFormat::Text => print_quickstart_text(&guide),
        OutputFormat::Json => print_json(&json!(guide)),
    }
}

fn run_feedback(args: FeedbackArgs) -> Result<()> {
    let report = build_feedback(FeedbackInput {
        persona: args.persona.into(),
        telemetry_override: None,
        review_effort_baseline_hours: args.review_effort_hours,
    });
    match args.format {
        OutputFormat::Text => print_feedback_text(&report),
        OutputFormat::Json => print_json(&json!(report)),
    }
}

fn run_triage(args: TriageArgs) -> Result<()> {
    let report = build_triage(TriageInput {
        persona: args.persona.into(),
        apdex_target: args.apdex_target,
        latency_target_ms: args.latency_target_ms,
        audit_fallback_target: args.audit_target,
        review_effort_target_hours: args.review_target_hours,
        review_effort_hours: args.review_hours,
    });
    match args.format {
        OutputFormat::Text => print_triage_text(&report),
        OutputFormat::Json => print_json(&json!(report)),
    }
}

fn print_investigation_text(plan: &InvestigationPlaybook) -> Result<()> {
    println!("Investigation: {}", plan.metadata.title);
    println!(
        "Severity: {} | Persona: {}",
        plan.metadata.severity, plan.metadata.persona
    );
    if let Some(impact) = &plan.metadata.impact {
        println!("Impact: {impact}");
    }
    if let Some(hypothesis) = &plan.metadata.hypothesis {
        println!("Hypothesis: {hypothesis}");
    }
    println!(
        "Telemetry snapshot → latency p95: {:.1} ms | audit fallbacks: {} | cache hit: {:.0}%",
        plan.metadata.telemetry.latency_p95_ms,
        plan.metadata.telemetry.audit_fallback_count,
        plan.metadata.telemetry.cache_hit_ratio * 100.0
    );

    for (idx, phase) in plan.phases.iter().enumerate() {
        println!();
        println!("Phase {} — {}", idx + 1, phase.name);
        for item in &phase.checklist {
            println!(
                "  [{}] {} ({})",
                if item.required { '!' } else { ' ' },
                item.description,
                item.owner
            );
        }
    }

    println!();
    println!("Dry-run objective: {}", plan.dry_run.objective);
    for step in &plan.dry_run.steps {
        println!("  - {step}");
    }
    if !plan.dry_run.rollback_conditions.is_empty() {
        println!("  Rollback when:");
        for cond in &plan.dry_run.rollback_conditions {
            println!("    • {cond}");
        }
    }

    println!();
    println!("Audit trail: {}", plan.audit.ledger_resource);
    if !plan.audit.tags.is_empty() {
        println!("  Tags: {}", plan.audit.tags.join(", "));
    }
    if !plan.audit.evidence_requirements.is_empty() {
        println!("  Evidence:");
        for item in &plan.audit.evidence_requirements {
            println!("    • {item}");
        }
    }
    Ok(())
}

fn print_quickstart_text(guide: &QuickstartGuide) -> Result<()> {
    println!("{}", guide.headline);
    for section in &guide.sections {
        println!();
        println!("{}:", section.title);
        for bullet in &section.bullets {
            println!("  • {bullet}");
        }
    }
    if !guide.recommended_commands.is_empty() {
        println!();
        println!("Recommended commands:");
        for cmd in &guide.recommended_commands {
            println!("  $ {cmd}");
        }
    }
    Ok(())
}

fn print_feedback_text(report: &FeedbackReport) -> Result<()> {
    println!("Feedback snapshot @ {}", report.captured_at.to_rfc3339());
    println!("Persona: {}", report.persona);
    println!(
        "Status: {:?} — {}",
        report.status.overall, report.status.summary
    );
    println!(
        "Telemetry → latency p95: {:.1} ms | audit fallbacks: {} | cache hit: {:.0}%",
        report.metrics.latency_p95_ms,
        report.metrics.audit_fallback_count,
        report.metrics.cache_hit_ratio * 100.0
    );
    println!(
        "Review effort baseline: {:.1} h",
        report.metrics.review_effort_hours
    );
    println!("Recommendations:");
    for item in &report.recommendations {
        println!("  • {item}");
    }
    Ok(())
}

fn print_triage_text(report: &TriageReport) -> Result<()> {
    println!("Triage snapshot @ {}", report.captured_at.to_rfc3339());
    println!("Persona: {}", report.persona);
    println!(
        "APDEX: {} — {:?}",
        report.metrics.apdex.summary, report.metrics.apdex.status
    );
    println!(
        "Latency: {} — {:?}",
        report.metrics.latency_p95_ms.summary, report.metrics.latency_p95_ms.status
    );
    println!(
        "Audit: {} — {:?}",
        report.metrics.audit_fallback_count.summary, report.metrics.audit_fallback_count.status
    );
    println!(
        "Review effort: {} — {:?}",
        report.metrics.review_effort_hours.summary, report.metrics.review_effort_hours.status
    );
    if !report.checklist_updates.is_empty() {
        println!("Checklist updates:");
        for item in &report.checklist_updates {
            println!("  • {item}");
        }
    }
    if !report.notes.is_empty() {
        println!("Notes:");
        for note in &report.notes {
            println!("  • {note}");
        }
    }
    Ok(())
}

fn print_json(value: &serde_json::Value) -> Result<()> {
    let rendered = serde_json::to_string_pretty(value)?;
    println!("{rendered}");
    Ok(())
}

// Provide `Display` implementations for Clap value enums using existing conversions.
impl std::fmt::Display for PersonaArg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let persona: StellarPersona = (*self).into();
        write!(f, "{}", persona)
    }
}

impl std::fmt::Display for SeverityArg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let severity: IncidentSeverity = (*self).into();
        write!(f, "{}", severity.label())
    }
}

// Expose `Display` for `StellarPersona` (provided via `Display` impl in core).

// Unit tests ensure text formatters stay stable for documentation snippets.
#[cfg(test)]
mod tests {
    use super::*;
    use codex_core::orchestrator::HealthState;
    use codex_core::telemetry::TelemetryHub;
    use codex_core::telemetry::TelemetrySnapshot;
    use std::time::Duration;

    #[test]
    fn investigation_text_renderer_succeeds() {
        let plan = build_investigation(InvestigationInput {
            title: Some("Cache incident".to_string()),
            severity: IncidentSeverity::Severity1,
            persona: StellarPersona::Operator,
            impact: Some("Customer requests timing out".to_string()),
            hypothesis: None,
            requested_at: Utc::now(),
        });
        print_investigation_text(&plan).expect("render");
    }

    #[test]
    fn feedback_report_classifies_health() {
        let report = build_feedback(FeedbackInput {
            persona: StellarPersona::SecOps,
            telemetry_override: Some(TelemetrySnapshot {
                latency_p95_ms: 150.0,
                audit_fallback_count: 0,
                cache_hit_ratio: 0.95,
                apdex: 0.98,
            }),
            review_effort_baseline_hours: 4.0,
        });
        assert_eq!(report.status.overall, HealthState::Green);
    }

    #[test]
    fn triage_report_uses_live_snapshot() {
        let hub = TelemetryHub::global();
        hub.record_exec_latency(Duration::from_millis(90));
        hub.record_exec_latency(Duration::from_millis(350));
        hub.record_cache_hit();
        hub.record_cache_miss();
        let report = build_triage(TriageInput {
            review_effort_hours: Some(5.0),
            ..TriageInput::default()
        });
        assert!(!report.metrics.apdex.summary.is_empty());
    }
}
