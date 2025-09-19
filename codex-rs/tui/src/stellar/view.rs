use crate::status_bar::telemetry_spans;
use codex_core::stellar::GoldenPathHint;
use codex_core::stellar::KernelSnapshot;
use codex_core::stellar::LayoutMode;
use codex_core::stellar::PaneFocus;
use codex_core::stellar::RiskSeverity;
use codex_core::stellar::StellarPersona;
use codex_core::telemetry::TelemetryHub;
use ratatui::buffer::Buffer;
use ratatui::layout::Constraint;
use ratatui::layout::Direction;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Block;
use ratatui::widgets::Borders;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Widget;
use ratatui::widgets::WidgetRef;
use ratatui::widgets::Wrap;

pub struct StellarView<'a> {
    snapshot: &'a KernelSnapshot,
}

impl<'a> StellarView<'a> {
    pub fn new(snapshot: &'a KernelSnapshot) -> Self {
        Self { snapshot }
    }

    fn render_wide(&self, area: Rect, buf: &mut Buffer) {
        if area.height <= 3 || area.width <= 20 {
            return;
        }
        let vertical = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(area.height.saturating_sub(4)),
                Constraint::Length(3),
            ])
            .split(area);

        let main = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
            .split(vertical[0]);

        self.render_canvas(main[0], buf);
        self.render_telemetry(main[1], buf);

        let footer = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
            .split(vertical[1]);

        self.render_command_log(footer[0], buf);
        self.render_golden_path(footer[1], buf);
    }

    fn render_compact(&self, area: Rect, buf: &mut Buffer) {
        if area.height <= 6 {
            return;
        }
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(area.height.saturating_sub(6)),
                Constraint::Length(2),
                Constraint::Length(2),
                Constraint::Length(2),
            ])
            .split(area);

        self.render_canvas(layout[0], buf);
        self.render_tab_strip(layout[1], buf);
        self.render_command_log(layout[2], buf);
        self.render_golden_path(layout[3], buf);
    }

    fn render_canvas(&self, area: Rect, buf: &mut Buffer) {
        let focus = self.snapshot.focus == PaneFocus::InsightCanvas;
        let mut block = styled_block("Insight Canvas", focus);
        if self.snapshot.field_locked {
            block = block.title(" Insight Canvas (locked) ");
        }
        block.render_ref(area, buf);
        if area.height <= 2 {
            return;
        }
        let inner = block.inner(area);
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Min(inner.height.saturating_sub(6)),
            ])
            .split(inner);

        let persona = format!(
            "Persona: {}{}",
            self.snapshot.persona,
            if self.snapshot.assistive_mode {
                " (assistive)"
            } else {
                ""
            }
        );
        let status_line = Paragraph::new(Line::from(persona.cyan()));
        status_line.render_ref(chunks[0], buf);

        let input = if self.snapshot.field_text.is_empty() {
            "<type an insight or use CLI bridge>".italic()
        } else {
            self.snapshot.field_text.clone().into()
        };
        Paragraph::new(input)
            .wrap(Wrap { trim: true })
            .render_ref(chunks[1], buf);

        self.render_suggestions(chunks[2], buf);
    }

    fn render_suggestions(&self, area: Rect, buf: &mut Buffer) {
        let mut lines = Vec::new();
        if !self.snapshot.risk_alerts.is_empty() {
            for alert in &self.snapshot.risk_alerts {
                let styled = match alert.severity {
                    RiskSeverity::Info => format!("ℹ {}", alert.message).blue(),
                    RiskSeverity::Warning => format!("⚠ {}", alert.message).yellow().bold(),
                    RiskSeverity::Critical => format!("⛔ {}", alert.message).red().bold(),
                };
                lines.push(Line::from(styled));
            }
            lines.push(Line::from(Span::raw("")));
        }
        if let Some(confidence) = &self.snapshot.confidence {
            if confidence.visible {
                lines.push(Line::from(vec![
                    Span::from(format!(
                        "Confidence {:>3}% - {}",
                        (confidence.score * 100.0).round() as i32,
                        confidence.trend
                    ))
                    .green(),
                ]));
                for reason in &confidence.reasons {
                    lines.push(prefix_bullet(reason));
                }
                lines.push(Line::from(Span::raw("")));
            }
        }
        for suggestion in &self.snapshot.suggestions {
            lines.push(prefix_bullet(suggestion));
        }
        if self.snapshot.status_messages.is_empty() {
            lines.push(Line::from("Press Ctrl+Enter to submit".dim()));
        } else {
            for status in &self.snapshot.status_messages {
                lines.push(Line::from(status.clone().yellow()));
            }
        }
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .render_ref(area, buf);
    }

    fn render_telemetry(&self, area: Rect, buf: &mut Buffer) {
        let focus = self.snapshot.focus == PaneFocus::Telemetry;
        let title = if self.snapshot.telemetry_visible {
            "Observability Overlay"
        } else {
            "Observability Overlay (hidden)"
        };
        let block = styled_block(title, focus);
        block.render_ref(area, buf);
        if area.height <= 2 {
            return;
        }
        let inner = block.inner(area);
        if self.snapshot.telemetry_visible {
            let snapshot = TelemetryHub::global().snapshot();
            let mut lines = Vec::new();
            lines.push(Line::from(telemetry_spans(snapshot)));
            lines.push(investigate_hint(
                self.snapshot.persona,
                self.snapshot.assistive_mode,
            ));
            lines.push(Line::from("Ctrl+O to hide overlay".cyan()));
            Paragraph::new(lines)
                .wrap(Wrap { trim: true })
                .render_ref(inner, buf);
        } else {
            let mut lines = Vec::new();
            lines.push(Line::from("Press Ctrl+O to reveal live metrics".cyan()));
            lines.push(investigate_hint(
                self.snapshot.persona,
                self.snapshot.assistive_mode,
            ));
            Paragraph::new(lines)
                .wrap(Wrap { trim: true })
                .render_ref(inner, buf);
        }
    }

    fn render_command_log(&self, area: Rect, buf: &mut Buffer) {
        let focus = self.snapshot.focus == PaneFocus::CommandLog;
        let block = styled_block("Command Log", focus);
        block.render_ref(area, buf);
        if area.height == 0 {
            return;
        }
        let inner = block.inner(area);
        let lines: Vec<_> = if self.snapshot.command_log.is_empty() {
            vec![Line::from("No insights submitted yet".dim())]
        } else {
            self.snapshot
                .command_log
                .iter()
                .rev()
                .take(inner.height as usize)
                .map(|entry| Line::from(entry.clone()))
                .collect()
        };
        Paragraph::new(lines).render_ref(inner, buf);
    }

    fn render_golden_path(&self, area: Rect, buf: &mut Buffer) {
        let focus = matches!(
            self.snapshot.focus,
            PaneFocus::FooterSubmit | PaneFocus::FooterOverlay
        );
        let block = styled_block("Golden Path", focus);
        block.render_ref(area, buf);
        let inner = block.inner(area);
        let mut lines = Vec::new();
        for hint in &self.snapshot.golden_path {
            lines.push(render_hint(hint));
        }
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .render_ref(inner, buf);
    }

    fn render_tab_strip(&self, area: Rect, buf: &mut Buffer) {
        let tabs = [
            ("Telemetry", PaneFocus::Telemetry),
            ("Command Log", PaneFocus::CommandLog),
            ("Runbook", PaneFocus::Runbook),
        ];
        let spans: Vec<Span> = tabs
            .into_iter()
            .map(|(label, focus)| {
                if self.snapshot.focus == focus {
                    Span::from(format!("[{label}] ")).bold()
                } else {
                    Span::from(format!(" {label} ")).dim()
                }
            })
            .collect();
        Paragraph::new(Line::from(spans)).render_ref(area, buf);
    }
}

