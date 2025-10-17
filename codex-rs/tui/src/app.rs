use crate::UpdateAction;
use crate::app_backtrack::BacktrackState;
use crate::app_event::AppEvent;
use crate::app_event_sender::AppEventSender;
use crate::bottom_pane::ApprovalRequest;
use crate::chatwidget::ChatWidget;
use crate::diff_render::DiffSummary;
use crate::exec_command::strip_bash_lc_and_escape;
use crate::file_search::FileSearchManager;
use crate::history_cell::HistoryCell;
use crate::mcp::McpManagerEntry;
use crate::mcp::McpManagerState;
use crate::mcp::McpWizardDraft;
use crate::pager_overlay::Overlay;
use crate::process_manager::ProcessManagerEntry;
use crate::process_manager::entry_and_data_from_output;
use crate::render::highlight::highlight_bash_to_lines;
use crate::resume_picker::ResumeSelection;
use crate::tui;
use crate::tui::TuiEvent;
use codex_ansi_escape::ansi_escape_line;
use codex_core::AuthManager;
use codex_core::CodexConversation;
use codex_core::ConversationManager;
use codex_core::UnifiedExecError;
use codex_core::UnifiedExecOutputWindow;
use codex_core::config::Config;
use codex_core::config::persist_model_selection;
use codex_core::config_types::McpServerConfig;
use codex_core::mcp::registry::McpRegistry;
use codex_core::mcp::templates::TemplateCatalog;
use codex_core::model_family::find_family_for_model;
use codex_core::protocol::SessionSource;
use codex_core::protocol::TokenUsage;
use codex_core::protocol_config_types::ReasoningEffort as ReasoningEffortConfig;
use codex_protocol::ConversationId;
use color_eyre::eyre::Result;
use color_eyre::eyre::WrapErr;
use color_eyre::eyre::eyre;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use ratatui::style::Stylize;
use ratatui::text::Line;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;
use tokio::select;
use tokio::sync::mpsc::unbounded_channel;
// use uuid::Uuid;
use tracing::warn;

#[derive(Debug, Clone)]
pub struct AppExitInfo {
    pub token_usage: TokenUsage,
    pub conversation_id: Option<ConversationId>,
    pub update_action: Option<UpdateAction>,
}

pub(crate) struct App {
    pub(crate) server: Arc<ConversationManager>,
    pub(crate) app_event_tx: AppEventSender,
    pub(crate) chat_widget: ChatWidget,
    pub(crate) auth_manager: Arc<AuthManager>,

    /// Config is stored here so we can recreate ChatWidgets as needed.
    pub(crate) config: Config,
    pub(crate) active_profile: Option<String>,

    pub(crate) file_search: FileSearchManager,

    pub(crate) transcript_cells: Vec<Arc<dyn HistoryCell>>,

    // Pager overlay state (Transcript or Static like Diff)
    pub(crate) overlay: Option<Overlay>,
    pub(crate) deferred_history_lines: Vec<Line<'static>>,
    pub(crate) has_emitted_history_lines: bool,

    pub(crate) enhanced_keys_supported: bool,

    /// Controls the animation thread that sends CommitTick events.
    pub(crate) commit_anim_running: Arc<AtomicBool>,

    // Esc-backtracking state grouped
    pub(crate) backtrack: crate::app_backtrack::BacktrackState,

    /// Set when the user confirms an update; propagated on exit.
    pub(crate) pending_update_action: Option<UpdateAction>,

    /// Cached unified exec sessions for reuse when opening the manager.
    latest_process_manager_entries: Vec<ProcessManagerEntry>,
}

