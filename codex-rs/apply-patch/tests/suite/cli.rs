use assert_cmd::prelude::*;
use serde_json::Value;
use std::fs;
use std::process::Command;
use tempfile::tempdir;

#[test]
fn test_apply_patch_cli_add_and_update() -> anyhow::Result<()> {
    let tmp = tempdir()?;
    let file = "cli_test.txt";
    let absolute_path = tmp.path().join(file);

    // 1) Add a file
    let add_patch = format!(
        r#"*** Begin Patch
*** Add File: {file}
+hello
*** End Patch"#
    );
    Command::cargo_bin("apply_patch")
        .expect("should find apply_patch binary")
        .arg(add_patch)
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(format!(
            "Applied operations:\n- add: {file} (+1)\n✔ Patch applied successfully.\n"
        ));
    assert_eq!(fs::read_to_string(&absolute_path)?, "hello\n");

    // 2) Update the file
    let update_patch = format!(
        r#"*** Begin Patch
*** Update File: {file}
@@
-hello
+world
*** End Patch"#
    );
    Command::cargo_bin("apply_patch")
        .expect("should find apply_patch binary")
        .arg(update_patch)
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(format!(
            "Applied operations:\n- update: {file} (+1, -1)\n✔ Patch applied successfully.\n"
        ));
    assert_eq!(fs::read_to_string(&absolute_path)?, "world\n");

    Ok(())
}

#[test]
fn test_apply_patch_cli_stdin_add_and_update() -> anyhow::Result<()> {
    let tmp = tempdir()?;
    let file = "cli_test_stdin.txt";
    let absolute_path = tmp.path().join(file);

    // 1) Add a file via stdin
    let add_patch = format!(
        r#"*** Begin Patch
*** Add File: {file}
+hello
*** End Patch"#
    );
    let mut cmd =
        assert_cmd::Command::cargo_bin("apply_patch").expect("should find apply_patch binary");
    cmd.current_dir(tmp.path());
    cmd.write_stdin(add_patch)
        .assert()
        .success()
        .stdout(format!(
            "Applied operations:\n- add: {file} (+1)\n✔ Patch applied successfully.\n"
        ));
    assert_eq!(fs::read_to_string(&absolute_path)?, "hello\n");

    // 2) Update the file via stdin
    let update_patch = format!(
        r#"*** Begin Patch
*** Update File: {file}
@@
-hello
+world
*** End Patch"#
    );
    let mut cmd =
        assert_cmd::Command::cargo_bin("apply_patch").expect("should find apply_patch binary");
    cmd.current_dir(tmp.path());
    cmd.write_stdin(update_patch)
        .assert()
        .success()
        .stdout(format!(
            "Applied operations:\n- update: {file} (+1, -1)\n✔ Patch applied successfully.\n"
        ));
    assert_eq!(fs::read_to_string(&absolute_path)?, "world\n");

    Ok(())
}

#[test]
fn test_apply_patch_cli_delete_file() -> anyhow::Result<()> {
    let tmp = tempdir()?;
    let file = "cli_delete.txt";
    let absolute_path = tmp.path().join(file);
    fs::write(
        &absolute_path,
        "obsolete
",
    )?;

    let delete_patch = format!(
        r"*** Begin Patch
*** Delete File: {file}
*** End Patch"
    );
    Command::cargo_bin("apply_patch")
        .expect("should find apply_patch binary")
        .arg(delete_patch)
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(format!(
            "Applied operations:
- delete: {file} (-1)
✔ Patch applied successfully.
"
        ));
    assert!(
        !absolute_path.exists(),
        "{file} should be removed after apply_patch"
    );

    Ok(())
}

