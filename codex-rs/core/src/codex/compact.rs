use std::fmt::Write as FmtWrite;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use super::AgentTask;
use super::MutexExt;
use super::Session;
use super::TurnContext;
use super::get_last_assistant_message_from_turn;
use crate::Prompt;
use crate::client_common::ResponseEvent;
use crate::error::CodexErr;
use crate::error::Result as CodexResult;
use crate::git_info::resolve_root_git_project_for_trust;
use crate::openai_tools::ConfigShellToolType;
use crate::plan_tool::StepStatus;
use crate::plan_tool::UpdatePlanArgs;
use crate::protocol::AgentMessageEvent;
use crate::protocol::CompactedItem;
use crate::protocol::ErrorEvent;
use crate::protocol::Event;
use crate::protocol::EventMsg;
use crate::protocol::InputItem;
use crate::protocol::InputMessageKind;
use crate::protocol::TaskCompleteEvent;
use crate::protocol::TaskStartedEvent;
use crate::protocol::TurnContextItem;
use crate::util::backoff;
use askama::Template;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseInputItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::RolloutItem;
use futures::prelude::*;

pub(super) const COMPACT_TRIGGER_TEXT: &str = "Start Summarization";
const SUMMARIZATION_PROMPT: &str = include_str!("../../templates/compact/prompt.md");

#[derive(Template)]
#[template(path = "compact/history_bridge.md", escape = "none")]
struct HistoryBridgeTemplate<'a> {
    user_messages_text: &'a str,
    summary_text: &'a str,
    user_instructions_text: Option<String>,
    environment_context_text: Option<String>,
    plan_text: Option<String>,
    repo_outline_text: Option<String>,
    session_context_text: Option<String>,
}

pub(crate) struct HistoryBridgeContext {
    pub user_instructions_text: Option<String>,
    pub environment_context_text: Option<String>,
    pub plan_text: Option<String>,
    pub repo_outline_text: Option<String>,
    pub session_context_text: Option<String>,
}

const MAX_USER_MESSAGE_FRAGMENT: usize = 280;
const MAX_USER_MESSAGE_SAMPLES: usize = 5;
const FALLBACK_COMPRESSION_LIMIT: usize = 320;

pub(super) fn spawn_compact_task(
    sess: Arc<Session>,
    turn_context: Arc<TurnContext>,
    sub_id: String,
    input: Vec<InputItem>,
) {
    let task = AgentTask::compact(
        sess.clone(),
        turn_context,
        sub_id,
        input,
        SUMMARIZATION_PROMPT.to_string(),
    );
    sess.set_task(task);
}

pub(super) async fn run_inline_auto_compact_task(
    sess: Arc<Session>,
    turn_context: Arc<TurnContext>,
) {
    let sub_id = sess.next_internal_sub_id();
    let input = vec![InputItem::Text {
        text: COMPACT_TRIGGER_TEXT.to_string(),
    }];
    run_compact_task_inner(
        sess,
        turn_context,
        sub_id,
        input,
        SUMMARIZATION_PROMPT.to_string(),
        false,
    )
    .await;
}

pub(super) async fn run_compact_task(
    sess: Arc<Session>,
    turn_context: Arc<TurnContext>,
    sub_id: String,
    input: Vec<InputItem>,
    compact_instructions: String,
) {
    run_compact_task_inner(
        sess,
        turn_context,
        sub_id,
        input,
        compact_instructions,
        true,
    )
    .await;
}