impl App {
    pub async fn run(
        tui: &mut tui::Tui,
        auth_manager: Arc<AuthManager>,
        config: Config,
        active_profile: Option<String>,
        initial_prompt: Option<String>,
        initial_images: Vec<PathBuf>,
        resume_selection: ResumeSelection,
    ) -> Result<AppExitInfo> {
        use tokio_stream::StreamExt;
        let (app_event_tx, mut app_event_rx) = unbounded_channel();
        let app_event_tx = AppEventSender::new(app_event_tx);

        let conversation_manager = Arc::new(ConversationManager::new(
            auth_manager.clone(),
            SessionSource::Cli,
        ));

        let enhanced_keys_supported = tui.enhanced_keys_supported();

        let chat_widget = match resume_selection {
            ResumeSelection::StartFresh | ResumeSelection::Exit => {
                let init = crate::chatwidget::ChatWidgetInit {
                    config: config.clone(),
                    frame_requester: tui.frame_requester(),
                    app_event_tx: app_event_tx.clone(),
                    initial_prompt: initial_prompt.clone(),
                    initial_images: initial_images.clone(),
                    enhanced_keys_supported,
                    auth_manager: auth_manager.clone(),
                };
                ChatWidget::new(init, conversation_manager.clone())
            }
            ResumeSelection::Resume(path) => {
                let resumed = conversation_manager
                    .resume_conversation_from_rollout(
                        config.clone(),
                        path.clone(),
                        auth_manager.clone(),
                    )
                    .await
                    .wrap_err_with(|| {
                        format!("Failed to resume session from {}", path.display())
                    })?;
                let init = crate::chatwidget::ChatWidgetInit {
                    config: config.clone(),
                    frame_requester: tui.frame_requester(),
                    app_event_tx: app_event_tx.clone(),
                    initial_prompt: initial_prompt.clone(),
                    initial_images: initial_images.clone(),
                    enhanced_keys_supported,
                    auth_manager: auth_manager.clone(),
                };
                ChatWidget::new_from_existing(
                    init,
                    resumed.conversation,
                    resumed.session_configured,
                )
            }
        };

        let file_search = FileSearchManager::new(config.cwd.clone(), app_event_tx.clone());

        let mut app = Self {
            server: conversation_manager,
            app_event_tx,
            chat_widget,
            auth_manager: auth_manager.clone(),
            config,
            active_profile,
            file_search,
            enhanced_keys_supported,
            transcript_cells: Vec::new(),
            overlay: None,
            deferred_history_lines: Vec::new(),
            has_emitted_history_lines: false,
            commit_anim_running: Arc::new(AtomicBool::new(false)),
            backtrack: BacktrackState::default(),
            pending_update_action: None,
            latest_process_manager_entries: Vec::new(),
        };

        let tui_events = tui.event_stream();
        tokio::pin!(tui_events);

        tui.frame_requester().schedule_frame();

        while select! {
            Some(event) = app_event_rx.recv() => {
                app.handle_event(tui, event).await?
            }
            Some(event) = tui_events.next() => {
                app.handle_tui_event(tui, event).await?
            }
        } {}
        tui.terminal.clear()?;
        Ok(AppExitInfo {
            token_usage: app.token_usage(),
            conversation_id: app.chat_widget.conversation_id(),
            update_action: app.pending_update_action,
        })
    }

    pub(crate) async fn handle_tui_event(
        &mut self,
        tui: &mut tui::Tui,
        event: TuiEvent,
    ) -> Result<bool> {
        if self.overlay.is_some() {
            let _ = self.handle_backtrack_overlay_event(tui, event).await?;
        } else {
            match event {
                TuiEvent::Key(key_event) => {
                    self.handle_key_event(tui, key_event).await;
                }
                TuiEvent::Paste(pasted) => {
                    // Many terminals convert newlines to \r when pasting (e.g., iTerm2),
                    // but tui-textarea expects \n. Normalize CR to LF.
                    // [tui-textarea]: https://github.com/rhysd/tui-textarea/blob/4d18622eeac13b309e0ff6a55a46ac6706da68cf/src/textarea.rs#L782-L783
                    // [iTerm2]: https://github.com/gnachman/iTerm2/blob/5d0c0d9f68523cbd0494dad5422998964a2ecd8d/sources/iTermPasteHelper.m#L206-L216
                    let pasted = pasted.replace("\r", "\n");
                    self.chat_widget.handle_paste(pasted);
                }
                TuiEvent::Draw => {
                    self.chat_widget.maybe_post_pending_notification(tui);
                    if self
                        .chat_widget
                        .handle_paste_burst_tick(tui.frame_requester())
                    {
                        return Ok(true);
                    }
                    tui.draw(
                        self.chat_widget.desired_height(tui.terminal.size()?.width),
                        |frame| {
                            frame.render_widget_ref(&self.chat_widget, frame.area());
                            if let Some((x, y)) = self.chat_widget.cursor_pos(frame.area()) {
                                frame.set_cursor_position((x, y));
                            }
                        },
                    )?;
                }
            }
        }
        Ok(true)
    }

