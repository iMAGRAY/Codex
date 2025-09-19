use crate::AppServiceHandle;
use crate::HistoryItem;
use crate::MessageRole;
use crate::PromptPayload;
use crate::SessionDescriptor;
use crate::SessionId;
use crate::SessionRequest;
use crate::SessionStream;
use crate::backend::{BackendEvent, HistoryEvent, StatusEvent};
use eframe::App;
use eframe::egui;
use eframe::egui::{Color32, FontId, Margin, RichText, Vec2, Visuals};
use egui::epaint::Shadow;
use egui::{
    Align, Context, Frame, KeyboardShortcut, Layout, Modifiers, ScrollArea, Sense, TextStyle,
    ViewportCommand,
};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

const SEND_SHORTCUT: KeyboardShortcut = KeyboardShortcut {
    modifiers: Modifiers::COMMAND,
    logical_key: egui::Key::Enter,
};

pub struct DesktopShell {
    service: AppServiceHandle,
    sessions: Vec<SessionDescriptor>,
    active_session: Option<SessionId>,
    history: Vec<HistoryItem>,
    composer: String,
    stream: Option<SessionStream>,
    last_error: Option<String>,
    status_banner: Option<String>,
}

impl DesktopShell {
    pub fn new(ctx: &Context, service: AppServiceHandle) -> Self {
        configure_visuals(ctx);
        let mut last_error = None;
        let sessions = match service.list_sessions() {
            Ok(list) => list,
            Err(err) => {
                let message = format!("Не удалось получить список сессий: {err}");
                tracing::error!("{message}");
                last_error = Some(message);
                Vec::new()
            }
        };
        let mut shell = Self {
            service: service.clone(),
            sessions,
            active_session: None,
            history: Vec::new(),
            composer: String::new(),
            stream: None,
            last_error,
            status_banner: None,
        };
        shell
            .sessions
            .sort_by(|a, b| a.created_at.cmp(&b.created_at));
        if shell.sessions.is_empty() {
            match shell.service.spawn_session(SessionRequest::default()) {
                Ok(descriptor) => {
                    if let Err(err) = shell.append_session(descriptor) {
                        shell.last_error = Some(err);
                    }
                }
                Err(err) => {
                    let message = format!("Не удалось создать сессию: {err}");
                    tracing::error!("{message}");
                    shell.last_error = Some(message);
                }
            }
        } else if let Some(first) = shell.sessions.last() {
            shell.activate_session(first.id.clone());
        }
        shell
    }

    fn activate_session(&mut self, session_id: SessionId) {
        if let Err(err) = self.select_session(&session_id) {
            self.last_error = Some(err);
        }
    }

    fn refresh_stream(&mut self) -> bool {
        let mut drained = Vec::new();
        if let Some(stream) = self.stream.as_ref() {
            while let Some(event) = stream.try_next() {
                drained.push(event);
            }
        }

        let updated = !drained.is_empty();
        for event in drained {
            match event {
                BackendEvent::History(history_event) => self.apply_history_event(history_event),
                BackendEvent::Status(status) => self.apply_status_event(status),
                BackendEvent::Error(message) => self.last_error = Some(message),
            }
        }

        updated
    }

    fn apply_history_event(&mut self, event: HistoryEvent) {
        match event {
            HistoryEvent::Snapshot(items) => {
                self.history = items;
            }
            HistoryEvent::Append(item) => {
                self.history.push(item);
            }
            HistoryEvent::Update { id, content } => {
                if let Some(existing) = self.history.iter_mut().find(|item| item.id == id) {
                    existing.content = content;
                }
            }
            HistoryEvent::Delta { id, chunk } => {
                if let Some(existing) = self.history.iter_mut().find(|item| item.id == id) {
                    existing.content.push_str(&chunk);
                }
            }
        }
    }

    fn select_session(&mut self, session_id: &SessionId) -> Result<(), String> {
        let history = self
            .service
            .list_history(session_id)
            .map_err(|err| err.to_string())?;
        self.history = history;
        self.active_session = Some(session_id.clone());
        self.stream = Some(
            self.service
                .subscribe(session_id)
                .map_err(|err| err.to_string())?,
        );
        Ok(())
    }

    fn append_session(&mut self, descriptor: SessionDescriptor) -> Result<(), String> {
        self.sessions.push(descriptor.clone());
        self.sessions
            .sort_by(|a, b| a.created_at.cmp(&b.created_at));
        self.select_session(&descriptor.id)
    }

    fn apply_status_event(&mut self, status: StatusEvent) {
        self.status_banner = Some(match status {
            StatusEvent::Info(message) => message,
            StatusEvent::TaskStarted => "Задача выполняется…".to_owned(),
            StatusEvent::TaskComplete => "Задача завершена".to_owned(),
        });
    }