#[test]
fn test_apply_patch_cli_move_file() -> anyhow::Result<()> {
    let tmp = tempdir()?;
    let src = "cli_move_src.txt";
    let dest = "cli_move_dest.txt";
    let src_path = tmp.path().join(src);
    fs::write(
        &src_path,
        "first line
",
    )?;

    let move_patch = format!(
        r"*** Begin Patch
*** Update File: {src}
*** Move to: {dest}
@@
-first line
+second line
*** End Patch"
    );
    Command::cargo_bin("apply_patch")
        .expect("should find apply_patch binary")
        .arg(move_patch)
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(format!(
            "Applied operations:
- move: {src} -> {dest} (+1, -1)
✔ Patch applied successfully.
"
        ));

    assert!(
        !src_path.exists(),
        "source file should be removed after move"
    );
    let dest_path = tmp.path().join(dest);
    assert_eq!(
        fs::read_to_string(&dest_path)?,
        "second line
"
    );

    Ok(())
}

#[test]
fn test_apply_patch_cli_machine_output() -> anyhow::Result<()> {
    let tmp = tempdir()?;
    let file = "cli_machine.txt";

    let add_patch = format!(
        r"*** Begin Patch
*** Add File: {file}
+machine
*** End Patch"
    );

    let assert = Command::cargo_bin("apply_patch")?
        .arg("--machine")
        .arg(&add_patch)
        .current_dir(tmp.path())
        .assert()
        .success();
    let output = assert.get_output();
    let stdout = String::from_utf8(output.stdout.clone())?;
    let first_line = stdout.trim_end_matches('\n');
    let json: Value = serde_json::from_str(first_line)?;
    assert_eq!(
        json.get("schema").and_then(Value::as_str),
        Some("apply_patch/v2"),
        "machine output should advertise schema"
    );

    let report = json
        .get("report")
        .and_then(Value::as_object)
        .expect("machine output embeds report object");
    assert_eq!(
        report.get("status").and_then(Value::as_str),
        Some("success")
    );

    Ok(())
}
#[test]
fn test_apply_patch_cli_writes_log_file() -> anyhow::Result<()> {
    let tmp = tempdir()?;
    let file = "cli_log.txt";
    let patch = format!(
        "*** Begin Patch
*** Add File: {file}
+log test
*** End Patch"
    );

    Command::cargo_bin("apply_patch")?
        .arg(&patch)
        .current_dir(tmp.path())
        .assert()
        .success();

    let log_dir = tmp.path().join("reports/logs");
    let entries: Vec<_> = fs::read_dir(&log_dir)?.collect();
    assert!(!entries.is_empty(), "expected log files to be written");
    Ok(())
}

#[test]
fn test_apply_patch_cli_respects_no_logs() -> anyhow::Result<()> {
    let tmp = tempdir()?;
    let file = "cli_no_log.txt";
    let patch = format!(
        "*** Begin Patch
*** Add File: {file}
+no log
*** End Patch"
    );

    Command::cargo_bin("apply_patch")?
        .args(["--no-logs"])
        .arg(&patch)
        .current_dir(tmp.path())
        .assert()
        .success();

    let log_dir = tmp.path().join("reports/logs");
    assert!(!log_dir.exists(), "logs directory should not exist");
    Ok(())
}

#[test]
fn test_apply_patch_cli_writes_conflict_hint_on_failure() -> anyhow::Result<()> {
    let tmp = tempdir()?;
    let file = tmp.path().join("conflict.txt");
    fs::write(&file, "current\n")?;

    let patch = "*** Begin Patch
*** Update File: conflict.txt
@@
-original
+updated
*** End Patch";

    Command::cargo_bin("apply_patch")?
        .args(["--conflict-dir", "conflicts", "--no-logs"])
        .arg(patch)
        .current_dir(tmp.path())
        .assert()
        .failure();

    let conflict_dir = tmp.path().join("conflicts");
    let hints: Vec<_> = fs::read_dir(&conflict_dir)?.collect();
    assert!(!hints.is_empty(), "expected conflict hint to be written");
    let hint_path = hints[0].as_ref().unwrap().path();
    let hint_contents = fs::read_to_string(hint_path)?;
    assert!(hint_contents.contains("original"));
    Ok(())
}
