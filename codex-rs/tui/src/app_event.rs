use std::path::PathBuf;

use codex_common::model_presets::ModelPreset;
use codex_core::protocol::ConversationPathResponseEvent;
use codex_core::protocol::Event;
use codex_file_search::FileMatch;

use crate::bottom_pane::ApprovalRequest;
use crate::history_cell::HistoryCell;
use crate::mcp::McpWizardDraft;

use codex_core::UnifiedExecOutputWindow;
use codex_core::protocol::AskForApproval;
use codex_core::protocol::SandboxPolicy;
use codex_core::protocol::UnifiedExecSessionState;
use codex_core::protocol_config_types::ReasoningEffort;

#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub(crate) enum AppEvent {
    CodexEvent(Event),

    /// Start a new session.
    NewSession,

    /// Request to exit the application gracefully.
    ExitRequest,

    /// Forward an `Op` to the Agent. Using an `AppEvent` for this avoids
    /// bubbling channels through layers of widgets.
    CodexOp(codex_core::protocol::Op),

    /// Kick off an asynchronous file search for the given query (text after
    /// the `@`). Previous searches may be cancelled by the app layer so there
    /// is at most one in-flight search.
    StartFileSearch(String),

    /// Result of a completed asynchronous file search. The `query` echoes the
    /// original search term so the UI can decide whether the results are
    /// still relevant.
    FileSearchResult {
        query: String,
        matches: Vec<FileMatch>,
    },

    /// Result of computing a `/diff` command.
    DiffResult(String),

    InsertHistoryCell(Box<dyn HistoryCell>),

    StartCommitAnimation,
    StopCommitAnimation,
    CommitTick,

    /// Update the current reasoning effort in the running app and widget.
    UpdateReasoningEffort(Option<ReasoningEffort>),

    /// Update the current model slug in the running app and widget.
    UpdateModel(String),

    /// Persist the selected model and reasoning effort to the appropriate config.
    PersistModelSelection {
        model: String,
        effort: Option<ReasoningEffort>,
    },

    /// Open the reasoning selection popup after picking a model.
    OpenReasoningPopup {
        model: String,
        presets: Vec<ModelPreset>,
    },

    /// Update the current approval policy in the running app and widget.
    UpdateAskForApprovalPolicy(AskForApproval),

    /// Update the current sandbox policy in the running app and widget.
    UpdateSandboxPolicy(SandboxPolicy),

    /// Forwarded conversation history snapshot from the current conversation.
    ConversationHistory(ConversationPathResponseEvent),

    /// Open the branch picker option from the review popup.
    OpenReviewBranchPicker(PathBuf),

    /// Open the commit picker option from the review popup.
    OpenReviewCommitPicker(PathBuf),

    /// Open the custom prompt option from the review popup.
    OpenReviewCustomPrompt,

    /// Open the approval popup.
    FullScreenApprovalRequest(ApprovalRequest),

    /// Open MCP manager panel when experimental overhaul is enabled.
    OpenMcpManager,

    /// Open the unified exec process manager overlay.
    OpenProcessManager,

    /// Open an input prompt targeting a unified exec session.
    OpenUnifiedExecInputPrompt {
        session_id: i32,
    },

    /// Open the output viewer for a specific unified exec session.
    OpenUnifiedExecOutput {
        session_id: i32,
    },

    /// Open the MCP wizard with optional template hint and pre-filled draft.
    OpenMcpWizard {
        template_id: Option<String>,
        draft: Option<McpWizardDraft>,
        existing_name: Option<String>,
    },

    /// Apply wizard changes (persist configuration, refresh views).
    ApplyMcpWizard {
        draft: McpWizardDraft,
        existing_name: Option<String>,
    },

    /// Reload MCP servers from disk and refresh the manager view.
    ReloadMcpServers,

    /// Remove a configured MCP server.
    RemoveMcpServer {
        name: String,
    },

    /// Send input to a running unified exec session.
    SendUnifiedExecInput {
        session_id: i32,
        input: String,
    },

    /// Kill a running unified exec session.
    KillUnifiedExecSession {
        session_id: i32,
    },

    /// Remove a unified exec session from the manager (after completion).
    RemoveUnifiedExecSession {
        session_id: i32,
    },

    /// Update the process manager with the latest unified exec snapshot.
    UpdateProcessManagerSessions {
        sessions: Vec<UnifiedExecSessionState>,
    },

    /// Refresh the process overview badge without opening the manager UI.
    RefreshProcessOverview,

    /// Refresh the output view for a running session.
    RefreshUnifiedExecOutput {
        session_id: i32,
    },

    /// Load a specific window of output for a unified exec session.
    LoadUnifiedExecOutputWindow {
        session_id: i32,
        window: UnifiedExecOutputWindow,
    },

    /// Prompt the user for a destination path to export a unified exec log.
    OpenUnifiedExecExportPrompt {
        session_id: i32,
    },

    /// Export the unified exec log to a user-provided destination.
    ExportUnifiedExecLog {
        session_id: i32,
        destination: PathBuf,
    },
}
