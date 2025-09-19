use std::io::{self, IsTerminal as _, Write};
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

use crate::Selection::{ApplyAll, Explicit, SkipAll};
use crate::cli::{
    self, CliConfig, CliError, CliInvocation, Command as CliCommand, InteractiveMode, PreviewMode,
    SummaryMode,
};
use crate::history::{self, UndoError, UndoRecord};
use crate::interactive::{self, InteractiveCapability, InteractiveError, InteractiveRequest};
use crate::{
    ApplyPatchAction, ApplyPatchError, MaybeApplyPatchVerified, apply_patch,
    build_selected_patch, maybe_parse_apply_patch_verified,
};

pub fn main() -> ! {
    let exit_code = run_main();
    std::process::exit(exit_code);
}

/// We would prefer to return `std::process::ExitCode`, but its `exit_process()`
/// method is still a nightly API and we want main() to return !.
pub fn run_main() -> i32 {
    match cli::parse_from_env() {
        Ok(invocation) => execute(invocation),
        Err(CliError::DisplayedHelp) => {
            let _ = cli::print_usage(std::io::stdout());
            0
        }
        Err(err) => render_cli_error(err),
    }
}

fn execute(invocation: CliInvocation) -> i32 {
    match invocation.command.clone() {
        CliCommand::UndoLast => match run_undo(&invocation.config) {
            Ok(()) => 0,
            Err(err) => {
                eprintln!("{}", err);
                err.exit_code()
            }
        },
        CliCommand::Apply { patch } => match run_apply(invocation, patch) {
            Ok(()) => 0,
            Err(err) => {
                eprintln!("{}", err);
                err.exit_code()
            }
        },
    }
}

fn render_cli_error(err: CliError) -> i32 {
    match err {
        CliError::DisplayedHelp => 0,
        CliError::InvalidPatchUtf8 => {
            eprintln!("{}", err);
            1
        }
        CliError::TooManyArguments => {
            eprintln!("{}", err);
            2
        }
        CliError::FailedToReadPatch(_) => {
            eprintln!("{}", err);
            1
        }
        CliError::MissingPatch => {
            eprintln!("{}", err);
            2
        }
        CliError::MissingRunAfterValue => {
            eprintln!("{}", err);
            2
        }
        CliError::Usage(message) | CliError::FlagConflict(message) => {
            eprintln!("{message}");
            let _ = cli::print_usage(std::io::stderr());
            2
        }
    }
}

fn run_apply(invocation: CliInvocation, patch: String) -> Result<(), WorkflowError> {
    let cwd = std::env::current_dir().map_err(WorkflowError::from)?;
    let action = parse_action(&patch, &cwd)?;

    let should_interactive = determine_interactive(&invocation.config);
    let mut selection = if should_interactive {
        let request = InteractiveRequest {
            config: &invocation.config,
            plan: &action,
        };
        interactive::run_interactive(&request)?
    } else {
        if invocation.config.assume_yes {
            ApplyAll
        } else {
            ApplyAll
        }
    };

    if matches!(selection, ApplyAll) && invocation.config.assume_yes {
        selection = ApplyAll;
    }

    let mut working_action = match &selection {
        ApplyAll => action.clone(),
        SkipAll => {
            if invocation.config.summary_mode != SummaryMode::Quiet {
                println!("No changes applied.");
            }
            return Ok(());
        }
        Explicit(_) => {
            let maybe_selected = build_selected_patch(&action.hunks, &selection, &action.cwd)
                .map_err(WorkflowError::from)?;
            match maybe_selected {
                Some(selected) => selected.action,
                None => {
                    if invocation.config.summary_mode != SummaryMode::Quiet {
                        println!("No changes selected.");
                    }
                    return Ok(());
                }
            }
        }
    };

    let should_preview = invocation.config.dry_run
        || matches!(invocation.config.preview_mode, PreviewMode::Always)
        || (matches!(invocation.config.preview_mode, PreviewMode::Auto) && should_interactive);

    if should_preview && !should_interactive {
        print_preview(&working_action, &invocation.config)?;
    }

    if should_interactive && !invocation.config.assume_yes && !invocation.config.dry_run {
        if !prompt_yes_no("Apply this patch?", true)? {
            if invocation.config.summary_mode != SummaryMode::Quiet {
                println!("Patch application cancelled.");
            }
            return Ok(());
        }
    }

    if invocation.config.dry_run {
        if invocation.config.summary_mode == SummaryMode::Detailed {
            let summary = compute_summary(&working_action);
            print_summary(&summary, &invocation.config.summary_mode, true)?;
        } else if invocation.config.summary_mode != SummaryMode::Quiet {
            println!("Dry run completed – no files were changed.");
        }
        return Ok(());
    }

    let undo_record = UndoRecord::build(&working_action);
    {
        let _guard = DirGuard::change_to(&working_action.cwd)?;
        apply_selected_patch(&working_action, invocation.config.summary_mode)?;
    }

    history::store_last_record(&working_action.cwd, &undo_record)?;

    if !invocation.config.run_after.is_empty() {
        run_post_commands(&invocation.config.run_after, &working_action.cwd)?;
    }

    Ok(())
}

