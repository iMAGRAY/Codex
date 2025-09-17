use std::path::Path;

use anyhow::Result;
use predicates::prelude::PredicateBooleanExt;
use predicates::str::contains;
use pretty_assertions::assert_eq;
use serde_json::Value as JsonValue;
use tempfile::TempDir;

fn codex_command(codex_home: &Path) -> Result<assert_cmd::Command> {
    let mut cmd = assert_cmd::Command::cargo_bin("codex")?;
    cmd.env("CODEX_HOME", codex_home);
    Ok(cmd)
}

#[test]
fn list_shows_empty_state() -> Result<()> {
    let codex_home = TempDir::new()?;

    let mut cmd = codex_command(codex_home.path())?;
    let output = cmd.args(["mcp", "list"]).output()?;
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("No MCP servers configured yet."));

    Ok(())
}

#[test]
fn list_and_get_render_expected_output() -> Result<()> {
    let codex_home = TempDir::new()?;

    let mut add = codex_command(codex_home.path())?;
    add.args([
        "mcp",
        "add",
        "docs",
        "--env",
        "TOKEN=secret",
        "--",
        "docs-server",
        "--port",
        "4000",
    ])
    .assert()
    .success();

    let mut list_cmd = codex_command(codex_home.path())?;
    let list_output = list_cmd.args(["mcp", "list"]).output()?;
    assert!(list_output.status.success());
    let stdout = String::from_utf8(list_output.stdout)?;
    assert!(stdout.contains("Name"));
    assert!(stdout.contains("docs"));
    assert!(stdout.contains("docs-server"));
    assert!(stdout.contains("TOKEN=secret"));
    assert!(stdout.contains("Status"));
    assert!(stdout.contains("Auth"));
    assert!(stdout.contains("enabled"));
    assert!(stdout.contains("Unsupported"));

    let mut list_json_cmd = codex_command(codex_home.path())?;
    let json_output = list_json_cmd.args(["mcp", "list", "--json"]).output()?;
    assert!(json_output.status.success());
    let stdout = String::from_utf8(json_output.stdout)?;
    let parsed: JsonValue = serde_json::from_str(&stdout)?;
    let servers = parsed.as_array().expect("list response must be an array");
    assert_eq!(servers.len(), 1, "expected a single server entry");
    let server = servers[0]
        .as_object()
        .expect("server entry should be an object");

    let get_str = |key: &str| {
        server
            .get(key)
            .and_then(JsonValue::as_str)
            .unwrap_or_else(|| panic!("missing {key}"))
    };
    let get_null = |key: &str| {
        server
            .get(key)
            .unwrap_or_else(|| panic!("missing {key}"))
            .is_null()
    };

    assert_eq!(get_str("name"), "docs");
    assert!(get_null("display_name"));
    assert!(get_null("category"));
    assert!(get_null("template_id"));
    assert!(get_null("description"));
    assert_eq!(get_str("command"), "docs-server");
    let args = server
        .get("args")
        .and_then(JsonValue::as_array)
        .expect("args should be an array");
    let args_strings: Vec<_> = args
        .iter()
        .map(|v| v.as_str().expect("args entries should be strings"))
        .collect();
    assert_eq!(args_strings, ["--port", "4000"]);
    let env = server
        .get("env")
        .and_then(JsonValue::as_object)
        .expect("env should be an object");
    assert_eq!(
        env.get("TOKEN")
            .and_then(JsonValue::as_str)
            .expect("TOKEN entry should exist"),
        "secret"
    );
    assert!(get_null("url"));
    assert!(get_null("bearer_token_env_var"));
    let tags = server
        .get("tags")
        .and_then(JsonValue::as_array)
        .expect("tags should be an array");
    assert!(tags.is_empty());
    assert!(get_null("created_at"));
    assert!(get_null("last_verified_at"));
    assert!(get_null("metadata"));
    assert!(get_null("auth"));
    assert!(get_null("healthcheck"));
    assert_eq!(
        server
            .get("enabled")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false),
        true
    );
    let transport = server
        .get("transport")
        .and_then(JsonValue::as_object)
        .expect("transport should be an object");
    assert_eq!(
        transport
            .get("type")
            .and_then(JsonValue::as_str)
            .expect("transport.type should exist"),
        "stdio"
    );
    assert_eq!(
        transport
            .get("command")
            .and_then(JsonValue::as_str)
            .expect("transport.command should exist"),
        "docs-server"
    );
    let transport_args = transport
        .get("args")
        .and_then(JsonValue::as_array)
        .expect("transport.args should be an array");
    let transport_args_strings: Vec<_> = transport_args
        .iter()
        .map(|v| {
            v.as_str()
                .expect("transport args entries should be strings")
        })
        .collect();
    assert_eq!(transport_args_strings, ["--port", "4000"]);
    let transport_env = transport
        .get("env")
        .and_then(JsonValue::as_object)
        .expect("transport.env should be an object");
    assert_eq!(
        transport_env
            .get("TOKEN")
            .and_then(JsonValue::as_str)
            .expect("transport env TOKEN"),
        "secret"
    );
    assert!(get_null("startup_timeout_sec"));
    assert!(get_null("tool_timeout_sec"));
    assert_eq!(
        server
            .get("auth_status")
            .and_then(JsonValue::as_str)
            .expect("auth_status should exist"),
        "unsupported"
    );

    let mut get_cmd = codex_command(codex_home.path())?;
    let get_output = get_cmd.args(["mcp", "get", "docs"]).output()?;
    assert!(get_output.status.success());
    let stdout = String::from_utf8(get_output.stdout)?;
    assert!(stdout.contains("docs"));
    assert!(stdout.contains("transport: stdio"));
    assert!(stdout.contains("command: docs-server"));
    assert!(stdout.contains("args: --port 4000"));
    assert!(stdout.contains("env: TOKEN=secret"));
    assert!(stdout.contains("enabled: true"));
    assert!(stdout.contains("remove: codex mcp remove docs"));

    let mut get_json_cmd = codex_command(codex_home.path())?;
    get_json_cmd
        .args(["mcp", "get", "docs", "--json"])
        .assert()
        .success()
        .stdout(contains("\"name\": \"docs\"").and(contains("\"enabled\": true")));

    Ok(())
}