async fn run_compact_task_inner(
    sess: Arc<Session>,
    turn_context: Arc<TurnContext>,
    sub_id: String,
    input: Vec<InputItem>,
    compact_instructions: String,
    remove_task_on_completion: bool,
) {
    let model_context_window = turn_context.client.get_model_context_window();
    let start_event = Event {
        id: sub_id.clone(),
        msg: EventMsg::TaskStarted(TaskStartedEvent {
            model_context_window,
        }),
    };
    sess.send_event(start_event).await;

    let initial_input_for_turn: ResponseInputItem = ResponseInputItem::from(input);
    let instructions_override = compact_instructions;
    let turn_input = sess.turn_input_with_history(vec![initial_input_for_turn.clone().into()]);

    let prompt = Prompt {
        input: turn_input,
        tools: Vec::new(),
        base_instructions_override: Some(instructions_override),
    };

    let max_retries = turn_context.client.get_provider().stream_max_retries();
    let mut retries = 0;

    let rollout_item = RolloutItem::TurnContext(TurnContextItem {
        cwd: turn_context.cwd.clone(),
        approval_policy: turn_context.approval_policy,
        sandbox_policy: turn_context.sandbox_policy.clone(),
        model: turn_context.client.get_model(),
        effort: turn_context.client.get_reasoning_effort(),
        summary: turn_context.client.get_reasoning_summary(),
    });
    sess.persist_rollout_items(&[rollout_item]).await;

    loop {
        let attempt_result = drain_to_completed(&sess, turn_context.as_ref(), &prompt).await;

        match attempt_result {
            Ok(()) => {
                break;
            }
            Err(CodexErr::Interrupted) => {
                return;
            }
            Err(e) => {
                if retries < max_retries {
                    retries += 1;
                    let delay = backoff(retries);
                    sess.notify_stream_error(
                        &sub_id,
                        format!(
                            "stream error: {e}; retrying {retries}/{max_retries} in {delay:?}…"
                        ),
                    )
                    .await;
                    tokio::time::sleep(delay).await;
                    continue;
                } else {
                    let event = Event {
                        id: sub_id.clone(),
                        msg: EventMsg::Error(ErrorEvent {
                            message: e.to_string(),
                        }),
                    };
                    sess.send_event(event).await;
                    return;
                }
            }
        }
    }

    if remove_task_on_completion {
        sess.remove_task(&sub_id);
    }
    let history_snapshot = {
        let state = sess.state.lock_unchecked();
        state.history.contents()
    };
    let summary_text = get_last_assistant_message_from_turn(&history_snapshot).unwrap_or_default();
    let user_messages = collect_user_messages(&history_snapshot);
    let initial_context = sess.build_initial_context(turn_context.as_ref());
    let initial_sections = extract_initial_context_sections(&initial_context);
    let plan_snapshot = sess.plan_snapshot();
    let plan_text = plan_snapshot.as_ref().map(format_plan_overview);
    let repo_outline = build_repo_outline(&turn_context.cwd);
    let session_context_text = build_session_snapshot(turn_context.as_ref());
    let session_context_ref =
        (!session_context_text.trim().is_empty()).then_some(session_context_text.as_str());
    let bridge_context = HistoryBridgeContext {
        user_instructions_text: initial_sections
            .user_instructions
            .as_deref()
            .map(compress_user_instructions),
        environment_context_text: initial_sections
            .environment_context
            .as_deref()
            .map(compress_environment_context),
        plan_text: plan_text.as_deref().map(compress_plan_snapshot),
        repo_outline_text: repo_outline.clone(),
        session_context_text: session_context_ref.map(|value| compress_session_snapshot(value)),
    };
    let new_history = build_compacted_history(
        initial_context,
        &user_messages,
        &summary_text,
        bridge_context,
    );
    {
        let mut state = sess.state.lock_unchecked();
        state.history.replace(new_history);
    }

    let rollout_item = RolloutItem::Compacted(CompactedItem {
        message: summary_text.clone(),
    });
    sess.persist_rollout_items(&[rollout_item]).await;

    let event = Event {
        id: sub_id.clone(),
        msg: EventMsg::AgentMessage(AgentMessageEvent {
            message: "Compact task completed".to_string(),
        }),
    };
    sess.send_event(event).await;
    let event = Event {
        id: sub_id.clone(),
        msg: EventMsg::TaskComplete(TaskCompleteEvent {
            last_agent_message: None,
        }),
    };
    sess.send_event(event).await;
}

fn content_items_to_text(content: &[ContentItem]) -> Option<String> {
    let mut pieces = Vec::new();
    for item in content {
        match item {
            ContentItem::InputText { text } | ContentItem::OutputText { text } => {
                if !text.is_empty() {
                    pieces.push(text.as_str());
                }
            }
            ContentItem::InputImage { .. } => {}
        }
    }
    if pieces.is_empty() {
        None
    } else {
        Some(pieces.join("\n"))
    }
}