fn run_undo(config: &CliConfig) -> Result<(), WorkflowError> {
    let cwd = std::env::current_dir().map_err(WorkflowError::from)?;
    let record = match history::load_last_record(&cwd)? {
        Some(record) => record,
        None => return Err(WorkflowError::Message("No undo history found.".to_string())),
    };

    history::apply_undo(&record)?;
    history::clear_history(&cwd)?;

    if config.summary_mode != SummaryMode::Quiet {
        let summary = summarize_undo(&record);
        print_summary(&summary, &config.summary_mode, false)?;
    }
    Ok(())
}

fn apply_selected_patch(
    action: &ApplyPatchAction,
    summary_mode: SummaryMode,
) -> Result<(), WorkflowError> {
    let mut stdout_buf = Vec::new();
    let mut stderr_buf = Vec::new();
    match apply_patch(&action.patch, &mut stdout_buf, &mut stderr_buf) {
        Ok(()) => {
            if !stderr_buf.is_empty() {
                eprint!("{}", String::from_utf8_lossy(&stderr_buf));
            }
            if summary_mode != SummaryMode::Quiet {
                let summary = compute_summary(action);
                print_summary(&summary, &summary_mode, false)?;
            }
            Ok(())
        }
        Err(err) => {
            if !stderr_buf.is_empty() {
                eprint!("{}", String::from_utf8_lossy(&stderr_buf));
            }
            Err(WorkflowError::from(err))
        }
    }
}

fn parse_action(patch: &str, cwd: &Path) -> Result<ApplyPatchAction, WorkflowError> {
    let argv = vec!["apply_patch".to_string(), patch.to_string()];
    match maybe_parse_apply_patch_verified(&argv, cwd) {
        MaybeApplyPatchVerified::Body(action) => Ok(action),
        MaybeApplyPatchVerified::CorrectnessError(err) => {
            Err(WorkflowError::PatchParse(err.to_string()))
        }
        MaybeApplyPatchVerified::ShellParseError(err) => {
            Err(WorkflowError::PatchParse(err.to_string()))
        }
        MaybeApplyPatchVerified::NotApplyPatch => Err(WorkflowError::PatchParse(
            "Provided input is not a valid apply_patch payload.".to_string(),
        )),
    }
}

fn determine_interactive(config: &CliConfig) -> bool {
    match config.interactive_mode {
        InteractiveMode::Enabled => true,
        InteractiveMode::Disabled => false,
        InteractiveMode::Auto => {
            let stdout_tty = std::io::stdout().is_terminal();
            matches!(
                interactive::auto_detect_interactive(stdout_tty, config),
                InteractiveCapability::Enabled
            )
        }
    }
}

fn prompt_yes_no(prompt: &str, default_yes: bool) -> Result<bool, WorkflowError> {
    let mut stdout = std::io::stdout();
    let default_hint = if default_yes { "[Y/n]" } else { "[y/N]" };
    loop {
        write!(stdout, "{prompt} {default_hint} ").map_err(WorkflowError::from)?;
        stdout.flush().map_err(WorkflowError::from)?;
        let mut input = String::new();
        std::io::stdin()
            .read_line(&mut input)
            .map_err(WorkflowError::from)?;
        let trimmed = input.trim().to_lowercase();
        if trimmed.is_empty() {
            return Ok(default_yes);
        }
        match trimmed.as_str() {
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => {
                println!("Please answer 'y' or 'n'.");
            }
        }
    }
}

