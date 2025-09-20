use crate::codex::Session;
use crate::codex::TurnContext;
use crate::protocol::FileChange;
use crate::protocol::ReviewDecision;
use crate::safety::assess_patch_safety;
use crate::safety::SafetyCheck;
use codex_apply_patch::ApplyPatchAction;
use codex_apply_patch::ApplyPatchFileChange;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::models::ResponseInputItem;
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;

pub const CODEX_APPLY_PATCH_ARG1: &str = "--codex-run-as-apply-patch";

pub(crate) enum InternalApplyPatchInvocation {
    /// The `apply_patch` call was handled programmatically, without any sort
    /// of sandbox, because the user explicitly approved it. This is the
    /// result to use with the `shell` function call that contained `apply_patch`.
    Output(ResponseInputItem),

    /// The `apply_patch` call was approved, either automatically because it
    /// appears that it should be allowed based on the user's sandbox policy
    /// *or* because the user explicitly approved it. In either case, we use
    /// exec with [`CODEX_APPLY_PATCH_ARG1`] to realize the `apply_patch` call,
    /// but [`ApplyPatchExec::auto_approved`] is used to determine the sandbox
    /// used with the `exec()`.
    DelegateToExec(ApplyPatchExec),
}

pub(crate) struct ApplyPatchExec {
    pub(crate) action: ApplyPatchAction,
    pub(crate) user_explicitly_approved_this_action: bool,
    pub(crate) orchestrator_advice: Option<ApplyPatchOrchestratorAdvice>,
}

#[derive(Clone, Debug)]
pub(crate) struct ApplyPatchOrchestratorAdvice {
    pub(crate) approval_reason: String,
    pub(crate) background_followup: String,
}

impl From<ResponseInputItem> for InternalApplyPatchInvocation {
    fn from(item: ResponseInputItem) -> Self {
        InternalApplyPatchInvocation::Output(item)
    }
}

pub(crate) async fn apply_patch(
    sess: &Session,
    turn_context: &TurnContext,
    sub_id: &str,
    call_id: &str,
    action: ApplyPatchAction,
) -> InternalApplyPatchInvocation {
    match assess_patch_safety(
        &action,
        turn_context.approval_policy,
        &turn_context.sandbox_policy,
        &turn_context.cwd,
    ) {
        SafetyCheck::AutoApprove { .. } => {
            let orchestrator_advice = build_apply_patch_orchestrator_advice(&action);
            InternalApplyPatchInvocation::DelegateToExec(ApplyPatchExec {
                action,
                user_explicitly_approved_this_action: false,
                orchestrator_advice,
            })
        }
        SafetyCheck::AskUser => {
            // Compute a readable summary of path changes to include in the
            // approval request so the user can make an informed decision.
            //
            // Note that it might be worth expanding this approval request to
            // give the user the option to expand the set of writable roots so
            // that similar patches can be auto-approved in the future during
            // this session.
            let orchestrator_advice = build_apply_patch_orchestrator_advice(&action);
            let rx_approve = sess
                .request_patch_approval(
                    sub_id.to_owned(),
                    call_id.to_owned(),
                    &action,
                    orchestrator_advice
                        .as_ref()
                        .map(|advice| advice.approval_reason.clone()),
                    None,
                )
                .await;
            match rx_approve.await.unwrap_or_default() {
                ReviewDecision::Approved | ReviewDecision::ApprovedForSession => {
                    InternalApplyPatchInvocation::DelegateToExec(ApplyPatchExec {
                        action,
                        user_explicitly_approved_this_action: true,
                        orchestrator_advice,
                    })
                }
                ReviewDecision::Denied | ReviewDecision::Abort => {
                    ResponseInputItem::FunctionCallOutput {
                        call_id: call_id.to_owned(),
                        output: FunctionCallOutputPayload {
                            content: "patch rejected by user".to_string(),
                            success: Some(false),
                        },
                    }
                    .into()
                }
            }
        }
        SafetyCheck::Reject { reason } => ResponseInputItem::FunctionCallOutput {
            call_id: call_id.to_owned(),
            output: FunctionCallOutputPayload {
                content: format!("patch rejected: {reason}"),
                success: Some(false),
            },
        }
        .into(),
    }
}

pub(crate) fn convert_apply_patch_to_protocol(
    action: &ApplyPatchAction,
) -> HashMap<PathBuf, FileChange> {
    let changes = action.changes();
    let mut result = HashMap::with_capacity(changes.len());
    for (path, change) in changes {
        let protocol_change = match change {
            ApplyPatchFileChange::Add { content } => FileChange::Add {
                content: content.clone(),
            },
            ApplyPatchFileChange::Delete { content } => FileChange::Delete {
                content: content.clone(),
            },
            ApplyPatchFileChange::Update {
                unified_diff,
                move_path,
                new_content: _new_content,
                original_content: _original_content,
            } => FileChange::Update {
                unified_diff: unified_diff.clone(),
                move_path: move_path.clone(),
            },
        };
        result.insert(path.clone(), protocol_change);
    }
    result
}


