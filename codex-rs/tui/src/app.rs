use crate::app_backtrack::BacktrackState;
use crate::app_event::AppEvent;
use crate::app_event_sender::AppEventSender;
use crate::chatwidget::ChatWidget;
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
use ratatui::style::Stylize;
use ratatui::text::Line;
use std::collections::BTreeMap;
use std::env;
use std::net::SocketAddr;
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

pub(crate) struct App {
    pub(crate) server: Arc<ConversationManager>,
    pub(crate) app_event_tx: AppEventSender,
    pub(crate) chat_widget: ChatWidget,
    pub(crate) auth_manager: Arc<AuthManager>,

    /// Config is stored here so we can recreate ChatWidgets as needed.
    pub(crate) config: Config,
    pub(crate) active_profile: Option<String>,

    pub(crate) file_search: FileSearchManager,

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

        let mut app = Self {
            server: conversation_manager,
            app_event_tx,
            chat_widget,
            auth_manager: auth_manager.clone(),
            config,
            active_profile,
            file_search,
            enhanced_keys_supported,
            transcript_lines: Vec::new(),
            overlay: None,
            deferred_history_lines: Vec::new(),
            has_emitted_history_lines: false,
            commit_anim_running: Arc::new(AtomicBool::new(false)),
            backtrack: BacktrackState::default(),
            stellar: StellarController::new(StellarPersona::Operator),
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
                    let stellar_active = self.stellar.is_active();
                    if stellar_active {
                        self.stellar.sync_layout(width);
                    }
                    let stellar_snapshot = stellar_active.then(|| self.stellar.snapshot());
                    let stellar_height = if stellar_active {
                        self.stellar.preferred_height().min(terminal_size.height)
                    } else {
                        0
                    };
                    let desired_height = self.chat_widget.desired_height(width) + stellar_height;
                    tui.draw(desired_height, |frame| {
                        let area = frame.area();
                        if let Some(snapshot) = stellar_snapshot.as_ref() {
                            let layout = ratatui::layout::Layout::default()
                                .direction(ratatui::layout::Direction::Vertical)
                                .constraints([
                                    ratatui::layout::Constraint::Length(
                                        stellar_height.min(area.height),
                                    ),
                                    ratatui::layout::Constraint::Min(1),
                                ])
                                .split(area);
                            frame.render_widget(StellarView::new(snapshot), layout[0]);
                            frame.render_widget_ref(&self.chat_widget, layout[1]);
                            if let Some((x, y)) = self.chat_widget.cursor_pos(layout[1]) {
                                frame.set_cursor_position((x, y));
                            }
                        } else {
                            frame.render_widget_ref(&self.chat_widget, area);
                            if let Some((x, y)) = self.chat_widget.cursor_pos(area) {
                                frame.set_cursor_position((x, y));
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
        if matches!(
            key_event,
            KeyEvent {
                code: KeyCode::Char('?'),
                kind: KeyEventKind::Press | KeyEventKind::Repeat,
                ..
            }
        ) {
            self.open_quickstart_overlay(tui);
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

        App {
            server,
            app_event_tx,
            chat_widget,
            auth_manager,
            config,
            active_profile: None,
            file_search,
            transcript_lines: Vec::<Line<'static>>::new(),
            overlay: None,
            deferred_history_lines: Vec::new(),
            has_emitted_history_lines: false,
            enhanced_keys_supported: false,
            commit_anim_running: Arc::new(AtomicBool::new(false)),
            backtrack: BacktrackState::default(),
            stellar: StellarController::new(StellarPersona::Operator),
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