fn print_preview(action: &ApplyPatchAction, config: &CliConfig) -> Result<(), WorkflowError> {
    if config.summary_mode != SummaryMode::Quiet {
        println!("Planned changes:");
    }
    for (index, hunk) in action.hunks.iter().enumerate() {
        let abs_path = hunk.resolve_path(&action.cwd);
        let display_path = display_path(&abs_path, &action.cwd);
        let change = action.changes().get(&abs_path).ok_or_else(|| {
            WorkflowError::Message(format!(
                "Missing change metadata for {}",
                abs_path.display()
            ))
        })?;
        println!("  [{}] {}", index, display_path);
        match change {
            crate::ApplyPatchFileChange::Add { content } => {
                for line in content.lines() {
                    println!("    + {line}");
                }
            }
            crate::ApplyPatchFileChange::Delete { content } => {
                for line in content.lines() {
                    println!("    - {line}");
                }
            }
            crate::ApplyPatchFileChange::Update {
                unified_diff,
                move_path,
                ..
            } => {
                if let Some(dest) = move_path {
                    let display_move = display_path(dest, &action.cwd);
                    println!("    move to: {display_move}");
                }
                for diff_line in unified_diff.lines() {
                    println!("    {diff_line}");
                }
            }
        }
        println!();
    }
    Ok(())
}

fn compute_summary(action: &ApplyPatchAction) -> Summary {
    let mut summary = Summary::default();
    for (path, change) in action.changes() {
        match change {
            crate::ApplyPatchFileChange::Add { content } => {
                summary.added.push(display_path(path, &action.cwd));
                summary.lines_added += count_added_lines(content);
            }
            crate::ApplyPatchFileChange::Delete { content } => {
                summary.deleted.push(display_path(path, &action.cwd));
                summary.lines_removed += count_removed_lines(content);
            }
            crate::ApplyPatchFileChange::Update {
                unified_diff,
                move_path,
                ..
            } => {
                let mut label = display_path(path, &action.cwd);
                if let Some(dest) = move_path {
                    label.push_str(" -> ");
                    label.push_str(&display_path(dest, &action.cwd));
                }
                summary.modified.push(label);
                let (added, removed) = count_diff_lines(unified_diff);
                summary.lines_added += added;
                summary.lines_removed += removed;
            }
        }
    }
    summary
}

fn summarize_undo(record: &UndoRecord) -> Summary {
    let mut summary = Summary::default();
    for entry in &record.entries {
        match entry {
            history::UndoEntry::Added { path, .. } => {
                summary
                    .deleted
                    .push(display_path(path, path.parent().unwrap_or(Path::new(""))));
            }
            history::UndoEntry::Deleted { path, .. } => {
                summary
                    .added
                    .push(display_path(path, path.parent().unwrap_or(Path::new(""))));
            }
            history::UndoEntry::Updated {
                original_path,
                moved_path,
                ..
            } => {
                let mut label = display_path(
                    original_path,
                    original_path.parent().unwrap_or(Path::new("")),
                );
                if let Some(moved) = moved_path {
                    label.push_str(" <- ");
                    label.push_str(&display_path(
                        moved,
                        moved.parent().unwrap_or(Path::new("")),
                    ));
                }
                summary.modified.push(label);
            }
        }
    }
    summary
}

