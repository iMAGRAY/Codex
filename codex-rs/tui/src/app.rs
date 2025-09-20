use crate::app_backtrack::BacktrackState;
use crate::app_event::AppEvent;
use crate::app_event_sender::AppEventSender;
use crate::chatwidget::ChatWidget;
use crate::file_explorer::FileExplorerState;
use crate::file_search::FileSearchManager;
use crate::mcp::McpManagerEntry;
use crate::mcp::McpManagerState;
use crate::mcp::McpWizardDraft;
use crate::pager_overlay::Overlay;
use crate::resume_picker::ResumeSelection;
use crate::stellar::ControllerOutcome;
use crate::stellar::StellarController;
use crate::stellar::StellarView;
use crate::tui;
use crate::tui::TuiEvent;
use codex_ansi_escape::ansi_escape_line;
use codex_core::AuthManager;
use codex_core::ConversationManager;
use codex_core::config::Config;
use codex_core::config::persist_model_selection;
use codex_core::config_types::McpServerConfig;
use codex_core::mcp::registry::McpRegistry;
use codex_core::mcp::templates::TemplateCatalog;
use codex_core::model_family::find_family_for_model;
use codex_core::orchestrator::QuickstartInput;
use codex_core::orchestrator::build_quickstart;
use codex_core::protocol::TokenUsage;
use codex_core::protocol_config_types::ReasoningEffort as ReasoningEffortConfig;
use codex_core::stellar::ActionApplied;
use codex_core::stellar::KernelEvent;
use codex_core::stellar::StellarPersona;
use codex_core::telemetry;
use codex_core::telemetry::TelemetryExporter;
use codex_core::telemetry::TelemetryInstallError;
use color_eyre::eyre::Result;
use color_eyre::eyre::WrapErr;
use color_eyre::eyre::eyre;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use crossterm::event::KeyModifiers;
use crossterm::terminal::supports_keyboard_enhancement;
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::Line;
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{self, Read};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;
use tokio::select;
use tokio::sync::mpsc::unbounded_channel;
// use uuid::Uuid;

use tracing::warn;

const FILE_PREVIEW_LIMIT_BYTES: usize = 512 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
enum FocusTarget {
    Chat,
    Explorer,
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
    pub(crate) file_explorer: FileExplorerState,

    pub(crate) transcript_lines: Vec<Line<'static>>,

    // Pager overlay state (Transcript or Static like Diff)
    pub(crate) overlay: Option<Overlay>,
    pub(crate) deferred_history_lines: Vec<Line<'static>>,
    has_emitted_history_lines: bool,

    pub(crate) enhanced_keys_supported: bool,

    /// Controls the animation thread that sends CommitTick events.
    pub(crate) commit_anim_running: Arc<AtomicBool>,

    // Esc-backtracking state grouped
    pub(crate) backtrack: crate::app_backtrack::BacktrackState,
    pub(crate) stellar: StellarController,

