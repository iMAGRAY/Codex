use assert_cmd::prelude::*;
use predicates::prelude::*;
use std::fs;
use std::process::Command;
use tempfile::tempdir;

#[test]
fn test_apply_patch_cli_add_and_update() -> anyhow::Result<()> {
    let tmp = tempdir()?;
    let file = "cli_test.txt";
    let absolute_path = tmp.path().join(file);

    let add_patch = format!(
        r#"*** Begin Patch
*** Add File: {file}
+hello
*** End Patch"#
    );

    Command::cargo_bin("apply_patch")
        .expect("should find apply_patch binary")
        .arg("--yes")
        .arg(add_patch)
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "Applied changes:
  A {file}
"
        )));
    assert_eq!(
        fs::read_to_string(&absolute_path)?,
        "hello
"
    );

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
        .arg("--yes")
        .arg(update_patch)
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "Applied changes:
  M {file}
"
        )));
    assert_eq!(
        fs::read_to_string(&absolute_path)?,
        "world
"
    );

    Ok(())
}

#[test]
fn test_apply_patch_cli_stdin_add_and_update() -> anyhow::Result<()> {
    let tmp = tempdir()?;
    let file = "cli_test_stdin.txt";
    let absolute_path = tmp.path().join(file);

    let add_patch = format!(
        r#"*** Begin Patch
*** Add File: {file}
+hello
*** End Patch"#
    );

    let mut cmd = Command::cargo_bin("apply_patch").expect("should find apply_patch binary");
    cmd.current_dir(tmp.path());
    cmd.arg("--yes");
    cmd.write_stdin(add_patch)
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "Applied changes:
  A {file}
"
        )));
    assert_eq!(
        fs::read_to_string(&absolute_path)?,
        "hello
"
    );

    let update_patch = format!(
        r#"*** Begin Patch
*** Update File: {file}
@@
-hello
+world
*** End Patch"#
    );

    let mut cmd = Command::cargo_bin("apply_patch").expect("should find apply_patch binary");
    cmd.current_dir(tmp.path());
    cmd.arg("--yes");
    cmd.write_stdin(update_patch)
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "Applied changes:
  M {file}
"
        )));
    assert_eq!(
        fs::read_to_string(&absolute_path)?,
        "world
"
    );

    Ok(())
}

#[test]
fn test_apply_patch_cli_dry_run() -> anyhow::Result<()> {
    let tmp = tempdir()?;
    let file = "cli_dry_run.txt";
    let patch = format!(
        r#"*** Begin Patch
*** Add File: {file}
+hello
*** End Patch"#
    );

    Command::cargo_bin("apply_patch")
        .expect("should find apply_patch binary")
        .arg("--dry-run")
        .arg("--yes")
        .arg(patch)
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "Dry run summary:
  A {file}
"
        )));

    assert!(!tmp.path().join(file).exists());
    assert!(!tmp.path().join(".codex/apply_patch_history.json").exists());

    Ok(())
}

#[test]
fn test_apply_patch_cli_undo_last() -> anyhow::Result<()> {
    let tmp = tempdir()?;
    let file = "cli_undo.txt";
    let patch = format!(
        r#"*** Begin Patch
*** Add File: {file}
+hello
*** End Patch"#
    );

    Command::cargo_bin("apply_patch")
        .expect("should find apply_patch binary")
        .arg("--yes")
        .arg(&patch)
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "Applied changes:
  A {file}
"
        )));

    let created_file = tmp.path().join(file);
    assert!(created_file.exists());

    Command::cargo_bin("apply_patch")
        .expect("should find apply_patch binary")
        .arg("--undo-last")
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Applied changes:
  D cli_undo.txt
",
        ));

    assert!(!created_file.exists());
    assert!(!tmp.path().join(".codex/apply_patch_history.json").exists());

    Ok(())
}
