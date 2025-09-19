use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

#[test]
fn audit_export_prints_valid_json() -> Result<(), Box<dyn std::error::Error>> {
    let codex_home = TempDir::new()?;

    let output = Command::cargo_bin("codex")?
        .arg("audit")
        .arg("export")
        .arg("--pretty")
        .env("CODEX_HOME", codex_home.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: Value = serde_json::from_slice(&output)?;
    assert!(json.get("records").is_some(), "records key missing");
    if let Some(evidence) = json.get("policy_evidence") {
        assert!(evidence.is_array(), "policy_evidence must be an array");
    }

    Ok(())
}
