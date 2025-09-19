use std::fs;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use chrono::DateTime;
use chrono::Utc;
use clap::Parser;
use clap::Subcommand;
use codex_core::security::AuditRecord;
use codex_core::security::PolicyEvidenceRecord;
use codex_core::security::export_audit_records;
use codex_core::security::export_audit_records_since;
use codex_core::security::export_policy_evidence_snapshot;

/// Audit tooling for SecOps to extract immutable ledger snapshots (REQ-SEC-02, #10, #57).
#[derive(Debug, Parser)]
pub struct AuditCli {
    #[command(subcommand)]
    command: AuditCommand,
}

#[derive(Debug, Subcommand)]
enum AuditCommand {
    /// Export immutable audit ledger entries as JSON (supports optional policy evidence snapshot).
    Export {
        /// Optional output file path (defaults to stdout when omitted).
        #[arg(long = "out", value_name = "PATH")]
        out: Option<PathBuf>,
        /// When set, include the 24h policy evidence log snapshot.
        #[arg(long = "policy-evidence", default_value_t = false)]
        policy_evidence: bool,
        /// Filter records newer than the provided RFC 3339 timestamp.
        #[arg(long = "since", value_name = "RFC3339")]
        since: Option<String>,
        /// Render pretty JSON instead of a single line.
        #[arg(long = "pretty", default_value_t = false)]
        pretty: bool,
    },
}

#[derive(Debug, serde::Serialize)]
struct AuditExportPayload {
    records: Vec<AuditRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    policy_evidence: Option<Vec<PolicyEvidenceRecord>>,
}

pub fn run(cli: AuditCli) -> Result<()> {
    match cli.command {
        AuditCommand::Export {
            out,
            policy_evidence,
            since,
            pretty,
        } => export(out, policy_evidence, since, pretty),
    }
}

fn export(
    out: Option<PathBuf>,
    include_policy: bool,
    since: Option<String>,
    pretty: bool,
) -> Result<()> {
    let since = parse_since(since)?;
    let records = match since {
        Some(when) => export_audit_records_since(Some(when))
            .map_err(anyhow::Error::from)
            .context("failed to export audit ledger entries")?,
        None => export_audit_records()
            .map_err(anyhow::Error::from)
            .context("failed to export audit ledger entries")?,
    };

    let policy_evidence = if include_policy {
        Some(
            export_policy_evidence_snapshot()
                .map_err(anyhow::Error::from)
                .context("failed to export policy evidence snapshot")?,
        )
    } else {
        None
    };

    let payload = AuditExportPayload {
        records,
        policy_evidence,
    };

    let serialized = if pretty {
        serde_json::to_string_pretty(&payload)?
    } else {
        serde_json::to_string(&payload)?
    };

    match out {
        Some(path) => {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).with_context(|| {
                    format!(
                        "failed to create audit export directory at {}",
                        parent.display()
                    )
                })?;
            }
            fs::write(&path, format!("{serialized}\n"))?;
            println!(
                "Audit ledger export complete: {} (policy evidence: {})",
                path.display(),
                if payload.policy_evidence.is_some() {
                    "included"
                } else {
                    "omitted"
                }
            );
        }
        None => {
            println!("{serialized}");
        }
    }
    Ok(())
}

fn parse_since(arg: Option<String>) -> Result<Option<DateTime<Utc>>> {
    if let Some(value) = arg {
        let parsed = DateTime::parse_from_rfc3339(&value)
            .with_context(|| format!("failed to parse --since timestamp '{value}'"))?;
        Ok(Some(parsed.with_timezone(&Utc)))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_since_handles_none() {
        assert!(parse_since(None).unwrap().is_none());
    }

    #[test]
    fn parse_since_parses_valid_timestamp() {
        let parsed = parse_since(Some("2025-09-18T12:00:00Z".to_string())).unwrap();
        assert!(parsed.is_some());
    }

    #[test]
    fn parse_since_rejects_invalid_timestamp() {
        assert!(parse_since(Some("not-a-timestamp".to_string())).is_err());
    }
}