    fn render_sidebar(&mut self, ui: &mut egui::Ui) {
        ui.heading(
            RichText::new("Sessions")
                .size(16.0)
                .color(Color32::from_rgb(20, 24, 31)),
        );
        ui.add_space(8.0);
        let sessions_snapshot = self.sessions.clone();
        for session in sessions_snapshot {
            let is_active = self
                .active_session
                .as_ref()
                .map(|id| id == &session.id)
                .unwrap_or(false);
            let frame = Frame::none()
                .fill(if is_active {
                    Color32::from_rgb(235, 239, 245)
                } else {
                    Color32::from_rgb(250, 251, 253)
                })
                .rounding(egui::Rounding::same(12.0))
                .inner_margin(Margin::symmetric(12.0, 10.0));
            let response = frame
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(RichText::new(&session.title).strong());
                            ui.label(
                                RichText::new(format_time(session.created_at))
                                    .color(Color32::from_gray(130))
                                    .italics(),
                            );
                        });
                    });
                })
                .response
                .interact(Sense::click());
            if response.clicked() {
                if let Err(err) = self.select_session(&session.id) {
                    self.last_error = Some(err);
                }
            }
            ui.add_space(6.0);
        }
        if ui.button("+ Новая сессия").clicked() {
            match self.service.spawn_session(Default::default()) {
                Ok(descriptor) => {
                    self.sessions.push(descriptor.clone());
                    self.sessions
                        .sort_by(|a, b| a.created_at.cmp(&b.created_at));
                    self.activate_session(descriptor.id);
                }
                Err(err) => {
                    self.last_error = Some(err.to_string());
                }
            }
        }
    }

    fn render_history(&mut self, ui: &mut egui::Ui) {
        ScrollArea::vertical()
            .auto_shrink([false; 2])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for item in &self.history {
                    self.render_message_card(ui, item);
                    ui.add_space(10.0);
                }
            });
    }

    fn render_message_card(&self, ui: &mut egui::Ui, item: &HistoryItem) {
        let (fill, text_color) = match item.role {
            MessageRole::System => (Color32::from_rgb(244, 246, 251), Color32::BLACK),
            MessageRole::User => (
                Color32::from_rgb(255, 255, 255),
                Color32::from_rgb(30, 33, 41),
            ),
            MessageRole::Assistant => (
                Color32::from_rgb(241, 247, 255),
                Color32::from_rgb(28, 30, 38),
            ),
        };
        Frame::none()
            .fill(fill)
            .rounding(egui::Rounding::same(16.0))
            .inner_margin(Margin::symmetric(18.0, 14.0))
            .stroke(egui::Stroke::new(1.0, Color32::from_rgb(232, 235, 241)))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = Vec2::new(0.0, 6.0);
                let header = match item.role {
                    MessageRole::System => "Assistant",
                    MessageRole::User => "Вы",
                    MessageRole::Assistant => "Codex",
                };
                ui.label(
                    RichText::new(header)
                        .font(FontId::proportional(14.0))
                        .color(Color32::from_gray(90)),
                );
                ui.label(
                    RichText::new(&item.content)
                        .font(FontId::proportional(16.0))
                        .color(text_color),
                );
                ui.label(
                    RichText::new(format_time(item.created_at))
                        .size(12.0)
                        .color(Color32::from_gray(140)),
                );
            });
    }

    fn render_composer(&mut self, ui: &mut egui::Ui) {
        Frame::none()
            .fill(Color32::from_rgb(248, 250, 255))
            .rounding(egui::Rounding::same(18.0))
            .inner_margin(Margin::same(16.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.set_min_height(120.0);
                    let text_edit = egui::TextEdit::multiline(&mut self.composer)
                        .hint_text("Опиши задачу или вставь код — Ctrl/⌘ + Enter чтобы отправить")
                        .desired_rows(4)
                        .frame(false);
                    ui.add_sized(ui.available_size() - Vec2::new(120.0, 0.0), text_edit);
                    ui.add_space(12.0);
                    let send_button =
                        ui.add_sized(Vec2::new(96.0, 40.0), egui::Button::new("Отправить"));
                    if send_button.clicked() {
                        self.submit_prompt();
                    }
                });
            });

        if ui.input_mut(|i| i.consume_shortcut(&SEND_SHORTCUT)) {
            self.submit_prompt();
        }
        ui.label(
            RichText::new("Быстрая клавиша: ⌘/Ctrl + Enter")
                .size(12.0)
                .color(Color32::from_gray(130)),
        );
    }

    fn submit_prompt(&mut self) {
        if self.composer.trim().is_empty() {
            return;
        }
        if let Some(session_id) = self.active_session.clone() {
            let payload = PromptPayload {
                text: self.composer.trim().to_owned(),
            };
            if let Err(err) = self.service.send_prompt(&session_id, payload) {
                self.last_error = Some(err.to_string());
            } else {
                self.composer.clear();
            }
        }
    }

    fn render_error(&mut self, ui: &mut egui::Ui) {
        if let Some(err) = self.last_error.clone() {
            Frame::none()
                .fill(Color32::from_rgb(255, 245, 245))
                .rounding(egui::Rounding::same(12.0))
                .stroke(egui::Stroke::new(1.0, Color32::from_rgb(254, 205, 211)))
                .inner_margin(Margin::same(12.0))
                .show(ui, |ui| {
                    ui.label(RichText::new(&err).color(Color32::from_rgb(200, 40, 60)));
                    if ui.button("Скрыть").clicked() {
                        self.last_error = None;
                    }
                });
        }
    }

    fn render_status(&mut self, ui: &mut egui::Ui) {
        if let Some(status) = self.status_banner.clone() {
            Frame::none()
                .fill(Color32::from_rgb(242, 248, 255))
                .rounding(egui::Rounding::same(12.0))
                .stroke(egui::Stroke::new(1.0, Color32::from_rgb(210, 226, 255)))
                .inner_margin(Margin::same(12.0))
                .show(ui, |ui| {
                    ui.label(
                        RichText::new(&status)
                            .color(Color32::from_rgb(40, 80, 160))
                            .strong(),
                    );
                    if ui.button("Скрыть").clicked() {
                        self.status_banner = None;
                    }
                });
        }
    }
}