pub(crate) fn collect_user_messages(items: &[ResponseItem]) -> Vec<String> {
    items
        .iter()
        .filter_map(|item| match item {
            ResponseItem::Message { role, content, .. } if role == "user" => {
                content_items_to_text(content)
            }
            _ => None,
        })
        .filter(|text| !is_session_prefix_message(text))
        .collect()
}

fn is_session_prefix_message(text: &str) -> bool {
    matches!(
        InputMessageKind::from(("user", text)),
        InputMessageKind::UserInstructions | InputMessageKind::EnvironmentContext
    )
}

pub(crate) fn sanitize_inline(value: &str) -> String {
    value
        .split_whitespace()
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn truncate_for_bridge(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_string();
    }
    if limit <= 3 {
        return ".".repeat(limit);
    }
    let cutoff = limit - 3;
    let mut truncated = String::with_capacity(limit);
    for (idx, ch) in value.chars().enumerate() {
        if idx >= cutoff {
            break;
        }
        truncated.push(ch);
    }
    truncated.push_str("...");
    truncated
}

fn format_user_messages_for_bridge(user_messages: &[String]) -> String {
    if user_messages.is_empty() {
        return "(none)".to_string();
    }
    let total = user_messages.len();
    let mut pieces: Vec<String> = user_messages
        .iter()
        .enumerate()
        .map(|(idx, message)| {
            let normalized = sanitize_inline(message);
            let fragment = if normalized.is_empty() {
                "(empty)".to_string()
            } else {
                truncate_for_bridge(&normalized, MAX_USER_MESSAGE_FRAGMENT)
            };
            format!("TURN{}:{}", idx + 1, fragment)
        })
        .take(MAX_USER_MESSAGE_SAMPLES)
        .collect();
    if total > MAX_USER_MESSAGE_SAMPLES {
        pieces.push(format!("+{} more", total - MAX_USER_MESSAGE_SAMPLES));
    }
    pieces.join(";")
}

pub(crate) struct InitialContextSections {
    pub user_instructions: Option<String>,
    pub environment_context: Option<String>,
}

pub(crate) fn extract_initial_context_sections(items: &[ResponseItem]) -> InitialContextSections {
    let mut instructions = Vec::new();
    let mut environment_context = None;

    for item in items {
        let ResponseItem::Message { role, content, .. } = item else {
            continue;
        };
        if role != "user" {
            continue;
        }
        let Some(text) = content_items_to_text(content) else {
            continue;
        };
        match InputMessageKind::from((role.as_str(), text.as_str())) {
            InputMessageKind::UserInstructions => instructions.push(text),
            InputMessageKind::EnvironmentContext => environment_context = Some(text),
            _ => {}
        }
    }

    let user_instructions = if instructions.is_empty() {
        None
    } else {
        Some(instructions.join("\n\n"))
    };

    InitialContextSections {
        user_instructions,
        environment_context,
    }
}

pub(crate) fn format_plan_overview(plan: &UpdatePlanArgs) -> String {
    let mut lines = Vec::new();
    if let Some(explanation) = plan.explanation.as_ref() {
        let trimmed = explanation.trim();
        if !trimmed.is_empty() {
            lines.push(format!("explanation={trimmed}"));
        }
    }

    if plan.plan.is_empty() {
        lines.push("C0 plan-empty".to_string());
        return lines.join(";");
    }

    for item in &plan.plan {
        let step = item.step.trim();
        if step.is_empty() {
            continue;
        }
        let code = match item.status {
            StepStatus::Pending => "P",
            StepStatus::InProgress => "I",
            StepStatus::Completed => "C",
        };
        lines.push(format!("{code} {step}"));
    }

    lines.join(";")
}

fn fallback_compact(value: &str) -> String {
    let count = value.chars().count();
    if count <= FALLBACK_COMPRESSION_LIMIT {
        return value.to_string();
    }
    let keep = FALLBACK_COMPRESSION_LIMIT.saturating_sub(3);
    let mut truncated = value.chars().take(keep).collect::<String>();
    truncated.push_str("...");
    truncated
}

fn compress_user_instructions(value: &str) -> String {
    let sanitized = sanitize_inline(value);
    if sanitized.contains("Stellar TUI Agent Playbook") {
        return "Playbook: todo.md ведёт приоритеты; Workstreams: завершать M0→M6 последовательно; Quality: DoD+метрики; Rhythm: Align→Build→Validate→Close; Execution: just fmt после Rust, не трогать sandbox env vars; Tests: cargo test per crate, --all-features при изменении core/common/protocol; TUI: styles.md + Stylize; Snapshots: cargo insta; Cognitive: Plan→Build→Validate".to_string();
    }
    fallback_compact(&sanitized)
}