impl<'a> Widget for StellarView<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        match self.snapshot.layout_mode {
            LayoutMode::Wide => self.render_wide(area, buf),
            LayoutMode::Compact => self.render_compact(area, buf),
        }
    }
}

fn investigate_hint(persona: StellarPersona, assistive_mode: bool) -> Line<'static> {
    let key = if assistive_mode { "F9" } else { "Ctrl+R" };
    let detail = match persona {
        StellarPersona::Operator => "guide the operator runbook",
        StellarPersona::Sre => "run SRE latency playbook RB-07",
        StellarPersona::SecOps => "review audit and policy evidence",
        StellarPersona::PlatformEngineer => "inspect deployment timeline",
        StellarPersona::PartnerDeveloper => "handoff to partner support flow",
        StellarPersona::AssistiveBridge => "narrate investigation with accessible summary",
    };
    Line::from(vec![
        Span::from("[ Investigate ]").cyan().bold(),
        Span::raw(" "),
        Span::from(format!("{key} - {detail}")),
    ])
}

fn styled_block<'a>(title: &'a str, focused: bool) -> Block<'a> {
    let style = if focused {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    Block::default()
        .title(Span::styled(format!(" {title} "), style))
        .borders(Borders::ALL)
        .border_style(style)
}

fn prefix_bullet(text: &str) -> Line<'_> {
    Line::from(vec![" - ".into(), Span::raw(text.to_string())])
}

fn render_hint(hint: &GoldenPathHint) -> Line<'_> {
    let mut spans = Vec::new();
    spans.push(Span::raw(hint.label.clone()).bold());
    if let Some(shortcut) = &hint.shortcut {
        spans.push(Span::raw(" "));
        spans.push(Span::from(shortcut.clone()).cyan());
    }
    spans.push(Span::raw(" — "));
    spans.push(Span::raw(hint.description.clone()));
    Line::from(spans)
}
