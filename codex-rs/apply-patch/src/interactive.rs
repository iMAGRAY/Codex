//! Interactive workflow primitives for the apply_patch CLI.
//!
//! This module provides a lightweight, line-oriented interaction loop that lets the operator
//! preview each hunk, accept or reject it, and (for multi-chunk updates) cherry-pick individual
//! chunks. The interface intentionally avoids external dependencies so it can run in minimal
//! sandboxed environments.

use std::io::{self, Write};
use std::path::Path;

use thiserror::Error;

use crate::cli::{CliConfig, SummaryMode};
use crate::{
    ApplyPatchAction, ApplyPatchError, ApplyPatchFileChange, Hunk, MaybeApplyPatchVerified,
    Selection, SelectionEntry, UpdateFileChunk,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractiveCapability {
    Unsupported,
    Enabled,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InteractiveRequest<'a> {
    pub config: &'a CliConfig,
    pub plan: &'a ApplyPatchAction,
}

#[derive(Debug, Error)]
pub enum InteractiveError {
    #[error("Interactive mode requested but not supported in the current environment.")]
    Unsupported,
    #[error("Interactive selection aborted by user.")]
    Aborted,
    #[error("{0}")]
    Io(#[from] io::Error),
    #[error(transparent)]
    ApplyPatch(#[from] ApplyPatchError),
    #[error("Missing change metadata for {0}")]
    MissingChange(String),
}

/// Determine whether interactive mode should be enabled when the configuration requests `Auto`.
pub fn auto_detect_interactive(is_stdout_tty: bool, config: &CliConfig) -> InteractiveCapability {
    if matches!(config.summary_mode, SummaryMode::Quiet) {
        // Quiet mode normally indicates the caller is scripting the command.
        return InteractiveCapability::Unsupported;
    }

    if is_stdout_tty {
        InteractiveCapability::Enabled
    } else {
        InteractiveCapability::Unsupported
    }
}

/// Launches the interactive flow and returns the caller's selection.
///
/// When `--yes` is provided the function immediately returns `Selection::ApplyAll` so the caller
/// can still rely on a single entry point.
pub fn run_interactive(request: &InteractiveRequest) -> Result<Selection, InteractiveError> {
    if request.config.assume_yes {
        return Ok(Selection::ApplyAll);
    }

    let mut decisions = Vec::with_capacity(request.plan.hunks.len());
    for (index, hunk) in request.plan.hunks.iter().enumerate() {
        let abs_path = hunk.resolve_path(&request.plan.cwd);
        let display = display_path(&abs_path, &request.plan.cwd);
        let change = request
            .plan
            .changes()
            .get(&abs_path)
            .ok_or_else(|| InteractiveError::MissingChange(abs_path.display().to_string()))?;

        println!("─── [{}] {}", index, display);
        match change {
            ApplyPatchFileChange::Add { content } => {
                print_block("New file", format_removed_added_block(content.lines(), '+'))?;
                let apply = prompt_yes_no("Apply this addition?", true)?;
                decisions.push(if apply {
                    HunkDecision::Apply
                } else {
                    HunkDecision::Skip
                });
            }
            ApplyPatchFileChange::Delete { content } => {
                print_block(
                    "File will be deleted",
                    format_removed_added_block(content.lines(), '-'),
                )?;
                let apply = prompt_yes_no("Delete this file?", true)?;
                decisions.push(if apply {
                    HunkDecision::Apply
                } else {
                    HunkDecision::Skip
                });
            }
            ApplyPatchFileChange::Update { move_path, .. } => {
                if let Some(dest) = move_path {
                    let move_display = display_path(dest, &request.plan.cwd);
                    println!("rename → {move_display}");
                }
                let decision = decide_update_chunks(index, hunk)?;
                decisions.push(decision);
            }
        }
        println!();
    }

    let selection = decisions_to_selection(decisions);
    Ok(selection)
}

fn decide_update_chunks(hunk_index: usize, hunk: &Hunk) -> Result<HunkDecision, InteractiveError> {
    match hunk {
        Hunk::UpdateFile { chunks, .. } if chunks.len() <= 1 => {
            if let Some(chunk) = chunks.first() {
                print_chunk_preview(chunk);
            }
            let apply = prompt_yes_no("Apply this update?", true)?;
            Ok(if apply {
                HunkDecision::Apply
            } else {
                HunkDecision::Skip
            })
        }
        Hunk::UpdateFile { chunks, .. } => {
            println!("This update contains {} hunks:", chunks.len());
            for (chunk_index, chunk) in chunks.iter().enumerate() {
                println!("--- chunk {chunk_index} ---");
                print_chunk_preview(chunk);
            }
            let apply_all = prompt_yes_no("Apply all chunks for this file?", true)?;
            if apply_all {
                return Ok(HunkDecision::Apply);
            }
            let mut selected = Vec::new();
            for (chunk_index, chunk) in chunks.iter().enumerate() {
                println!("--- chunk {chunk_index} ---");
                print_chunk_preview(chunk);
                let apply = prompt_yes_no("Apply this chunk?", true)?;
                if apply {
                    selected.push(chunk_index);
                }
            }
            if selected.is_empty() {
                Ok(HunkDecision::Skip)
            } else if selected.len() == chunks.len() {
                Ok(HunkDecision::Apply)
            } else {
                Ok(HunkDecision::Chunks(selected))
            }
        }
        _ => Ok(HunkDecision::Apply),
    }
}

fn print_chunk_preview(chunk: &UpdateFileChunk) {
    if let Some(ctx) = &chunk.change_context {
        println!("@@ {ctx}");
    } else {
        println!("@@");
    }
    for line in &chunk.old_lines {
        println!("- {line}");
    }
    for line in &chunk.new_lines {
        println!("+ {line}");
    }
    if chunk.is_end_of_file {
        println!("*** End of File");
    }
}

fn format_removed_added_block<'a>(lines: impl Iterator<Item = &'a str>, prefix: char) -> String {
    let mut buf = String::new();
    for line in lines {
        buf.push_str("    ");
        buf.push(prefix);
        buf.push(' ');
        buf.push_str(line);
        buf.push('\n');
    }
    buf
}

fn print_block(title: &str, body: String) -> Result<(), InteractiveError> {
    println!("{title}:");
    print!("{body}");
    io::stdout().flush()?;
    Ok(())
}

fn prompt_yes_no(prompt: &str, default_yes: bool) -> Result<bool, InteractiveError> {
    let mut stdout = io::stdout();
    let hint = if default_yes { "[Y/n]" } else { "[y/N]" };
    loop {
        write!(stdout, "{prompt} {hint} ")?;
        stdout.flush()?;
        let mut buf = String::new();
        io::stdin().read_line(&mut buf)?;
        let trimmed = buf.trim().to_lowercase();
        if trimmed.is_empty() {
            return Ok(default_yes);
        }
        match trimmed.as_str() {
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            "abort" | "q" => return Err(InteractiveError::Aborted),
            _ => println!("Please answer 'y' or 'n' (or 'abort' to cancel)."),
        }
    }
}

fn decisions_to_selection(decisions: Vec<HunkDecision>) -> Selection {
    if decisions.iter().all(|d| matches!(d, HunkDecision::Apply)) {
        return Selection::ApplyAll;
    }
    if decisions.iter().all(|d| matches!(d, HunkDecision::Skip)) {
        return Selection::SkipAll;
    }
    let mut entries = Vec::new();
    for (index, decision) in decisions.into_iter().enumerate() {
        match decision {
            HunkDecision::Apply => {
                entries.push(SelectionEntry::EntireHunk { hunk_index: index });
            }
            HunkDecision::Skip => {}
            HunkDecision::Chunks(chunks) => {
                for chunk in chunks {
                    entries.push(SelectionEntry::UpdateChunk {
                        hunk_index: index,
                        chunk_index: chunk,
                    });
                }
            }
        }
    }
    Selection::Explicit(entries)
}

fn display_path(path: &Path, base: &Path) -> String {
    path.strip_prefix(base)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
}

#[derive(Debug)]
enum HunkDecision {
    Apply,
    Skip,
    Chunks(Vec<usize>),
}

/// Convert CLI invocation into an actionable plan without applying filesystem changes.
pub fn preview_hunks(
    verification: MaybeApplyPatchVerified,
) -> Result<Option<ApplyPatchAction>, ApplyPatchError> {
    match verification {
        MaybeApplyPatchVerified::Body(action) => Ok(Some(action)),
        MaybeApplyPatchVerified::CorrectnessError(err) => Err(err),
        MaybeApplyPatchVerified::ShellParseError(_) | MaybeApplyPatchVerified::NotApplyPatch => {
            Ok(None)
        }
    }
}
