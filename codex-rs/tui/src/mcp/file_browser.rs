use std::cell::Cell;
use std::cmp::Ordering;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use ratatui::buffer::Buffer;
use ratatui::layout::Constraint;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Block;
use ratatui::widgets::Borders;
use ratatui::widgets::Cell as TableCell;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Row;
use ratatui::widgets::StatefulWidget;
use ratatui::widgets::Table;
use ratatui::widgets::TableState;
use ratatui::widgets::Widget;
use ratatui::widgets::Wrap;

use crate::bottom_pane::ScrollState;

#[derive(Clone, Debug)]
pub(crate) struct FileBrowserEntry {
    label: String,
    summary: Option<String>,
    path: PathBuf,
    kind: FileBrowserEntryKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FileBrowserEntryKind {
    Current,
    Parent,
    Directory,
    Manifest,
}

pub(crate) enum FileBrowserOutcome {
    None,
    Selected(String),
    Cancelled,
}

pub(crate) struct FileBrowser {
    workspace_root: PathBuf,
    current_dir: PathBuf,
    entries: Vec<FileBrowserEntry>,
    scroll: ScrollState,
    error: Option<String>,
    visible_rows: Cell<usize>,
}

impl FileBrowser {
    pub(crate) fn new(start: PathBuf, workspace_root: PathBuf) -> Self {
        let normalized_root = workspace_root;
        let mut browser = Self {
            workspace_root: normalized_root.clone(),
            current_dir: normalize_start(&start, &normalized_root),
            entries: Vec::new(),
            scroll: ScrollState::new(),
            error: None,
            visible_rows: Cell::new(1),
        };
        browser.refresh_entries();
        browser
    }

    pub(crate) fn render(&self, area: Rect, buf: &mut Buffer, title: &str) {
        let help_height = 4;
        let header_height = 3;
        let layout = Layout::vertical([
            Constraint::Length(header_height),
            Constraint::Min(area.height.saturating_sub(header_height + help_height)),
            Constraint::Length(help_height),
        ])
        .split(area);

        let header_lines = vec![
            Line::from(vec![
                "Browse from: ".dim(),
                relative_display(&self.current_dir, &self.workspace_root).into(),
            ]),
            Line::from(vec![
                "Root: ".dim(),
                self.workspace_root.to_string_lossy().into_owned().into(),
            ]),
        ];
        let header = Paragraph::new(header_lines)
            .block(Block::default().borders(Borders::ALL).title(title))
            .wrap(Wrap { trim: true });
        Widget::render(header, layout[0], buf);

        self.render_table(layout[1], buf);

        let mut help_lines = Vec::new();
        if let Some(err) = &self.error {
            help_lines.push(err.as_str().red().into());
        }
        help_lines.push(Line::from(vec![
            "Enter".cyan(),
            " select  ".dim(),
            "→/l".cyan(),
            " open  ".dim(),
            "←/h/Backspace".cyan(),
            " up  ".dim(),
            "space".cyan(),
            " select highlighted  ".dim(),
            "Esc".cyan(),
            " cancel".dim(),
        ]));
        let help = Paragraph::new(help_lines)
            .block(Block::default().borders(Borders::ALL).title("Controls"))
            .wrap(Wrap { trim: true });
        Widget::render(help, layout[2], buf);
    }

    fn render_table(&self, area: Rect, buf: &mut Buffer) {
        let rows: Vec<Row> = self
            .entries
            .iter()
            .map(|entry| {
                let style = match entry.kind {
                    FileBrowserEntryKind::Current => Style::default().add_modifier(Modifier::BOLD),
                    FileBrowserEntryKind::Manifest => {
                        Style::default().fg(ratatui::style::Color::Yellow)
                    }
                    _ => Style::default(),
                };
                Row::new(vec![
                    TableCell::from(Span::styled(entry.label.clone(), style)),
                    TableCell::from(Span::styled(
                        entry.summary.clone().unwrap_or_default(),
                        style,
                    )),
                ])
            })
            .collect();

        let mut state = TableState::default();
        state.select(self.scroll.selected_idx);
        *state.offset_mut() = self.scroll.scroll_top;

        let table = Table::new(
            rows,
            [Constraint::Percentage(45), Constraint::Percentage(55)],
        )
        .header(Row::new(vec!["Entry".bold(), "Summary".bold()]))
        .block(Block::default().borders(Borders::ALL).title("Workspace"))
        .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));
        StatefulWidget::render(table, area, buf, &mut state);