fn print_summary(
    summary: &Summary,
    mode: &SummaryMode,
    dry_run: bool,
) -> Result<(), WorkflowError> {
    let mut stdout = std::io::stdout();
    match mode {
        SummaryMode::Quiet => {}
        SummaryMode::Standard => {
            if dry_run {
                writeln!(stdout, "Dry run summary:").map_err(WorkflowError::from)?;
            } else {
                writeln!(stdout, "Applied changes:").map_err(WorkflowError::from)?;
            }
            for path in &summary.added {
                writeln!(stdout, "  A {path}").map_err(WorkflowError::from)?;
            }
            for path in &summary.modified {
                writeln!(stdout, "  M {path}").map_err(WorkflowError::from)?;
            }
            for path in &summary.deleted {
                writeln!(stdout, "  D {path}").map_err(WorkflowError::from)?;
            }
        }
        SummaryMode::Detailed => {
            if dry_run {
                writeln!(stdout, "Dry run summary:").map_err(WorkflowError::from)?;
            } else {
                writeln!(stdout, "Apply summary:").map_err(WorkflowError::from)?;
            }
            writeln!(stdout, "  Added: {}", summary.added.len()).map_err(WorkflowError::from)?;
            writeln!(stdout, "  Modified: {}", summary.modified.len())
                .map_err(WorkflowError::from)?;
            writeln!(stdout, "  Deleted: {}", summary.deleted.len())
                .map_err(WorkflowError::from)?;
            writeln!(
                stdout,
                "  Lines +{} / -{}",
                summary.lines_added, summary.lines_removed
            )
            .map_err(WorkflowError::from)?;
            if !summary.added.is_empty() {
                writeln!(stdout, "  Added files:").map_err(WorkflowError::from)?;
                for path in &summary.added {
                    writeln!(stdout, "    {path}").map_err(WorkflowError::from)?;
                }
            }
            if !summary.modified.is_empty() {
                writeln!(stdout, "  Modified files:").map_err(WorkflowError::from)?;
                for path in &summary.modified {
                    writeln!(stdout, "    {path}").map_err(WorkflowError::from)?;
                }
            }
            if !summary.deleted.is_empty() {
                writeln!(stdout, "  Deleted files:").map_err(WorkflowError::from)?;
                for path in &summary.deleted {
                    writeln!(stdout, "    {path}").map_err(WorkflowError::from)?;
                }
            }
        }
    }
    stdout.flush().map_err(WorkflowError::from)?;
    Ok(())
}

fn display_path(path: &Path, base: &Path) -> String {
    path.strip_prefix(base)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
}

fn count_added_lines(content: &str) -> usize {
    if content.is_empty() {
        0
    } else {
        content.lines().count()
    }
}

fn count_removed_lines(content: &str) -> usize {
    if content.is_empty() {
        0
    } else {
        content.lines().count()
    }
}

fn count_diff_lines(diff: &str) -> (usize, usize) {
    let mut added = 0;
    let mut removed = 0;
    for line in diff.lines() {
        if line.starts_with("+++") || line.starts_with("---") {
            continue;
        }
        if line.starts_with('+') {
            added += 1;
        } else if line.starts_with('-') {
            removed += 1;
        }
    }
    (added, removed)
}

fn run_post_commands(commands: &[String], cwd: &Path) -> Result<(), WorkflowError> {
    for command in commands {
        let status = if cfg!(windows) {
            ProcessCommand::new("cmd")
                .args(["/C", command])
                .current_dir(cwd)
                .status()
        } else {
            ProcessCommand::new("sh")
                .args(["-c", command])
                .current_dir(cwd)
                .status()
        }
        .map_err(WorkflowError::from)?;
        if !status.success() {
            return Err(WorkflowError::Message(format!(
                "Post-command `{}` failed with status {}",
                command, status
            )));
        }
    }
    Ok(())
}

#[derive(Default)]
struct Summary {
    added: Vec<String>,
    modified: Vec<String>,
    deleted: Vec<String>,
    lines_added: usize,
    lines_removed: usize,
}

#[derive(Debug, thiserror::Error)]
enum WorkflowError {
    #[error("{0}")]
    Message(String),
    #[error("Patch parse error: {0}")]
    PatchParse(String),
    #[error(transparent)]
    Apply(#[from] ApplyPatchError),
    #[error(transparent)]
    Undo(#[from] UndoError),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Interactive(#[from] InteractiveError),
}

impl WorkflowError {
    fn exit_code(&self) -> i32 {
        match self {
            WorkflowError::PatchParse(_) => 2,
            WorkflowError::Message(_) => 1,
            WorkflowError::Apply(_) => 1,
            WorkflowError::Undo(_) => 1,
            WorkflowError::Io(_) => 1,
            WorkflowError::Interactive(_) => 1,
        }
    }
}

struct DirGuard {
    original: PathBuf,
}

impl DirGuard {
    fn change_to(target: &Path) -> Result<Option<Self>, WorkflowError> {
        let current = std::env::current_dir().map_err(WorkflowError::from)?;
        if current == target {
            return Ok(None);
        }
        std::env::set_current_dir(target).map_err(WorkflowError::from)?;
        Ok(Some(Self { original: current }))
    }
}

impl Drop for DirGuard {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.original);
    }
}