    focus: FocusTarget,
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
    ) -> Result<TokenUsage> {
        use tokio_stream::StreamExt;
        let (app_event_tx, mut app_event_rx) = unbounded_channel();
        let app_event_tx = AppEventSender::new(app_event_tx);

        let conversation_manager = Arc::new(ConversationManager::new(auth_manager.clone()));

        setup_telemetry_exporter(&config);

        let enhanced_keys_supported = supports_keyboard_enhancement().unwrap_or(false);

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
        let file_explorer = FileExplorerState::new(config.cwd.clone());

        let mut app = Self {
            server: conversation_manager,
            app_event_tx,
            chat_widget,
            auth_manager: auth_manager.clone(),
            config,
            active_profile,
            file_search,
            file_explorer,
            enhanced_keys_supported,
            transcript_lines: Vec::new(),
            overlay: None,
            deferred_history_lines: Vec::new(),
            has_emitted_history_lines: false,
            commit_anim_running: Arc::new(AtomicBool::new(false)),
            backtrack: BacktrackState::default(),
            stellar: StellarController::new(StellarPersona::Operator),
            focus: FocusTarget::Chat,
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
        Ok(app.token_usage())
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
                    let terminal_size = tui.terminal.size()?;
                    let width = terminal_size.width;
                    let explorer_width = self.explorer_panel_width(width);
                    let separator_width: u16 = if explorer_width > 0 { 1 } else { 0 };
                    let chat_width = width
                        .saturating_sub(explorer_width + separator_width)
                        .max(1);
                    let stellar_active = self.stellar.is_active();
                    if stellar_active {
                        self.stellar.sync_layout(chat_width);
                    }
                    let stellar_snapshot = stellar_active.then(|| self.stellar.snapshot());
                    let stellar_height = if stellar_active {
                        self.stellar
                            .preferred_height()
                            .min(terminal_size.height)
                    } else {
                        0
                    };
                    let desired_height = self.chat_widget.desired_height(chat_width) + stellar_height;
                    tui.draw(desired_height, |frame| {
                        let area = frame.area();
                        let explorer_width = self.explorer_panel_width(area.width);
                        let separator_width: u16 = if explorer_width > 0 { 1 } else { 0 };
                        let [explorer_area, separator_area, chat_container] =
                            ratatui::layout::Layout::horizontal([
                                ratatui::layout::Constraint::Length(explorer_width),
                                ratatui::layout::Constraint::Length(separator_width),
                                ratatui::layout::Constraint::Min(1),
                            ])
                            .areas(area);

                        if explorer_width > 0 {
                            frame.render_widget(
                                self.file_explorer
                                    .widget(self.focus == FocusTarget::Explorer),
                                explorer_area,
                            );
                            if separator_width > 0 {
                                Self::render_separator(separator_area, frame.buffer_mut());
                            }
                        }

                        if let Some(snapshot) = stellar_snapshot.as_ref() {
                            let layout: [ratatui::layout::Rect; 2] = ratatui::layout::Layout::vertical([
                                ratatui::layout::Constraint::Length(
                                    stellar_height.min(chat_container.height),
                                ),
                                ratatui::layout::Constraint::Min(1),
                            ])
                            .areas(chat_container);
                            frame.render_widget(StellarView::new(snapshot), layout[0]);
                            frame.render_widget_ref(&self.chat_widget, layout[1]);
                            if self.focus == FocusTarget::Chat {
                                if let Some((x, y)) = self.chat_widget.cursor_pos(layout[1]) {
                                    frame.set_cursor_position((x, y));
                                }
                            }
                        } else {
                            frame.render_widget_ref(&self.chat_widget, chat_container);
                            if self.focus == FocusTarget::Chat {
                                if let Some((x, y)) =
                                    self.chat_widget.cursor_pos(chat_container)
                                {
                                    frame.set_cursor_position((x, y));
                                }
                            }
                        }
                    })?;
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
                self.chat_widget
                    .set_input_focus(self.focus == FocusTarget::Chat);
                tui.frame_requester().schedule_frame();
            }
            AppEvent::InsertHistoryCell(cell) => {
                let mut cell_transcript = cell.transcript_lines();
                if !cell.is_stream_continuation() && !self.transcript_lines.is_empty() {
                    cell_transcript.insert(0, Line::from(""));
                }
                if let Some(Overlay::Transcript(t)) = &mut self.overlay {
                    t.insert_lines(cell_transcript.clone());
                    tui.frame_requester().schedule_frame();
                }
                self.transcript_lines.extend(cell_transcript.clone());
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
            AppEvent::OpenFilePath { path } => {
                self.open_file_path(tui, path)?;
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
                self.overlay = Some(Overlay::new_static_with_title(
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
            AppEvent::PersistModelSelection { model, effort } => {
                let profile = self.active_profile.as_deref();
                match persist_model_selection(&self.config.codex_home, profile, &model, effort)
                    .await
                {
                    Ok(()) => {
                        if let Some(profile) = profile {
                            self.chat_widget.add_info_message(
                                format!("Model changed to {model} for {profile} profile"),
                                None,
                            );
                        } else {
                            self.chat_widget
                                .add_info_message(format!("Model changed to {model}"), None);
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
        }
        Ok(true)
    }
    fn on_stellar_event(&mut self, event: KernelEvent) {
        match event {
            KernelEvent::Info { message } => self.chat_widget.add_info_message(message, None),
            KernelEvent::Submission { text } => self
                .chat_widget
                .add_info_message(format!("Stellar insight submitted: {text}"), None),
            KernelEvent::CacheStored { key } => self
                .chat_widget
                .add_info_message(format!("Resilience cache updated: {key}"), None),
            KernelEvent::ConflictResolution { conflict_id, state } => {
                self.chat_widget.add_info_message(
                    format!("Conflict {conflict_id} resolved as {:?}", state),
                    None,
                )
            }
        }
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
            .show_mcp_manager(entries, state.template_count, self.config.cwd.clone());
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

        self.chat_widget.show_mcp_wizard(
            catalog,
            Some(draft),
            existing_name,
            self.config.cwd.clone(),
        );
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
                self.chat_widget.show_mcp_wizard(
                    catalog,
                    Some(retry_draft),
                    existing_name,
                    self.config.cwd.clone(),
                );
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

    fn open_file_path(&mut self, tui: &mut tui::Tui, path: PathBuf) -> Result<()> {
        let metadata = match fs::metadata(&path) {
            Ok(meta) => meta,
            Err(err) => {
                self.chat_widget.add_error_message(format!(
                    "Failed to open {}: {err}",
                    path.display()
                ));
                return Ok(());
            }
        };

        if metadata.is_dir() {
            // Directory navigation is handled by the explorer pane state.
            return Ok(());
        }

        match Self::preview_lines_for_path(&path) {
            Ok(lines) => {
                let title = path
                    .strip_prefix(&self.config.cwd)
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|_| path.display().to_string());
                self.overlay = Some(Overlay::new_static_with_title(lines, title));
                tui.frame_requester().schedule_frame();
            }
            Err(err) => {
                self.chat_widget.add_error_message(format!(
                    "Failed to preview {}: {err}",
                    path.display()
                ));
            }
        }

        Ok(())
    }

    fn preview_lines_for_path(path: &Path) -> io::Result<Vec<Line<'static>>> {
        let mut file = fs::File::open(path)?;
        let mut buffer = Vec::with_capacity(FILE_PREVIEW_LIMIT_BYTES + 1);
        {
            let mut limited = file.by_ref().take((FILE_PREVIEW_LIMIT_BYTES + 1) as u64);
            limited.read_to_end(&mut buffer)?;
        }

        let truncated = if buffer.len() > FILE_PREVIEW_LIMIT_BYTES {
            buffer.truncate(FILE_PREVIEW_LIMIT_BYTES);
            true
        } else {
            let mut probe = [0u8; 1];
            file.read(&mut probe)? > 0
        };

        if buffer.is_empty() {
            let mut lines = Vec::new();
            lines.push(Line::from("(empty file)".to_string()).dim());
            return Ok(lines);
        }

        match String::from_utf8(buffer) {
            Ok(text) => {
                let mut lines: Vec<Line<'static>> = text
                    .lines()
                    .map(|line| {
                        let cleaned = line.trim_end_matches('\r').to_string();
                        Line::from(cleaned)
                    })
                    .collect();
                if text.ends_with('\n') {
                    lines.push(Line::from(String::new()));
                }
                if lines.is_empty() {
                    lines.push(Line::from("(empty file)".to_string()).dim());
                }
                if truncated {
                    lines.push(Self::truncated_line());
                }
                Ok(lines)
            }
            Err(_) => {
                let mut lines = vec![Line::from("Binary preview not available".to_string()).italic().dim()];
                if truncated {
                    lines.push(Self::truncated_line());
                }
                Ok(lines)
            }
        }
    }

    fn truncated_line() -> Line<'static> {
        Line::from(format!(
            "… (preview truncated at {} KB)",
            FILE_PREVIEW_LIMIT_BYTES / 1024
        ))
        .dim()
    }

    fn explorer_panel_width(&self, total_width: u16) -> u16 {
        const MIN_TOTAL: u16 = 70;
        const MIN_WIDTH: u16 = 22;
        const MAX_WIDTH: u16 = 42;
        const MIN_CHAT_WIDTH: u16 = 48;

        if total_width < MIN_TOTAL {
            return 0;
        }

        let max_allowed = total_width.saturating_sub(MIN_CHAT_WIDTH);
        if max_allowed < MIN_WIDTH {
            return 0;
        }

        let ideal = (total_width as f32 * 0.28).round() as u16;
        ideal.clamp(MIN_WIDTH, MAX_WIDTH).min(max_allowed)
    }

    fn render_separator(area: ratatui::layout::Rect, buf: &mut ratatui::buffer::Buffer) {
        if area.width == 0 {
            return;
        }
        for y in area.y..area.bottom() {
            buf.set_stringn(area.x, y, "│", 1, Style::default().fg(Color::DarkGray));
        }
    }

    fn focus_chat(&mut self) {
        if self.focus != FocusTarget::Chat {
            self.focus = FocusTarget::Chat;
            self.chat_widget.set_input_focus(true);
        }
    }

    fn focus_explorer(&mut self) {
        if self.focus != FocusTarget::Explorer {
            self.focus = FocusTarget::Explorer;
            self.chat_widget.set_input_focus(false);
        }
    }

    fn handle_file_explorer_key(&mut self, tui: &mut tui::Tui, key_event: &KeyEvent) -> bool {
        if matches!(key_event.code, KeyCode::F(2))
            && matches!(key_event.kind, KeyEventKind::Press)
        {
            if self.focus == FocusTarget::Explorer {
                self.focus_chat();
            } else {
                self.focus_explorer();
            }
            tui.frame_requester().schedule_frame();
            return true;
        }

        if self.focus != FocusTarget::Explorer {
            return false;
        }

        match key_event.code {
            KeyCode::Esc | KeyCode::Tab => {
                if matches!(key_event.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
                    self.focus_chat();
                    tui.frame_requester().schedule_frame();
                    return true;
                }
            }
            KeyCode::Up => {
                if matches!(key_event.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
                    self.file_explorer.move_selection(-1);
                    tui.frame_requester().schedule_frame();
                    return true;
                }
            }
            KeyCode::Down => {
                if matches!(key_event.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
                    self.file_explorer.move_selection(1);
                    tui.frame_requester().schedule_frame();
                    return true;
                }
            }
            KeyCode::PageUp => {
                if matches!(key_event.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
                    self.file_explorer.move_selection(-10);
                    tui.frame_requester().schedule_frame();
                    return true;
                }
            }
            KeyCode::PageDown => {
                if matches!(key_event.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
                    self.file_explorer.move_selection(10);
                    tui.frame_requester().schedule_frame();
                    return true;
                }
            }
            KeyCode::Home => {
                if matches!(key_event.kind, KeyEventKind::Press) {
                    self.file_explorer.select_first();
                    tui.frame_requester().schedule_frame();
                    return true;
                }
            }
            KeyCode::End => {
                if matches!(key_event.kind, KeyEventKind::Press) {
                    self.file_explorer.select_last();
                    tui.frame_requester().schedule_frame();
                    return true;
                }
            }
            KeyCode::Left => {
                if matches!(key_event.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
                    if let Err(err) = self.file_explorer.collapse_selected() {
                        warn!(error = %err, "file explorer collapse failed");
                    }
                    tui.frame_requester().schedule_frame();
                    return true;
                }
            }
            KeyCode::Right => {
                if matches!(key_event.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
                    if let Some(entry) = self.file_explorer.selected_entry().cloned() {
                        if entry.is_placeholder {
                            // no-op placeholder row (ellipsis)
                        } else if entry.is_dir {
                            if entry.is_expanded {
                                let idx = self.file_explorer.selected_index();
                                if let Some(next) = self
                                    .file_explorer
                                    .visible_items()
                                    .get(idx.saturating_add(1))
                                {
                                    if next.depth > entry.depth {
                                        self.file_explorer.move_selection(1);
                                    }
                                }
                            } else if let Err(err) = self.file_explorer.expand_selected() {
                                warn!(error = %err, "file explorer expand failed");
                            }
                        } else {
                            self.open_selected_path_from_explorer(tui);
                        }
                    }
                    tui.frame_requester().schedule_frame();
                    return true;
                }
            }
            KeyCode::Char(' ') => {
                if matches!(key_event.kind, KeyEventKind::Press) {
                    if let Err(err) = self.file_explorer.toggle_expanded() {
                        warn!(error = %err, "file explorer toggle failed");
                    }
                    tui.frame_requester().schedule_frame();
                    return true;
                }
            }
            KeyCode::Enter => {
                if matches!(key_event.kind, KeyEventKind::Press) {
                    if let Some(entry) = self.file_explorer.selected_entry().cloned() {
                        if entry.is_placeholder {
                            // ignore placeholder rows
                        } else if entry.is_dir {
                            if let Err(err) = self.file_explorer.toggle_expanded() {
                                warn!(error = %err, "file explorer toggle failed");
                            }
                        } else {
                            self.open_selected_path_from_explorer(tui);
                        }
                    }
                    tui.frame_requester().schedule_frame();
                    return true;
                }
            }
            KeyCode::Char('r') => {
                if key_event.modifiers.contains(KeyModifiers::CONTROL)
                    && matches!(key_event.kind, KeyEventKind::Press)
                {
                    if let Err(err) = self.file_explorer.refresh() {
                        warn!(error = %err, "file explorer refresh failed");
                    }
                    tui.frame_requester().schedule_frame();
                    return true;
                }
            }
            _ => {}
        }

        false
    }

    fn open_selected_path_from_explorer(&mut self, tui: &mut tui::Tui) {
        if let Some(path) = self.file_explorer.selected_path() {
            if let Err(err) = self.open_file_path(tui, path) {
                self.chat_widget
                    .add_error_message(format!("Failed to open file: {err}"));
            }
        }
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
        if matches!(
            key_event,
            KeyEvent {
                code: KeyCode::Char('?'),
                modifiers,
                kind: KeyEventKind::Press | KeyEventKind::Repeat,
                ..
            } if modifiers.contains(KeyModifiers::CONTROL)
        ) {
            self.open_quickstart_overlay(tui);
            return;
        }

        if self.handle_file_explorer_key(tui, &key_event) {
            return;
        }

        let mut route_to_stellar = self.stellar.is_active();
        if !route_to_stellar {
            route_to_stellar = matches!(key_event,
                KeyEvent {
                    code: KeyCode::Char('i'),
                    modifiers,
                    kind: KeyEventKind::Press | KeyEventKind::Repeat,
                    ..
                } if modifiers.contains(KeyModifiers::CONTROL) || modifiers.contains(KeyModifiers::ALT)
            ) || matches!(key_event,
                KeyEvent {
                    code: KeyCode::Char('a'),
                    modifiers,
                    kind: KeyEventKind::Press | KeyEventKind::Repeat,
                    ..
                } if modifiers == (KeyModifiers::CONTROL | KeyModifiers::ALT)
            ) || matches!(key_event,
                KeyEvent {
                    code: KeyCode::Char('c'),
                    modifiers,
                    kind: KeyEventKind::Press | KeyEventKind::Repeat,
                    ..
                } if modifiers.contains(KeyModifiers::CONTROL) && modifiers.contains(KeyModifiers::SHIFT)
            );
        }

        if route_to_stellar {
            match self.stellar.handle_key_event(key_event) {
                ControllerOutcome::Consumed { applied, .. } => {
                    for event in self.stellar.take_events() {
                        self.on_stellar_event(event);
                    }
                    if matches!(applied, ActionApplied::StateChanged) {
                        tui.frame_requester().schedule_frame();
                    }
                    return;
                }
                ControllerOutcome::Rejected { error, .. } => {
                    self.chat_widget
                        .add_error_message(format!("Stellar guard: {error}"));
                    tui.frame_requester().schedule_frame();
                    return;
                }
                ControllerOutcome::Unhandled => {}
            }
        }

        match key_event {
            KeyEvent {
                code: KeyCode::Char('t'),
                modifiers: KeyModifiers::CONTROL,
                kind: KeyEventKind::Press,
                ..
            } => {
                // Enter alternate screen and set viewport to full size.
                let _ = tui.enter_alt_screen();
                self.overlay = Some(Overlay::new_transcript(self.transcript_lines.clone()));
                tui.frame_requester().schedule_frame();
            }
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
            KeyEvent {
                code: KeyCode::Enter,
                kind: KeyEventKind::Press,
                ..
            } if self.backtrack.primed
                && self.backtrack.count > 0
                && self.chat_widget.composer_is_empty() =>
            {
                self.confirm_backtrack_from_main();
            }
            KeyEvent {
                kind: KeyEventKind::Press | KeyEventKind::Repeat,
                ..
            } => {
                if key_event.code != KeyCode::Esc && self.backtrack.primed {
                    self.reset_backtrack_state();
                }
                self.chat_widget.handle_key_event(key_event);
            }
            _ => {}
        }
    }

    fn open_quickstart_overlay(&mut self, tui: &mut tui::Tui) {
        let lines = quickstart_overlay_lines(self.stellar.persona());
        let _ = tui.enter_alt_screen();
        self.overlay = Some(Overlay::new_static_with_title(
            lines,
            "Q U I C K S T A R T".to_string(),
        ));
        tui.frame_requester().schedule_frame();
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

fn quickstart_overlay_lines(persona: StellarPersona) -> Vec<Line<'static>> {
    let guide = build_quickstart(QuickstartInput { persona });
    let mut lines: Vec<Line<'static>> = Vec::new();
    lines.push(guide.headline.bold().into());
    for section in guide.sections {
        lines.push(Line::from(""));
        lines.push(section.title.bold().into());
        for bullet in section.bullets {
            lines.push(Line::from(format!("  • {bullet}")));
        }
    }
    if !guide.recommended_commands.is_empty() {
        lines.push(Line::from(""));
        lines.push("Recommended commands:".bold().into());
        for cmd in guide.recommended_commands {
            lines.push(Line::from(format!("  $ {cmd}")));
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_backtrack::BacktrackState;
    use crate::chatwidget::tests::make_chatwidget_manual_with_sender;
    use crate::file_explorer::FileExplorerState;
    use crate::file_search::FileSearchManager;
    use codex_core::AuthManager;
    use codex_core::CodexAuth;
    use codex_core::ConversationManager;
    use codex_core::stellar::StellarPersona;
    use ratatui::text::Line;
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
        let file_explorer = FileExplorerState::new(config.cwd.clone());

        App {
            server,
            app_event_tx,
            chat_widget,
            auth_manager,
            config,
            active_profile: None,
            file_search,
            file_explorer,
            transcript_lines: Vec::<Line<'static>>::new(),
            overlay: None,
            deferred_history_lines: Vec::new(),
            has_emitted_history_lines: false,
            enhanced_keys_supported: false,
            commit_anim_running: Arc::new(AtomicBool::new(false)),
            backtrack: BacktrackState::default(),
            stellar: StellarController::new(StellarPersona::Operator),
            focus: FocusTarget::Chat,
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
    fn quickstart_overlay_contains_pipeline_hints() {
        let lines = quickstart_overlay_lines(StellarPersona::Operator);
        let rendered: Vec<String> = lines.iter().map(|line| line.to_string()).collect();
        assert!(rendered.iter().any(|l| l.contains("pipeline sign")));
        assert!(rendered.iter().any(|l| l.contains("orchestrator feedback")));
        assert!(rendered.iter().any(|l| l.contains("orchestrator triage")));
    }
}

// Observability Mesh bootstrap (REQ-OBS-01, REQ-OPS-01; MaxThink-Stellar.md).
fn setup_telemetry_exporter(config: &Config) {
    let mut exporter_cfg = telemetry::exporter_config(&config.codex_home);

    if let Ok(endpoint) = env::var("CODEX_TELEMETRY_OTLP_ENDPOINT") {
        let trimmed = endpoint.trim();
        if !trimmed.is_empty() {
            exporter_cfg = exporter_cfg.with_otlp_endpoint(trimmed.to_string());
            if let Ok(headers) = env::var("CODEX_TELEMETRY_OTLP_HEADERS") {
                for raw in headers.split(',') {
                    if let Some((key, value)) = raw.split_once('=') {
                        exporter_cfg = exporter_cfg
                            .with_otlp_header(key.trim().to_string(), value.trim().to_string());
                    }
                }
            }
        }
    }

    if let Ok(addr) = env::var("CODEX_TELEMETRY_PROMETHEUS_ADDR") {
        match addr.parse::<SocketAddr>() {
            Ok(socket) => {
                exporter_cfg = exporter_cfg.with_prometheus_bind(socket);
            }
            Err(err) => {
                warn!(%addr, %err, "invalid CODEX_TELEMETRY_PROMETHEUS_ADDR");
            }
        }
    }

    match TelemetryExporter::new(exporter_cfg) {
        Ok(exporter) => {
            if let Err(err) = telemetry::install_exporter(exporter) {
                if !matches!(err, TelemetryInstallError::AlreadyInstalled) {
                    warn!("failed to install telemetry exporter: {err}");
                }
            }
        }
        Err(err) => {
            warn!("failed to initialize telemetry exporter: {err}");
        }
    }
}