impl App for DesktopShell {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        if self.refresh_stream() {
            ctx.request_repaint();
        }
        egui::TopBottomPanel::top("top_bar")
            .frame(
                Frame::none()
                    .fill(Color32::from_rgb(255, 255, 255))
                    .inner_margin(Margin::symmetric(18.0, 10.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.heading(
                        RichText::new("Codex Desktop")
                            .size(22.0)
                            .color(Color32::from_rgb(20, 24, 31)),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("Закрыть").clicked() {
                            ctx.send_viewport_cmd(ViewportCommand::Close);
                        }
                    });
                });
            });

        egui::SidePanel::left("sidebar")
            .frame(
                Frame::none()
                    .fill(Color32::from_rgb(245, 247, 252))
                    .inner_margin(Margin::same(18.0)),
            )
            .exact_width(240.0)
            .show(ctx, |ui| {
                self.render_sidebar(ui);
            });

        egui::CentralPanel::default()
            .frame(
                Frame::none()
                    .fill(Color32::from_rgb(252, 253, 255))
                    .inner_margin(Margin::symmetric(24.0, 16.0)),
            )
            .show(ctx, |ui| {
                ui.set_min_height(300.0);
                self.render_error(ui);
                self.render_status(ui);
                ui.add_space(12.0);
                self.render_history(ui);
            });

        egui::TopBottomPanel::bottom("composer")
            .frame(
                Frame::none()
                    .fill(Color32::from_rgba_unmultiplied(248, 250, 255, 245))
                    .inner_margin(Margin::symmetric(24.0, 12.0)),
            )
            .show(ctx, |ui| {
                self.render_composer(ui);
            });
    }
}

fn configure_visuals(ctx: &Context) {
    let mut visuals = Visuals::light();
    visuals.override_text_color = Some(Color32::from_rgb(33, 37, 43));
    visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(252, 253, 255);
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(248, 250, 255);
    visuals.window_fill = Color32::from_rgb(252, 253, 255);
    visuals.window_rounding = egui::Rounding::same(20.0);
    visuals.popup_shadow = Shadow {
        offset: Vec2::new(0.0, 6.0),
        blur: 16.0,
        spread: 0.0,
        color: Color32::from_rgba_unmultiplied(0, 0, 0, 25),
    };
    ctx.set_visuals(visuals);
    ctx.set_pixels_per_point(1.1);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = Vec2::new(12.0, 14.0);
    style.spacing.window_margin = Margin::same(12.0);
    style
        .text_styles
        .insert(TextStyle::Heading, FontId::proportional(24.0));
    style
        .text_styles
        .insert(TextStyle::Body, FontId::proportional(16.0));
    ctx.set_style(style);
}

fn format_time(time: OffsetDateTime) -> String {
    time.format(&Rfc3339).unwrap_or_else(|_| time.to_string())
}