    async fn handle_event(&mut self, tui: &mut tui::Tui, event: AppEvent) -> Result<bool> {
        match event {
            AppEvent::NewSession => {
                let init = crate::chatwidget::ChatWidgetInit {
                    config: self.config.clone(),
                    frame_requester: tui.frame_requester(),
                    app_event_tx: self.app_event_tx.clone(),
                    initial_prompt: None,
                    initial_images: Vec::new(),
                    enhanced_keys_supported: self.enhanced_keys_supported,
                    auth_manager: self.auth_manager.clone(),
                };
                self.chat_widget = ChatWidget::new(init, self.server.clone());
                tui.frame_requester().schedule_frame();
            }
            AppEvent::InsertHistoryCell(cell) => {
                let cell: Arc<dyn HistoryCell> = cell.into();
                if let Some(Overlay::Transcript(t)) = &mut self.overlay {
                    t.insert_cell(cell.clone());
                    tui.frame_requester().schedule_frame();
                }
                self.transcript_cells.push(cell.clone());
                let mut display = cell.display_lines(tui.terminal.last_known_screen_size.width);
                if !display.is_empty() {
                    // Only insert a separating blank line for new cells that are not
                    // part of an ongoing stream. Streaming continuations should not
                    // accrue extra blank lines between chunks.
                    if !cell.is_stream_continuation() {
                        if self.has_emitted_history_lines {
                            display.insert(0, Line::from(""));
                        } else {
                            self.has_emitted_history_lines = true;
                        }
                    }
                    if self.overlay.is_some() {
                        self.deferred_history_lines.extend(display);
                    } else {
                        tui.insert_history_lines(display);
                    }
                }
            }
            AppEvent::StartCommitAnimation => {
                if self
                    .commit_anim_running
                    .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                    .is_ok()
                {
                    let tx = self.app_event_tx.clone();
                    let running = self.commit_anim_running.clone();
                    thread::spawn(move || {
                        while running.load(Ordering::Relaxed) {
                            thread::sleep(Duration::from_millis(50));
                            tx.send(AppEvent::CommitTick);
                        }
                    });
                }
            }
            AppEvent::StopCommitAnimation => {
                self.commit_anim_running.store(false, Ordering::Release);
            }
            AppEvent::CommitTick => {
                self.chat_widget.on_commit_tick();
            }
            AppEvent::CodexEvent(event) => {
                self.chat_widget.handle_codex_event(event);
            }
            AppEvent::ConversationHistory(ev) => {
                self.on_conversation_history_for_backtrack(tui, ev).await?;
            }
            AppEvent::ExitRequest => {
                return Ok(false);
            }
            AppEvent::OpenMcpManager => {
                self.open_mcp_manager()?;
            }
            AppEvent::OpenProcessManager => {
                self.open_process_manager().await?;
            }
            AppEvent::OpenUnifiedExecOutput { session_id } => {
                self.open_unified_exec_output(session_id).await?;
            }
            AppEvent::OpenUnifiedExecInputPrompt { session_id } => {
                self.chat_widget.open_unified_exec_input_prompt(session_id);
            }
            AppEvent::SendUnifiedExecInput { session_id, input } => {
                self.send_unified_exec_input(session_id, input).await?;
            }
            AppEvent::KillUnifiedExecSession { session_id } => {
                self.kill_unified_exec_session(session_id).await?;
            }
            AppEvent::RemoveUnifiedExecSession { session_id } => {
                self.remove_unified_exec_session(session_id).await?;
            }
            AppEvent::RefreshUnifiedExecOutput { session_id } => {
                self.refresh_unified_exec_output(session_id).await?;
            }
            AppEvent::LoadUnifiedExecOutputWindow { session_id, window } => {
                self.load_unified_exec_output_window(session_id, window)
                    .await?;
            }
            AppEvent::OpenUnifiedExecExportPrompt { session_id } => {
                self.chat_widget.open_unified_exec_export_prompt(session_id);
            }
            AppEvent::ExportUnifiedExecLog {
                session_id,
                destination,
            } => {
                self.export_unified_exec_log(session_id, destination)
                    .await?;
            }
            AppEvent::UpdateProcessManagerSessions { sessions } => {
                self.latest_process_manager_entries = sessions
                    .into_iter()
                    .map(ProcessManagerEntry::from_state)
                    .collect();
                self.chat_widget
                    .update_process_manager(self.latest_process_manager_entries.clone());
            }
            AppEvent::RefreshProcessOverview => {
                self.chat_widget
                    .refresh_process_overview(&self.latest_process_manager_entries);
            }
            AppEvent::OpenMcpWizard {
                template_id,
                draft,
                existing_name,
            } => {
                self.open_mcp_wizard(template_id, draft, existing_name)?;
            }
            AppEvent::ApplyMcpWizard {
                draft,
                existing_name,
            } => {
                self.apply_mcp_wizard(draft, existing_name)?;
            }
            AppEvent::ReloadMcpServers => {
                self.reload_mcp_servers()?;
            }
            AppEvent::RemoveMcpServer { name } => {
                self.remove_mcp_server(name)?;
            }
            AppEvent::CodexOp(op) => self.chat_widget.submit_op(op),
            AppEvent::DiffResult(text) => {
                // Clear the in-progress state in the bottom pane
                self.chat_widget.on_diff_complete();
                // Enter alternate screen using TUI helper and build pager lines
                let _ = tui.enter_alt_screen();
                let pager_lines: Vec<ratatui::text::Line<'static>> = if text.trim().is_empty() {
                    vec!["No changes detected.".italic().into()]
                } else {
                    text.lines().map(ansi_escape_line).collect()
                };
                self.overlay = Some(Overlay::new_static_with_lines(
                    pager_lines,
                    "D I F F".to_string(),
                ));
                tui.frame_requester().schedule_frame();
            }
            AppEvent::StartFileSearch(query) => {
                if !query.is_empty() {
                    self.file_search.on_user_query(query);
                }
            }
            AppEvent::FileSearchResult { query, matches } => {
                self.chat_widget.apply_file_search_result(query, matches);
            }
            AppEvent::UpdateReasoningEffort(effort) => {
                self.on_update_reasoning_effort(effort);
            }
            AppEvent::UpdateModel(model) => {
                self.chat_widget.set_model(&model);
                self.config.model = model.clone();
                if let Some(family) = find_family_for_model(&model) {
                    self.config.model_family = family;
                }
            }
            AppEvent::OpenReasoningPopup { model, presets } => {
                self.chat_widget.open_reasoning_popup(model, presets);
            }
            AppEvent::PersistModelSelection { model, effort } => {
                let profile = self.active_profile.as_deref();
                match persist_model_selection(&self.config.codex_home, profile, &model, effort)
                    .await
                {
                    Ok(()) => {
                        let effort_label = effort
                            .map(|eff| format!(" with {eff} reasoning"))
                            .unwrap_or_else(|| " with default reasoning".to_string());
                        if let Some(profile) = profile {
                            self.chat_widget.add_info_message(
                                format!(
                                    "Model changed to {model}{effort_label} for {profile} profile"
                                ),
                                None,
                            );
                        } else {
                            self.chat_widget.add_info_message(
                                format!("Model changed to {model}{effort_label}"),
                                None,
                            );
                        }
                    }
                    Err(err) => {
                        tracing::error!(
                            error = %err,
                            "failed to persist model selection"
                        );
                        if let Some(profile) = profile {
                            self.chat_widget.add_error_message(format!(
                                "Failed to save model for profile `{profile}`: {err}"
                            ));
                        } else {
                            self.chat_widget
                                .add_error_message(format!("Failed to save default model: {err}"));
                        }
                    }
                }
            }
            AppEvent::UpdateAskForApprovalPolicy(policy) => {
                self.chat_widget.set_approval_policy(policy);
            }
            AppEvent::UpdateSandboxPolicy(policy) => {
                self.chat_widget.set_sandbox_policy(policy);
            }
            AppEvent::OpenReviewBranchPicker(cwd) => {
                self.chat_widget.show_review_branch_picker(&cwd).await;
            }
            AppEvent::OpenReviewCommitPicker(cwd) => {
                self.chat_widget.show_review_commit_picker(&cwd).await;
            }
            AppEvent::OpenReviewCustomPrompt => {
                self.chat_widget.show_review_custom_prompt();
            }
            AppEvent::FullScreenApprovalRequest(request) => match request {
                ApprovalRequest::ApplyPatch { cwd, changes, .. } => {
                    let _ = tui.enter_alt_screen();
                    let diff_summary = DiffSummary::new(changes, cwd);
                    self.overlay = Some(Overlay::new_static_with_renderables(
                        vec![diff_summary.into()],
                        "P A T C H".to_string(),
                    ));
                }
                ApprovalRequest::Exec { command, .. } => {
                    let _ = tui.enter_alt_screen();
                    let full_cmd = strip_bash_lc_and_escape(&command);
                    let full_cmd_lines = highlight_bash_to_lines(&full_cmd);
                    self.overlay = Some(Overlay::new_static_with_lines(
                        full_cmd_lines,
                        "E X E C".to_string(),
                    ));
                }
            },
        }
        Ok(true)
    }

    pub(crate) fn token_usage(&self) -> codex_core::protocol::TokenUsage {
        self.chat_widget.token_usage()
    }
    fn open_mcp_manager(&mut self) -> Result<()> {
        if !self.config.experimental_mcp_overhaul {
            self.chat_widget.show_mcp_history_summary();
            return Ok(());
        }

        let catalog = self.load_template_catalog();
        let registry = McpRegistry::new(&self.config, catalog);
        let state = McpManagerState::from_registry(&registry);
        let entries: Vec<McpManagerEntry> = state
            .servers
            .into_iter()
            .map(|snapshot| {
                let health = registry.health_report(&snapshot.name);
                McpManagerEntry { snapshot, health }
            })
            .collect();

        self.chat_widget
            .show_mcp_manager(entries, state.template_count);
        Ok(())
    }

    fn open_mcp_wizard(
        &mut self,
        template_id: Option<String>,
        draft: Option<McpWizardDraft>,
        existing_name: Option<String>,
    ) -> Result<()> {
        if !self.config.experimental_mcp_overhaul {
            self.chat_widget.show_mcp_history_summary();
            return Ok(());
        }

        let catalog = self.load_template_catalog();
        let mut draft = draft.unwrap_or_default();
        if draft.template_id.is_none()
            && let Some(id) = template_id
            && let Some(cfg) = catalog.instantiate(&id)
        {
            draft.apply_template_config(&cfg);
            draft.template_id = Some(id);
        }
        if draft.name.is_empty()
            && existing_name.is_none()
            && let Some(id) = draft.template_id.clone()
        {
            draft.name = sanitize_name(&id);
        }

        self.chat_widget
            .show_mcp_wizard(catalog, Some(draft), existing_name);
        Ok(())
    }

    async fn open_process_manager(&mut self) -> Result<()> {
        if let Some(entries) = self.load_unified_exec_entries().await? {
            self.latest_process_manager_entries = entries.clone();
            self.chat_widget.show_process_manager(entries);
        } else if !self.latest_process_manager_entries.is_empty() {
            self.chat_widget
                .show_process_manager(self.latest_process_manager_entries.clone());
        }
        Ok(())
    }

    async fn refresh_process_manager(&mut self) -> Result<()> {
        self.open_process_manager().await
    }

    async fn load_unified_exec_entries(&mut self) -> Result<Option<Vec<ProcessManagerEntry>>> {
        let conversation = match self.conversation_for_unified_exec().await? {
            Some(conv) => conv,
            None => return Ok(None),
        };

        let snapshots = conversation.unified_exec_sessions().await;
        let entries = snapshots
            .into_iter()
            .map(ProcessManagerEntry::from_snapshot)
            .collect();
        Ok(Some(entries))
    }

    fn upsert_process_entry(&mut self, entry: ProcessManagerEntry) {
        if let Some(existing) = self
            .latest_process_manager_entries
            .iter_mut()
            .find(|existing| existing.session_id == entry.session_id)
        {
            *existing = entry;
        } else {
            self.latest_process_manager_entries.push(entry);
        }
    }

    async fn open_unified_exec_output(&mut self, session_id: i32) -> Result<()> {
        let conversation = match self.conversation_for_unified_exec().await? {
            Some(conv) => conv,
            None => return Ok(()),
        };

        let Some(output) = conversation.unified_exec_output(session_id).await else {
            self.chat_widget
                .add_info_message(format!("Session {session_id} is no longer tracked."), None);
            return Ok(());
        };

        let (entry, data) = entry_and_data_from_output(output);
        self.upsert_process_entry(entry.clone());
        self.chat_widget
            .update_process_manager(self.latest_process_manager_entries.clone());
        self.chat_widget.update_unified_exec_output(entry, data);
        Ok(())
    }

    async fn refresh_unified_exec_output(&mut self, session_id: i32) -> Result<()> {
        self.open_unified_exec_output(session_id).await
    }

    async fn load_unified_exec_output_window(
        &mut self,
        session_id: i32,
        window: UnifiedExecOutputWindow,
    ) -> Result<()> {
        let conversation = match self.conversation_for_unified_exec().await? {
            Some(conv) => conv,
            None => return Ok(()),
        };

        let Some(output) = conversation
            .unified_exec_output_window(session_id, window)
            .await
        else {
            self.chat_widget
                .add_info_message(format!("Session {session_id} is no longer tracked."), None);
            return Ok(());
        };

        let (entry, data) = entry_and_data_from_output(output);
        self.upsert_process_entry(entry.clone());
        self.chat_widget
            .update_process_manager(self.latest_process_manager_entries.clone());
        self.chat_widget.update_unified_exec_output(entry, data);
        Ok(())
    }

    async fn export_unified_exec_log(
        &mut self,
        session_id: i32,
        destination: PathBuf,
    ) -> Result<()> {
        let conversation = match self.conversation_for_unified_exec().await? {
            Some(conv) => conv,
            None => return Ok(()),
        };

        match conversation
            .export_unified_exec_log(session_id, destination.clone())
            .await
        {
            Ok(()) => self.chat_widget.add_info_message(
                format!(
                    "Exported session {session_id} log to {}",
                    destination.display()
                ),
                None,
            ),
            Err(err) => self
                .chat_widget
                .add_error_message(format!("Failed to export session {session_id} log: {err}")),
        }
        Ok(())
    }

    async fn conversation_for_unified_exec(&mut self) -> Result<Option<Arc<CodexConversation>>> {
        let Some(conversation_id) = self.chat_widget.conversation_id() else {
            self.chat_widget.add_info_message(
                "Unified exec sessions become available after the first turn.".to_string(),
                None,
            );
            return Ok(None);
        };

        match self.server.get_conversation(conversation_id).await {
            Ok(conv) => Ok(Some(conv)),
            Err(err) => {
                self.chat_widget
                    .add_error_message(format!("Failed to access active conversation: {err}"));
                Ok(None)
            }
        }
    }

    async fn send_unified_exec_input(&mut self, session_id: i32, input: String) -> Result<()> {
        let conversation = match self.conversation_for_unified_exec().await? {
            Some(conv) => conv,
            None => return Ok(()),
        };

        let mut chunk = input;
        if !chunk.ends_with('\n') {
            chunk.push('\n');
        }
        let chunks = vec![chunk];
        match conversation
            .run_unified_exec(Some(session_id), &chunks, None)
            .await
        {
            Ok(_) => {}
            Err(UnifiedExecError::UnknownSessionId { .. }) => {
                self.chat_widget
                    .add_info_message(format!("Session {session_id} is no longer active."), None);
            }
            Err(err) => {
                self.chat_widget
                    .add_error_message(format!("Failed to send input: {err}"));
            }
        }

        self.refresh_process_manager().await?;
        Ok(())
    }

    async fn kill_unified_exec_session(&mut self, session_id: i32) -> Result<()> {
        let conversation = match self.conversation_for_unified_exec().await? {
            Some(conv) => conv,
            None => return Ok(()),
        };

        if !conversation.kill_unified_exec_session(session_id).await {
            self.chat_widget
                .add_info_message(format!("Session {session_id} is no longer running."), None);
        }

        self.refresh_process_manager().await?;
        Ok(())
    }

    async fn remove_unified_exec_session(&mut self, session_id: i32) -> Result<()> {
        let conversation = match self.conversation_for_unified_exec().await? {
            Some(conv) => conv,
            None => return Ok(()),
        };

        if !conversation.remove_unified_exec_session(session_id).await {
            self.chat_widget
                .add_info_message(format!("Session {session_id} is no longer tracked."), None);
        }

        self.refresh_process_manager().await?;
        Ok(())
    }

    fn apply_mcp_wizard(
        &mut self,
        draft: McpWizardDraft,
        existing_name: Option<String>,
    ) -> Result<()> {
        if !self.config.experimental_mcp_overhaul {
            self.chat_widget.show_mcp_history_summary();
            return Ok(());
        }

        let catalog = self.load_template_catalog();
        let retry_draft = draft.clone();
        let server_config = match draft.build_server_config(&catalog) {
            Ok(cfg) => cfg,
            Err(err) => {
                self.chat_widget
                    .add_error_message(format!("Failed to validate MCP server: {err}"));
                self.chat_widget
                    .show_mcp_wizard(catalog, Some(retry_draft), existing_name);
                return Ok(());
            }
        };

        let registry = McpRegistry::new(&self.config, catalog);
        registry
            .upsert_server_with_existing(
                existing_name.as_deref(),
                &draft.name,
                server_config.clone(),
            )
            .map_err(|err| eyre!(err))?;

        if let Some(ref old_name) = existing_name
            && old_name != &draft.name
        {
            self.config.mcp_servers.remove(old_name);
        }
        self.config
            .mcp_servers
            .insert(draft.name.clone(), server_config);
        self.chat_widget
            .set_mcp_servers(self.current_mcp_servers_btree());
        self.chat_widget
            .add_info_message(format!("Saved MCP server '{}'", draft.name), None);
        self.open_mcp_manager()?;
        Ok(())
    }

    fn reload_mcp_servers(&mut self) -> Result<()> {
        if !self.config.experimental_mcp_overhaul {
            self.chat_widget.show_mcp_history_summary();
            return Ok(());
        }

        let catalog = self.load_template_catalog();
        let registry = McpRegistry::new(&self.config, catalog);
        let servers = registry.reload_servers().map_err(|err| eyre!(err))?;
        self.config.mcp_servers = servers.clone().into_iter().collect();
        self.chat_widget.set_mcp_servers(servers);
        self.open_mcp_manager()?;
        Ok(())
    }

    fn remove_mcp_server(&mut self, name: String) -> Result<()> {
        if !self.config.experimental_mcp_overhaul {
            self.chat_widget.show_mcp_history_summary();
            return Ok(());
        }

        let catalog = self.load_template_catalog();
        let registry = McpRegistry::new(&self.config, catalog);
        if registry.remove_server(&name).map_err(|err| eyre!(err))? {
            self.config.mcp_servers.remove(&name);
            self.chat_widget
                .set_mcp_servers(self.current_mcp_servers_btree());
            self.chat_widget
                .add_info_message(format!("Removed MCP server '{name}'."), None);
        } else {
            self.chat_widget
                .add_info_message(format!("No MCP server named '{name}' found."), None);
        }
        self.open_mcp_manager()?;
        Ok(())
    }

    fn load_template_catalog(&self) -> TemplateCatalog {
        TemplateCatalog::load_default().unwrap_or_else(|err| {
            warn!(error = %err, "Failed to load MCP templates");
            TemplateCatalog::empty()
        })
    }

    fn current_mcp_servers_btree(&self) -> BTreeMap<String, McpServerConfig> {
        self.config
            .mcp_servers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    fn on_update_reasoning_effort(&mut self, effort: Option<ReasoningEffortConfig>) {
        self.chat_widget.set_reasoning_effort(effort);
        self.config.model_reasoning_effort = effort;
    }

    async fn handle_key_event(&mut self, tui: &mut tui::Tui, key_event: KeyEvent) {
        match key_event {
            KeyEvent {
                code: KeyCode::Char('t'),
                modifiers: crossterm::event::KeyModifiers::CONTROL,
                kind: KeyEventKind::Press,
                ..
            } => {
                // Enter alternate screen and set viewport to full size.
                let _ = tui.enter_alt_screen();
                self.overlay = Some(Overlay::new_transcript(self.transcript_cells.clone()));
                tui.frame_requester().schedule_frame();
            }
            // Esc primes/advances backtracking only in normal (not working) mode
            // with the composer focused and empty. In any other state, forward
            // Esc so the active UI (e.g. status indicator, modals, popups)
            // handles it.
            KeyEvent {
                code: KeyCode::Esc,
                kind: KeyEventKind::Press | KeyEventKind::Repeat,
                ..
            } => {
                if self.chat_widget.is_normal_backtrack_mode()
                    && self.chat_widget.composer_is_empty()
                {
                    self.handle_backtrack_esc_key(tui);
                } else {
                    self.chat_widget.handle_key_event(key_event);
                }
            }
            // Enter confirms backtrack when primed + count > 0. Otherwise pass to widget.
            KeyEvent {
                code: KeyCode::Enter,
                kind: KeyEventKind::Press,
                ..
            } if self.backtrack.primed
                && self.backtrack.nth_user_message != usize::MAX
                && self.chat_widget.composer_is_empty() =>
            {
                // Delegate to helper for clarity; preserves behavior.
                self.confirm_backtrack_from_main();
            }
            KeyEvent {
                kind: KeyEventKind::Press | KeyEventKind::Repeat,
                ..
            } => {
                // Any non-Esc key press should cancel a primed backtrack.
                // This avoids stale "Esc-primed" state after the user starts typing
                // (even if they later backspace to empty).
                if key_event.code != KeyCode::Esc && self.backtrack.primed {
                    self.reset_backtrack_state();
                }
                self.chat_widget.handle_key_event(key_event);
            }
            _ => {
                // Ignore Release key events.
            }
        };
    }
}
fn sanitize_name(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_backtrack::BacktrackState;
    use crate::app_backtrack::user_count;
    use crate::chatwidget::tests::make_chatwidget_manual_with_sender;
    use crate::file_search::FileSearchManager;
    use crate::history_cell::AgentMessageCell;
    use crate::history_cell::HistoryCell;
    use crate::history_cell::UserHistoryCell;
    use crate::history_cell::new_session_info;
    use codex_core::AuthManager;
    use codex_core::CodexAuth;
    use codex_core::ConversationManager;
    use codex_core::protocol::SessionConfiguredEvent;
    use codex_protocol::ConversationId;
    use ratatui::prelude::Line;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    fn make_test_app() -> App {
        let (chat_widget, app_event_tx, _rx, _op_rx) = make_chatwidget_manual_with_sender();
        let config = chat_widget.config_ref().clone();

        let server = Arc::new(ConversationManager::with_auth(CodexAuth::from_api_key(
            "Test API Key",
        )));
        let auth_manager =
            AuthManager::from_auth_for_testing(CodexAuth::from_api_key("Test API Key"));
        let file_search = FileSearchManager::new(config.cwd.clone(), app_event_tx.clone());

        App {
            server,
            app_event_tx,
            chat_widget,
            auth_manager,
            config,
            active_profile: None,
            file_search,
            transcript_cells: Vec::new(),
            overlay: None,
            deferred_history_lines: Vec::new(),
            has_emitted_history_lines: false,
            enhanced_keys_supported: false,
            commit_anim_running: Arc::new(AtomicBool::new(false)),
            backtrack: BacktrackState::default(),
            pending_update_action: None,
            latest_process_manager_entries: Vec::new(),
        }
    }

    #[test]
    fn update_reasoning_effort_updates_config() {
        let mut app = make_test_app();
        app.config.model_reasoning_effort = Some(ReasoningEffortConfig::Medium);
        app.chat_widget
            .set_reasoning_effort(Some(ReasoningEffortConfig::Medium));

        app.on_update_reasoning_effort(Some(ReasoningEffortConfig::High));

        assert_eq!(
            app.config.model_reasoning_effort,
            Some(ReasoningEffortConfig::High)
        );
        assert_eq!(
            app.chat_widget.config_ref().model_reasoning_effort,
            Some(ReasoningEffortConfig::High)
        );
    }

    #[test]
    fn backtrack_selection_with_duplicate_history_targets_unique_turn() {
        let mut app = make_test_app();

        let user_cell = |text: &str| -> Arc<dyn HistoryCell> {
            Arc::new(UserHistoryCell {
                message: text.to_string(),
            }) as Arc<dyn HistoryCell>
        };
        let agent_cell = |text: &str| -> Arc<dyn HistoryCell> {
            Arc::new(AgentMessageCell::new(
                vec![Line::from(text.to_string())],
                true,
            )) as Arc<dyn HistoryCell>
        };

        let make_header = |is_first| {
            let event = SessionConfiguredEvent {
                session_id: ConversationId::new(),
                model: "gpt-test".to_string(),
                reasoning_effort: None,
                history_log_id: 0,
                history_entry_count: 0,
                initial_messages: None,
                rollout_path: PathBuf::new(),
            };
            Arc::new(new_session_info(
                app.chat_widget.config_ref(),
                event,
                is_first,
            )) as Arc<dyn HistoryCell>
        };

        // Simulate the transcript after trimming for a fork, replaying history, and
        // appending the edited turn. The session header separates the retained history
        // from the forked conversation's replayed turns.
        app.transcript_cells = vec![
            make_header(true),
            user_cell("first question"),
            agent_cell("answer first"),
            user_cell("follow-up"),
            agent_cell("answer follow-up"),
            make_header(false),
            user_cell("first question"),
            agent_cell("answer first"),
            user_cell("follow-up (edited)"),
            agent_cell("answer edited"),
        ];

        assert_eq!(user_count(&app.transcript_cells), 2);

        app.backtrack.base_id = Some(ConversationId::new());
        app.backtrack.primed = true;
        app.backtrack.nth_user_message = user_count(&app.transcript_cells).saturating_sub(1);

        app.confirm_backtrack_from_main();

        let (_, nth, prefill) = app.backtrack.pending.clone().expect("pending backtrack");
        assert_eq!(nth, 1);
        assert_eq!(prefill, "follow-up (edited)");
    }
}