const MAX_TITLE_LEN: usize = 48;
const MAX_LABEL_LEN: usize = 80;
const MAX_SUMMARY_LEN: usize = 160;
const SUMMARY_PREVIEW_COUNT: usize = 3;

fn build_apply_patch_orchestrator_advice(
    action: &ApplyPatchAction,
) -> Option<ApplyPatchOrchestratorAdvice> {
    if action.is_empty() {
        return None;
    }

    let mut descriptors: Vec<String> = action
        .changes()
        .iter()
        .filter_map(|(path, change)| describe_change(path, change, &action.cwd))
        .collect();

    if descriptors.is_empty() {
        return None;
    }

    descriptors.sort();
    descriptors.dedup();

    let summary = summarize_descriptors(&descriptors);
    let title_seed = descriptors
        .first()
        .cloned()
        .unwrap_or_else(|| "patch".to_string());
    let title = sanitize_label(&title_seed, MAX_TITLE_LEN, true);

    let investigate_cmd = format!(
        "codex orchestrator investigate --title \"{}\" --severity sev2 --persona operator",
        title
    );
    let feedback_cmd = "codex orchestrator feedback --persona operator".to_string();
    let triage_cmd =
        "codex orchestrator triage --persona operator --review-hours 5.0".to_string();

    let approval_reason = format!(
        "Orchestrator follow-up required for: {}. Run {} and then {}.",
        summary, investigate_cmd, feedback_cmd
    );
    let background_followup = format!(
        "Orchestrator follow-up:
- Investigate → {}
- Feedback → {}
- Triage → {}",
        investigate_cmd, feedback_cmd, triage_cmd
    );

    Some(ApplyPatchOrchestratorAdvice {
        approval_reason,
        background_followup,
    })
}

fn describe_change(
    path: &Path,
    change: &ApplyPatchFileChange,
    cwd: &Path,
) -> Option<String> {
    let from_label = normalize_path_label(path, cwd);
    let description = match change {
        ApplyPatchFileChange::Add { .. } => format!("+{}", from_label),
        ApplyPatchFileChange::Delete { .. } => format!("-{}", from_label),
        ApplyPatchFileChange::Update { move_path: Some(dest), .. } => {
            let dest_label = normalize_path_label(dest, cwd);
            if dest_label == from_label {
                from_label
            } else {
                format!("{} -> {}", from_label, dest_label)
            }
        }
        ApplyPatchFileChange::Update { .. } => from_label,
    };

    let trimmed = description.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn normalize_path_label(path: &Path, cwd: &Path) -> String {
    let absolute = absolutize(path, cwd);
    let relative_candidate = absolute
        .strip_prefix(cwd)
        .map(|rel| rel.to_path_buf())
        .unwrap_or_else(|_| {
            absolute
                .file_name()
                .map(PathBuf::from)
                .unwrap_or_else(|| absolute.clone())
        });

    let display = relative_candidate.to_string_lossy().replace('\\', "/");
    let trimmed = display.trim_matches('/').trim();

    sanitize_label(trimmed, MAX_LABEL_LEN, true)
}

fn absolutize(path: &Path, cwd: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

fn sanitize_label(input: &str, max_len: usize, allow_space: bool) -> String {
    let mut sanitized = String::with_capacity(max_len.min(input.len()));
    let mut last_was_space = false;

    for ch in input.chars() {
        if sanitized.len() >= max_len {
            break;
        }
        match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '/' | '.' | '_' | '-' => {
                sanitized.push(ch);
                last_was_space = false;
            }
            ' ' if allow_space => {
                if !sanitized.is_empty() && !last_was_space {
                    sanitized.push(' ');
                    last_was_space = true;
                }
            }
            ch if ch.is_whitespace() => {
                sanitized.push('_');
                last_was_space = false;
            }
            _ => {
                sanitized.push('_');
                last_was_space = false;
            }
        }
    }

    let trimmed = sanitized.trim();
    if trimmed.is_empty() {
        "<unknown>".to_string()
    } else {
        trimmed.to_string()
    }
}

fn summarize_descriptors(labels: &[String]) -> String {
    let mut summary = labels
        .iter()
        .take(SUMMARY_PREVIEW_COUNT)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");

    if labels.len() > SUMMARY_PREVIEW_COUNT {
        if !summary.is_empty() {
            summary.push(' ');
        }
        summary.push_str(&format!(
            "(+{} more)",
            labels.len() - SUMMARY_PREVIEW_COUNT
        ));
    }

    if summary.len() > MAX_SUMMARY_LEN {
        summary.truncate(MAX_SUMMARY_LEN - 3);
        summary.push_str("...");
    }

    if summary.is_empty() {
        "<unknown change>".to_string()
    } else {
        summary
    }
}