fn extract_xml_tag(value: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = value.find(&open)? + open.len();
    let rest = &value[start..];
    let end_rel = rest.find(&close)?;
    Some(rest[..end_rel].trim().to_string())
}

fn compress_environment_context(value: &str) -> String {
    let cwd = extract_xml_tag(value, "cwd").unwrap_or_default();
    let approval = extract_xml_tag(value, "approval_policy").unwrap_or_default();
    let sandbox = extract_xml_tag(value, "sandbox_mode").unwrap_or_default();
    let network = extract_xml_tag(value, "network_access").unwrap_or_default();
    let shell = extract_xml_tag(value, "shell").unwrap_or_default();
    let extras = [
        extract_xml_tag(value, "workspace"),
        extract_xml_tag(value, "sandbox_allow"),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    let mut parts = vec![
        format!("cwd={cwd}"),
        format!("approval={approval}"),
        format!("sandbox={sandbox}"),
        format!("network={network}"),
        format!("shell={shell}"),
    ];
    if !extras.is_empty() {
        parts.push(format!("extra={}", extras.join(",")));
    }
    parts.join(";")
}

fn compress_plan_snapshot(value: &str) -> String {
    let sanitized = sanitize_inline(value);
    sanitized.replace('|', ",")
}

fn compress_session_snapshot(value: &str) -> String {
    let sanitized = sanitize_inline(value);
    sanitized
        .split_whitespace()
        .map(|segment| segment.replace('|', ","))
        .collect::<Vec<_>>()
        .join(";")
}

const MAX_REPO_LIST_ENTRIES: usize = 5;
const MAX_GIT_STATUS_SAMPLES: usize = 4;
const REPO_IGNORED_DIRS: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    ".idea",
    ".vscode",
    "__pycache__",
];
const REPO_IGNORED_FILES: &[&str] = &[".DS_Store", "Thumbs.db"];
const REPO_KEY_MANIFESTS: &[&str] = &[
    "Cargo.toml",
    "package.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "requirements.txt",
    "pyproject.toml",
    "go.mod",
    "Gemfile",
    "Makefile",
];

fn display_list(items: &[String], limit: usize) -> (String, usize) {
    if items.is_empty() {
        return (String::new(), 0);
    }
    let shown: Vec<&String> = items.iter().take(limit).collect();
    let overflow = items.len().saturating_sub(shown.len());
    let mut rendered = shown
        .iter()
        .map(|s| s.as_str())
        .collect::<Vec<_>>()
        .join(",");
    if overflow > 0 {
        let _ = write!(rendered, ",+{overflow}");
    }
    (rendered, items.len())
}

fn resolve_git_dir(repo_root: &Path) -> Option<PathBuf> {
    let dot_git = repo_root.join(".git");
    let metadata = fs::metadata(&dot_git).ok()?;
    if metadata.is_dir() {
        return Some(dot_git);
    }
    if metadata.is_file() {
        let raw = fs::read_to_string(&dot_git).ok()?;
        let path = raw.strip_prefix("gitdir:")?.trim();
        if path.is_empty() {
            return None;
        }
        let candidate = Path::new(path);
        if candidate.is_absolute() {
            Some(candidate.to_path_buf())
        } else {
            Some(repo_root.join(candidate))
        }
    } else {
        None
    }
}

fn read_head_reference(git_dir: &Path) -> (Option<String>, Option<String>) {
    let head_path = git_dir.join("HEAD");
    let Ok(raw_head) = fs::read_to_string(&head_path) else {
        return (None, None);
    };
    let trimmed = raw_head.trim();
    if let Some(reference) = trimmed.strip_prefix("ref:") {
        let reference = reference.trim();
        let branch = reference
            .rsplit('/')
            .next()
            .map(|value| sanitize_inline(value));
        let target_path = git_dir.join(reference);
        let commit = fs::read_to_string(&target_path)
            .ok()
            .map(|content| content.trim().to_string());
        (branch, commit)
    } else if !trimmed.is_empty() {
        (None, Some(trimmed.to_string()))
    } else {
        (None, None)
    }
}

