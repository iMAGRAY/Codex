use std::fs;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Result;
use assert_cmd::Command;
use serde_json::Value as JsonValue;
use tempfile::TempDir;

fn codex_command(codex_home: &Path) -> Result<Command> {
    let mut cmd = Command::cargo_bin("codex")?;
    cmd.env("CODEX_HOME", codex_home);
    Ok(cmd)
}

fn write_experimental_config(home: &Path) -> Result<()> {
    let contents = "[experimental]\nmcp_overhaul = true\n";
    fs::write(home.join("config.toml"), contents)?;
    Ok(())
}

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../fixtures/mcp")
        .join(name)
}

#[test]
fn wizard_json_with_source_outputs_analysis() -> Result<()> {
    let codex_home = TempDir::new()?;
    write_experimental_config(codex_home.path())?;

    let source_dir = fixture_path("node-basic");
    assert!(source_dir.exists(), "fixture {:?} missing", source_dir);

    let mut cmd = codex_command(codex_home.path())?;
    let output = cmd
        .args([
            "mcp",
            "wizard",
            "--json",
            "--name",
            "node-basic",
            "--command",
            "node",
            "--arg",
            "server.js",
            "--source",
            source_dir.to_str().unwrap(),
        ])
        .output()?;

    assert!(
        output.status.success(),
        "wizard command failed: {:?}",
        output
    );
    let stdout = String::from_utf8(output.stdout)?;
    let json: JsonValue = serde_json::from_str(&stdout)?;

    let source = json
        .get("source")
        .and_then(|value| value.as_object())
        .expect("source report present");
    assert_eq!(
        source.get("path"),
        Some(&JsonValue::String(
            source_dir.to_string_lossy().into_owned()
        ))
    );

    let key_files = source
        .get("key_files")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(
        key_files.iter().any(|value| value == "package.json"),
        "key files include package.json"
    );

    let policy_warnings = source
        .get("policy_warnings")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(
        policy_warnings.iter().all(|value| value.is_string()),
        "policy warnings should be human readable strings"
    );

    assert_eq!(source.get("error"), None, "analysis succeeded");

    let fingerprint_file = codex_home
        .path()
        .join("cache")
        .join("mcp_intake_fingerprints.json");
    assert!(fingerprint_file.exists(), "fingerprint store created");

    Ok(())
}
