use codex_core::protocol::ConversationPathResponseEvent;
use codex_core::protocol::Event;
use codex_file_search::FileMatch;
use std::path::PathBuf;

use crate::history_cell::HistoryCell;
use crate::mcp::McpWizardDraft;

use codex_core::protocol::AskForApproval;
use codex_core::protocol::SandboxPolicy;
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

    /// Update the current approval policy in the running app and widget.
    UpdateAskForApprovalPolicy(AskForApproval),

    /// Update the current sandbox policy in the running app and widget.
    UpdateSandboxPolicy(SandboxPolicy),

    /// Forwarded conversation history snapshot from the current conversation.
    ConversationHistory(ConversationPathResponseEvent),

    /// Open MCP manager panel when experimental overhaul is enabled.
    OpenMcpManager,

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

    /// Request to open a file or directory from the explorer pane.
    #[allow(dead_code)]
    OpenFilePath {
        path: PathBuf,
    },
}