struct GitStatusSummary {
    total: usize,
    staged: usize,
    unstaged: usize,
    untracked: usize,
    samples: Vec<String>,
}

fn collect_git_status_summary(repo_root: &Path) -> Option<GitStatusSummary> {
    let output = Command::new("git")
        .args(["status", "--porcelain=v1"])
        .current_dir(repo_root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    if stdout.trim().is_empty() {
        return Some(GitStatusSummary {
            total: 0,
            staged: 0,
            unstaged: 0,
            untracked: 0,
            samples: Vec::new(),
        });
    }
    let mut summary = GitStatusSummary {
        total: 0,
        staged: 0,
        unstaged: 0,
        untracked: 0,
        samples: Vec::new(),
    };
    for line in stdout.lines() {
        if line.len() < 3 {
            continue;
        }
        summary.total += 1;
        let status_bytes = line.as_bytes();
        let staged_flag = status_bytes[0] as char;
        let unstaged_flag = status_bytes[1] as char;
        if staged_flag == '?' && unstaged_flag == '?' {
            summary.untracked += 1;
        } else {
            if staged_flag != ' ' {
                summary.staged += 1;
            }
            if unstaged_flag != ' ' {
                summary.unstaged += 1;
            }
        }
        let mut path_fragment = &line[3..];
        if let Some(idx) = path_fragment.find(" -> ") {
            path_fragment = &path_fragment[idx + 4..];
        }
        let normalized = sanitize_inline(path_fragment);
        if !normalized.is_empty() && summary.samples.len() < MAX_GIT_STATUS_SAMPLES {
            summary.samples.push(normalized);
        }
    }
    Some(summary)
}

fn build_git_outline(repo_root: &Path) -> Option<String> {
    let git_dir = resolve_git_dir(repo_root)?;
    let (branch, commit) = read_head_reference(&git_dir);
    let status_summary = collect_git_status_summary(repo_root);

    let mut segments = Vec::new();
    if let Some(branch) = branch {
        segments.push(format!("branch={branch}"));
    }
    if let Some(commit) = commit {
        let short: String = commit.chars().take(12).collect();
        if !short.is_empty() {
            segments.push(format!("sha={short}"));
        }
    }
    match status_summary {
        Some(summary) => {
            if summary.total == 0 {
                segments.push("status=clean".to_string());
            } else {
                segments.push("status=dirty".to_string());
                segments.push(format!("staged={}", summary.staged));
                segments.push(format!("unstaged={}", summary.unstaged));
                segments.push(format!("untracked={}", summary.untracked));
                if summary.total > 0 {
                    let (rendered, total) = display_list(&summary.samples, MAX_GIT_STATUS_SAMPLES);
                    if total > 0 {
                        segments.push(format!("files[{total}]={rendered}"));
                    }
                }
            }
        }
        None => segments.push("status=unknown".to_string()),
    }

    if segments.is_empty() {
        return None;
    }
    Some(format!("git={}", segments.join("|")))
}

pub(crate) fn build_repo_outline(cwd: &Path) -> Option<String> {
    let repo_root = resolve_root_git_project_for_trust(cwd).unwrap_or_else(|| cwd.to_path_buf());
    let entries = fs::read_dir(&repo_root).ok()?;
    let mut dirs = Vec::new();
    let mut files = Vec::new();

    for entry_result in entries {
        let entry = entry_result.ok()?;
        let file_type = entry.file_type().ok()?;
        let name = entry.file_name();
        let name = name.to_string_lossy().to_string();
        if file_type.is_dir() {
            if REPO_IGNORED_DIRS.contains(&name.as_str()) {
                continue;
            }
            dirs.push(name);
        } else if file_type.is_file() {
            if REPO_IGNORED_FILES.contains(&name.as_str()) {
                continue;
            }
            files.push(name);
        }
    }

    dirs.sort_unstable();
    files.sort_unstable();

    let key_manifests: Vec<String> = files
        .into_iter()
        .filter(|name| REPO_KEY_MANIFESTS.contains(&name.as_str()))
        .collect();

    let mut lines = Vec::new();
    lines.push(format!("root={}", repo_root.display()));

    if !dirs.is_empty() {
        let (rendered, total) = display_list(&dirs, MAX_REPO_LIST_ENTRIES);
        lines.push(format!("dirs={total}:{rendered}"));
    }

    if !key_manifests.is_empty() {
        let (rendered, total) = display_list(&key_manifests, MAX_REPO_LIST_ENTRIES);
        lines.push(format!("manifests={total}:{rendered}"));
    }

    if let Some(git_snapshot) = build_git_outline(&repo_root) {
        lines.push(git_snapshot);
    }

    Some(lines.join(";"))
}

pub(crate) fn build_session_snapshot(turn_context: &TurnContext) -> String {
    let mut sections = Vec::new();

    let mut session = Vec::new();
    session.push(format!("cwd={}", turn_context.cwd.display()));
    session.push(format!("approval={}", turn_context.approval_policy));
    session.push(format!("sandbox={}", turn_context.sandbox_policy));
    let network = match &turn_context.sandbox_policy {
        crate::protocol::SandboxPolicy::WorkspaceWrite { network_access, .. } => {
            if *network_access {
                "enabled"
            } else {
                "restricted"
            }
        }
        crate::protocol::SandboxPolicy::DangerFullAccess => "enabled",
        crate::protocol::SandboxPolicy::ReadOnly => "restricted",
    };
    session.push(format!("network={network}"));
    sections.push(session.join("|"));

    if let crate::protocol::SandboxPolicy::WorkspaceWrite {
        writable_roots,
        exclude_tmpdir_env_var,
        exclude_slash_tmp,
        ..
    } = &turn_context.sandbox_policy
    {
        if !writable_roots.is_empty() {
            let rendered: Vec<String> = writable_roots
                .iter()
                .map(|p| p.display().to_string())
                .collect();
            sections.push(format!("writable=[{}]", rendered.join(",")));
        }
        if *exclude_tmpdir_env_var {
            sections.push("exclude_tmpdir_env_var=true".to_string());
        }
        if *exclude_slash_tmp {
            sections.push("exclude_/tmp=true".to_string());
        }
    }

    let provider = turn_context.client.get_provider();
    let mut model = Vec::new();
    model.push(format!("provider={}", provider.name));
    model.push(format!("model={}", turn_context.client.get_model()));
    if let Some(window) = turn_context.client.get_model_context_window() {
        model.push(format!("ctx_window={window}"));
    }
    if let Some(limit) = turn_context.client.get_auto_compact_token_limit() {
        model.push(format!("auto_compact_limit={limit}"));
    }
    if let Some(effort) = turn_context.client.get_reasoning_effort() {
        model.push(format!("effort={effort}"));
    }
    model.push(format!(
        "summary={}",
        turn_context.client.get_reasoning_summary()
    ));
    sections.push(model.join("|"));

    let shell_tool = match &turn_context.tools_config.shell_type {
        ConfigShellToolType::DefaultShell => "default",
        ConfigShellToolType::ShellWithRequest { .. } => "approval",
        ConfigShellToolType::LocalShell => "local",
        ConfigShellToolType::StreamableShell => "stream",
    };
    let mut tool = Vec::new();
    tool.push(format!("shell={shell_tool}"));
    tool.push(format!(
        "plan={}",
        if turn_context.tools_config.plan_tool {
            "on"
        } else {
            "off"
        }
    ));
    let apply_patch_tool = match turn_context.tools_config.apply_patch_tool_type {
        Some(crate::tool_apply_patch::ApplyPatchToolType::Freeform) => "freeform",
        Some(crate::tool_apply_patch::ApplyPatchToolType::Function) => "function",
        None => "off",
    };
    tool.push(format!("apply_patch={apply_patch_tool}"));
    tool.push(format!(
        "web_search={}",
        if turn_context.tools_config.web_search_request {
            "on"
        } else {
            "off"
        }
    ));
    tool.push(format!(
        "view_image={}",
        if turn_context.tools_config.include_view_image_tool {
            "on"
        } else {
            "off"
        }
    ));
    tool.push(format!(
        "unified_exec={}",
        if turn_context.tools_config.experimental_unified_exec_tool {
            "on"
        } else {
            "off"
        }
    ));
    sections.push(tool.join("|"));

    let env_policy = &turn_context.shell_environment_policy;
    let mut env = Vec::new();
    env.push(format!("inherit={:?}", env_policy.inherit));
    env.push(format!("use_profile={}", env_policy.use_profile));
    if env_policy.ignore_default_excludes {
        env.push("ignore_default_excludes=true".to_string());
    }
    if !env_policy.exclude.is_empty() {
        env.push(format!("exclude_count={}", env_policy.exclude.len()));
    }
    if !env_policy.include_only.is_empty() {
        env.push(format!(
            "include_only_count={}",
            env_policy.include_only.len()
        ));
    }
    if !env_policy.r#set.is_empty() {
        let mut keys: Vec<&String> = env_policy.r#set.keys().collect();
        keys.sort();
        let rendered: Vec<String> = keys.into_iter().map(|k| k.clone()).collect();
        env.push(format!("set_keys={}", rendered.join(",")));
    }
    sections.push(env.join("|"));

    sections.push(format!("review_mode={}", turn_context.is_review_mode));

    sections.join(" ")
}