        let visible_rows = area.height.saturating_sub(3) as usize;
        self.visible_rows.set(visible_rows.max(1));
    }

    pub(crate) fn handle_key(&mut self, key: crossterm::event::KeyEvent) -> FileBrowserOutcome {
        use crossterm::event::KeyCode;
        self.error = None;
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => FileBrowserOutcome::Cancelled,
            KeyCode::Up => {
                self.scroll.move_up_wrap(self.entries.len());
                self.scroll
                    .ensure_visible(self.entries.len(), self.visible_rows());
                FileBrowserOutcome::None
            }
            KeyCode::Down => {
                self.scroll.move_down_wrap(self.entries.len());
                self.scroll
                    .ensure_visible(self.entries.len(), self.visible_rows());
                FileBrowserOutcome::None
            }
            KeyCode::PageUp => {
                let steps = self.visible_rows().saturating_sub(1);
                for _ in 0..steps {
                    self.scroll.move_up_wrap(self.entries.len());
                }
                self.scroll
                    .ensure_visible(self.entries.len(), self.visible_rows());
                FileBrowserOutcome::None
            }
            KeyCode::PageDown => {
                let steps = self.visible_rows().saturating_sub(1);
                for _ in 0..steps {
                    self.scroll.move_down_wrap(self.entries.len());
                }
                self.scroll
                    .ensure_visible(self.entries.len(), self.visible_rows());
                FileBrowserOutcome::None
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                if let Some(entry) = self.selected_entry() {
                    match entry.kind {
                        FileBrowserEntryKind::Current => {
                            return FileBrowserOutcome::Selected(relative_display(
                                &self.current_dir,
                                &self.workspace_root,
                            ));
                        }
                        FileBrowserEntryKind::Parent => {
                            self.go_up();
                            return FileBrowserOutcome::None;
                        }
                        FileBrowserEntryKind::Directory => {
                            return FileBrowserOutcome::Selected(relative_display(
                                &entry.path,
                                &self.workspace_root,
                            ));
                        }
                        FileBrowserEntryKind::Manifest => {
                            let target = entry
                                .path
                                .parent()
                                .map(Path::to_path_buf)
                                .unwrap_or_else(|| self.current_dir.clone());
                            return FileBrowserOutcome::Selected(relative_display(
                                &target,
                                &self.workspace_root,
                            ));
                        }
                    }
                }
                FileBrowserOutcome::None
            }
            KeyCode::Left | KeyCode::Backspace | KeyCode::Char('h') => {
                self.go_up();
                FileBrowserOutcome::None
            }
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Char('/') => {
                if let Some(entry) = self.selected_entry()
                    && matches!(entry.kind, FileBrowserEntryKind::Directory)
                {
                    self.descend(entry.path.clone());
                }
                FileBrowserOutcome::None
            }
            _ => FileBrowserOutcome::None,
        }
    }

    fn visible_rows(&self) -> usize {
        self.visible_rows.get().max(1)
    }

    fn selected_entry(&self) -> Option<&FileBrowserEntry> {
        self.scroll
            .selected_idx
            .and_then(|idx| self.entries.get(idx))
    }

    fn go_up(&mut self) {
        if self.current_dir == self.workspace_root {
            return;
        }
        if let Some(parent) = self.current_dir.parent() {
            self.current_dir = parent.to_path_buf();
            self.refresh_entries();
        }
    }

    fn descend(&mut self, candidate: PathBuf) {
        if !candidate.starts_with(&self.workspace_root) {
            self.error = Some("Cannot leave workspace root".to_string());
            return;
        }
        if candidate.is_dir() {
            self.current_dir = candidate;
            self.refresh_entries();
        }
    }

    fn refresh_entries(&mut self) {
        self.entries.clear();
        self.scroll = ScrollState::new();

        let mut items: Vec<FileBrowserEntry> = Vec::new();
        items.push(FileBrowserEntry {
            label: format!("📁 {}", "Use this directory"),
            summary: Some(relative_display(&self.current_dir, &self.workspace_root)),
            path: self.current_dir.clone(),
            kind: FileBrowserEntryKind::Current,
        });

        if self.current_dir != self.workspace_root {
            if let Some(parent) = self.current_dir.parent() {
                items.push(FileBrowserEntry {
                    label: "⬆  Parent".to_string(),
                    summary: Some(relative_display(parent, &self.workspace_root)),
                    path: parent.to_path_buf(),
                    kind: FileBrowserEntryKind::Parent,
                });
            }
        }

        match fs::read_dir(&self.current_dir) {
            Ok(read_dir) => {
                let mut dirs = Vec::new();
                let mut manifests = Vec::new();
                for entry in read_dir.flatten() {
                    let path = entry.path();
                    let file_name = entry.file_name().to_string_lossy().into_owned();
                    if let Ok(ft) = entry.file_type() {
                        if ft.is_dir() {
                            dirs.push(FileBrowserEntry {
                                label: format!("📁 {file_name}"),
                                summary: None,
                                path,
                                kind: FileBrowserEntryKind::Directory,
                            });
                        } else if ft.is_file() && looks_like_manifest(&file_name) {
                            manifests.push(FileBrowserEntry {
                                label: format!("📄 {file_name}"),
                                summary: Some("MCP manifest".to_string()),
                                path,
                                kind: FileBrowserEntryKind::Manifest,
                            });
                        }
                    }
                }
                dirs.sort_by(|a, b| a.label.cmp(&b.label));
                manifests.sort_by(|a, b| compare_manifest_entries(a, b));
                items.extend(dirs);
                items.extend(manifests);
            }
            Err(err) => {
                self.error = Some(format!("Failed to read directory: {err}"));
            }
        }

        self.entries = items;
        self.scroll.clamp_selection(self.entries.len());
    }
}

fn normalize_start(start: &Path, root: &Path) -> PathBuf {
    if start.is_dir() && start.starts_with(root) {
        return start.to_path_buf();
    }
    root.to_path_buf()
}

fn relative_display(path: &Path, root: &Path) -> String {
    match pathdiff::diff_paths(path, root) {
        Some(rel) if !rel.as_os_str().is_empty() => rel.to_string_lossy().into_owned(),
        _ => ".".to_string(),
    }
}

fn looks_like_manifest(name: &str) -> bool {
    matches!(
        name,
        "mcp.json" | "server.toml" | "mcp.toml" | "codex-mcp.toml"
    )
}

fn compare_manifest_entries(a: &FileBrowserEntry, b: &FileBrowserEntry) -> Ordering {
    a.label.cmp(&b.label)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_display_for_root_is_dot() {
        let root = PathBuf::from("/tmp/work");
        assert_eq!(relative_display(&root, &root), ".");
    }
}