pub(crate) fn build_compacted_history(
    initial_context: Vec<ResponseItem>,
    user_messages: &[String],
    summary_text: &str,
    bridge_context: HistoryBridgeContext,
) -> Vec<ResponseItem> {
    let mut history = initial_context;
    let user_messages_text = format_user_messages_for_bridge(user_messages);
    let summary_text = if summary_text.is_empty() {
        "(no summary available)".to_string()
    } else {
        summary_text.to_string()
    };
    let summary_compact = sanitize_inline(&summary_text);
    let HistoryBridgeContext {
        user_instructions_text,
        environment_context_text,
        plan_text,
        repo_outline_text,
        session_context_text,
    } = bridge_context;
    let Ok(bridge) = HistoryBridgeTemplate {
        user_messages_text: &user_messages_text,
        summary_text: &summary_compact,
        user_instructions_text,
        environment_context_text,
        plan_text,
        repo_outline_text,
        session_context_text,
    }
    .render() else {
        return vec![];
    };
    history.push(ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText { text: bridge }],
    });
    history
}

async fn drain_to_completed(
    sess: &Session,
    turn_context: &TurnContext,
    prompt: &Prompt,
) -> CodexResult<()> {
    let mut stream = turn_context.client.clone().stream(prompt).await?;
    loop {
        let maybe_event = stream.next().await;
        let Some(event) = maybe_event else {
            return Err(CodexErr::Stream(
                "stream closed before response.completed".into(),
                None,
            ));
        };
        match event {
            Ok(ResponseEvent::OutputItemDone(item)) => {
                let mut state = sess.state.lock_unchecked();
                state.history.record_items(std::slice::from_ref(&item));
            }
            Ok(ResponseEvent::Completed { .. }) => {
                return Ok(());
            }
            Ok(_) => continue,
            Err(e) => return Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_protocol::plan_tool::PlanItemArg;
    use codex_protocol::plan_tool::StepStatus;
    use codex_protocol::plan_tool::UpdatePlanArgs;
    use codex_protocol::protocol::ENVIRONMENT_CONTEXT_CLOSE_TAG;
    use codex_protocol::protocol::ENVIRONMENT_CONTEXT_OPEN_TAG;
    use codex_protocol::protocol::USER_INSTRUCTIONS_CLOSE_TAG;
    use codex_protocol::protocol::USER_INSTRUCTIONS_OPEN_TAG;
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    #[test]
    fn extract_initial_context_sections_captures_instructions_and_environment() {
        let instructions =
            format!("{USER_INSTRUCTIONS_OPEN_TAG}\n<!--test-->\n{USER_INSTRUCTIONS_CLOSE_TAG}");
        let environment = format!(
            "{ENVIRONMENT_CONTEXT_OPEN_TAG}\n  <cwd>/repo</cwd>\n{ENVIRONMENT_CONTEXT_CLOSE_TAG}"
        );
        let items = vec![
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: instructions.clone(),
                }],
            },
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: environment.clone(),
                }],
            },
        ];

        let captured = extract_initial_context_sections(&items);

        assert_eq!(
            captured.user_instructions.as_deref(),
            Some(instructions.as_str())
        );
        assert_eq!(
            captured.environment_context.as_deref(),
            Some(environment.as_str())
        );
    }

    #[test]
    fn format_plan_overview_renders_statuses() {
        let plan = UpdatePlanArgs {
            explanation: Some("Focus on core requirements".to_string()),
            plan: vec![
                PlanItemArg {
                    step: "Collect requirements".to_string(),
                    status: StepStatus::Completed,
                },
                PlanItemArg {
                    step: "Design architecture".to_string(),
                    status: StepStatus::InProgress,
                },
                PlanItemArg {
                    step: "Implement".to_string(),
                    status: StepStatus::Pending,
                },
            ],
        };

        let rendered = format_plan_overview(&plan);
        let expected = [
            "explanation=Focus on core requirements",
            "C Collect requirements",
            "I Design architecture",
            "P Implement",
        ]
        .join(";");
        assert_eq!(rendered, expected);
    }

    #[test]
    fn build_repo_outline_lists_visible_entries() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path();
        std::fs::create_dir(root.join("codex-rs")).unwrap();
        std::fs::create_dir(root.join("docs")).unwrap();
        std::fs::create_dir(root.join("target")).unwrap(); // ignored
        std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"demo\"\n").unwrap();
        std::fs::write(root.join("README.md"), "# demo\n").unwrap();

        let outline = build_repo_outline(root).expect("outline");
        assert!(outline.contains("root="));
        assert!(outline.contains("dirs=2:codex-rs,docs"));
        assert!(outline.contains("manifests=1:Cargo.toml"));
        assert!(!outline.contains("target"));
    }

    #[test]
    fn content_items_to_text_joins_non_empty_segments() {
        let items = vec![
            ContentItem::InputText {
                text: "hello".to_string(),
            },
            ContentItem::OutputText {
                text: String::new(),
            },
            ContentItem::OutputText {
                text: "world".to_string(),
            },
        ];

        let joined = content_items_to_text(&items);

        assert_eq!(Some("hello\nworld".to_string()), joined);
    }

    #[test]
    fn content_items_to_text_ignores_image_only_content() {
        let items = vec![ContentItem::InputImage {
            image_url: "file://image.png".to_string(),
        }];

        let joined = content_items_to_text(&items);

        assert_eq!(None, joined);
    }

    #[test]
    fn collect_user_messages_extracts_user_text_only() {
        let items = vec![
            ResponseItem::Message {
                id: Some("assistant".to_string()),
                role: "assistant".to_string(),
                content: vec![ContentItem::OutputText {
                    text: "ignored".to_string(),
                }],
            },
            ResponseItem::Message {
                id: Some("user".to_string()),
                role: "user".to_string(),
                content: vec![
                    ContentItem::InputText {
                        text: "first".to_string(),
                    },
                    ContentItem::OutputText {
                        text: "second".to_string(),
                    },
                ],
            },
            ResponseItem::Other,
        ];

        let collected = collect_user_messages(&items);

        assert_eq!(vec!["first\nsecond".to_string()], collected);
    }

    #[test]
    fn collect_user_messages_filters_session_prefix_entries() {
        let items = vec![
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "<user_instructions>do things</user_instructions>".to_string(),
                }],
            },
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "<ENVIRONMENT_CONTEXT>cwd=/tmp</ENVIRONMENT_CONTEXT>".to_string(),
                }],
            },
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "real user message".to_string(),
                }],
            },
        ];

        let collected = collect_user_messages(&items);

        assert_eq!(vec!["real user message".to_string()], collected);
    }

    #[test]
    fn format_user_messages_for_bridge_is_compact_and_ordered() {
        let mut messages = vec![
            " first info chunk ".to_string(),
            String::new(),
            "a".repeat(MAX_USER_MESSAGE_FRAGMENT + 16),
        ];
        for idx in 0..=MAX_USER_MESSAGE_SAMPLES {
            messages.push(format!("extra message {idx}"));
        }

        let rendered = format_user_messages_for_bridge(&messages);

        assert!(rendered.contains("TURN1:first info chunk"));
        assert!(rendered.contains("TURN2:(empty)"));
        assert!(rendered.contains("TURN3:"));
        assert!(rendered.contains("...") || messages[2].len() <= MAX_USER_MESSAGE_FRAGMENT);
        let overflow_indicator = format!(
            "+{} more",
            messages.len().saturating_sub(MAX_USER_MESSAGE_SAMPLES)
        );
        assert!(rendered.contains(&overflow_indicator));
    }
}
