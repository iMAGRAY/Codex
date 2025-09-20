use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Style;
use ratatui::style::Stylize;
use ratatui::widgets::{Widget, WidgetRef};
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const MAX_CHILDREN_PER_DIR: usize = 200;

#[derive(Debug, Clone)]
pub(crate) struct VisibleEntry {
    pub id: usize,
    pub depth: u16,
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub is_expanded: bool,
    pub is_placeholder: bool,
}

pub(crate) struct FileExplorerState {
    root: ExplorerNode,
    visible: Vec<VisibleEntry>,
    selected: usize,
    next_id: usize,
}

impl FileExplorerState {
    pub fn new(root_path: PathBuf) -> Self {
        let mut state = Self {
            root: ExplorerNode::directory(0, root_path.clone(), display_name_for_root(&root_path)),
            visible: Vec::new(),
            selected: 0,
            next_id: 1,
        };
        state.root.set_expanded(true);
        if let Err(err) = state.rebuild_visible() {
            tracing::warn!(error = %err, "failed to initialize file explorer view");
        }
        state
    }

    pub fn visible_items(&self) -> &[VisibleEntry] {
        &self.visible
    }

    pub fn widget(&self, focused: bool) -> FileExplorerWidget<'_> {
        FileExplorerWidget {
            state: self,
            focused,
        }
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    #[allow(dead_code)]
    pub fn select_index(&mut self, idx: usize) {
        if idx < self.visible.len() {
            self.selected = idx;
        }
    }

    pub fn move_selection(&mut self, delta: isize) {
        if self.visible.is_empty() {
            return;
        }
        let len = self.visible.len() as isize;
        let current = self.selected as isize;
        let next = (current + delta).clamp(0, len - 1);
        self.selected = next as usize;
    }

    pub fn selected_entry(&self) -> Option<&VisibleEntry> {
        self.visible.get(self.selected)
    }

    pub fn toggle_expanded(&mut self) -> io::Result<()> {
        let id = match self.selected_entry() {
            Some(entry) if entry.is_placeholder => return Ok(()),
            Some(entry) if entry.is_dir => entry.id,
            _ => return Ok(()),
        };
        self.toggle_dir_state(id)?;
        self.rebuild_visible()?;
        Ok(())
    }

    pub fn collapse_selected(&mut self) -> io::Result<()> {
        let id = match self.selected_entry() {
            Some(entry) if entry.is_dir => Some(entry.id),
            Some(entry) => self.parent_dir_id(entry.id),
            None => None,
        };
        if let Some(dir_id) = id {
            let changed = self.set_dir_expanded(dir_id, false)?;
            if changed {
                self.rebuild_visible()?;
            }
            self.select_node_by_id(dir_id);
        }
        Ok(())
    }

    pub fn expand_selected(&mut self) -> io::Result<()> {
        if let Some(entry) = self.selected_entry() {
            if entry.is_dir && self.set_dir_expanded(entry.id, true)? {
                self.rebuild_visible()?;
            }
        }
        Ok(())
    }

    pub fn refresh(&mut self) -> io::Result<()> {
        self.root.reset_cache_recursive();
        self.rebuild_visible()
    }

    pub fn selected_path(&self) -> Option<PathBuf> {
        self.selected_entry().map(|entry| entry.path.clone())
    }

    pub fn root_name(&self) -> &str {
        &self.root.name
    }

    pub fn select_node_by_id(&mut self, node_id: usize) {
        if let Some(idx) = self.visible.iter().position(|entry| entry.id == node_id) {
            self.selected = idx;
        }
    }

    pub fn select_first(&mut self) {
        if !self.visible.is_empty() {
            self.selected = 0;
        }
    }

    pub fn select_last(&mut self) {
        if let Some(last) = self.visible.len().checked_sub(1) {
            self.selected = last;
        }
    }

    pub fn parent_dir_id(&mut self, node_id: usize) -> Option<usize> {
        find_parent_id(&mut self.root, node_id, None)
    }

    fn toggle_dir_state(&mut self, node_id: usize) -> io::Result<()> {
        let expanded = {
            let node = self.find_node_mut(node_id)?;
            match &mut node.kind {
                ExplorerNodeKind::Directory(dir) => {
                    let new_state = !dir.expanded;
                    dir.expanded = new_state;
                    new_state
                }
                ExplorerNodeKind::File => return Ok(()),
            }
        };

        if expanded {
            self.ensure_children_loaded(node_id)?;
        }
        Ok(())
    }

    fn set_dir_expanded(&mut self, node_id: usize, expanded: bool) -> io::Result<bool> {
        let mut changed = false;
        {
            let node = self.find_node_mut(node_id)?;
            match &mut node.kind {
                ExplorerNodeKind::Directory(dir) => {
                    if dir.expanded != expanded {
                        dir.expanded = expanded;
                        changed = true;
                    }
                }
                ExplorerNodeKind::File => return Ok(false),
            }
        }
        if changed && expanded {
            self.ensure_children_loaded(node_id)?;
        }
        Ok(changed)
    }

    fn ensure_children_loaded(&mut self, node_id: usize) -> io::Result<()> {
        let path = {
            let node = self.find_node(node_id)?;
            match &node.kind {
                ExplorerNodeKind::Directory(dir) => {
                    if dir.children_loaded {
                        return Ok(());
                    }
                    node.path.clone()
                }
                ExplorerNodeKind::File => return Ok(()),
            }
        };
        let (children, truncated) = self.read_dir_entries(&path)?;
        let node = self.find_node_mut(node_id)?;
        if let ExplorerNodeKind::Directory(dir) = &mut node.kind {
            dir.children = children;
            dir.children_loaded = true;
            dir.truncated = truncated;
        }
        Ok(())
    }

    fn rebuild_visible(&mut self) -> io::Result<()> {
        self.visible.clear();
        let mut visible_entries = Vec::new();
        Self::build_visible_recursive(&mut self.root, &mut self.next_id, 0, &mut visible_entries)?;
        self.visible = visible_entries;
        if self.selected >= self.visible.len() {
            self.selected = self.visible.len().saturating_sub(1);
        }
        Ok(())
    }

    fn build_visible_recursive(
        node: &mut ExplorerNode,
        next_id: &mut usize,
        depth: u16,
        visible: &mut Vec<VisibleEntry>,
    ) -> io::Result<()> {
        match &mut node.kind {
            ExplorerNodeKind::File => {
                visible.push(VisibleEntry {
                    id: node.id,
                    depth,
                    name: node.name.clone(),
                    path: node.path.clone(),
                    is_dir: false,
                    is_expanded: false,
                    is_placeholder: false,
                });
            }
            ExplorerNodeKind::Directory(dir_state) => {
                let need_load = dir_state.expanded && !dir_state.children_loaded;
                if need_load {
                    let path = node.path.clone();
                    let (children, truncated) = Self::read_dir_entries_static(&path, next_id)?;
                    if let ExplorerNodeKind::Directory(dir) = &mut node.kind {
                        dir.children = children;
                        dir.children_loaded = true;
                        dir.truncated = truncated;
                    }
                }

                if let ExplorerNodeKind::Directory(dir) = &mut node.kind {
                    visible.push(VisibleEntry {
                        id: node.id,
                        depth,
                        name: node.name.clone(),
                        path: node.path.clone(),
                        is_dir: true,
                        is_expanded: dir.expanded,
                        is_placeholder: false,
                    });

                    if dir.expanded {
                        for child in dir.children.iter_mut() {
                            Self::build_visible_recursive(child, next_id, depth + 1, visible)?;
                        }
                        if dir.truncated {
                            visible.push(VisibleEntry {
                                id: node.id,
                                depth: depth + 1,
                                name: "…".to_string(),
                                path: node.path.clone(),
                                is_dir: false,
                                is_expanded: false,
                                is_placeholder: true,
                            });
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn read_dir_entries_static(
        path: &Path,
        next_id: &mut usize,
    ) -> io::Result<(Vec<ExplorerNode>, bool)> {
        let alloc_id = |next_id: &mut usize| -> usize {
            let id = *next_id;
            *next_id += 1;
            id
        };

        let mut heap = BinaryHeap::new();
        let mut truncated = false;

        let entries = match fs::read_dir(path) {
            Ok(entries) => entries,
            Err(err) => {
                tracing::warn!(error = %err, path = %path.display(), "file explorer read_dir failed");
                return Ok((Vec::new(), false));
            }
        };

        for entry in entries.flatten() {
            let entry_path = entry.path();
            let file_type = match entry.file_type() {
                Ok(ft) => ft,
                Err(_) => continue,
            };
            let name_os = entry.file_name();
            let name = match name_os.into_string() {
                Ok(name) => name,
                Err(_) => continue,
            };

            let node = if file_type.is_dir() {
                ExplorerNode::directory(alloc_id(next_id), entry_path, name)
            } else {
                ExplorerNode::file(alloc_id(next_id), entry_path, name)
            };

            heap.push(SortEntry::new(node));
            if heap.len() > MAX_CHILDREN_PER_DIR {
                heap.pop();
                truncated = true;
            }
        }

        let mut nodes: Vec<ExplorerNode> = heap
            .into_sorted_vec()
            .into_iter()
            .map(|entry| entry.node)
            .collect();
        nodes.shrink_to_fit();
        Ok((nodes, truncated))
    }

    fn read_dir_entries(&mut self, path: &Path) -> io::Result<(Vec<ExplorerNode>, bool)> {
        let mut heap = BinaryHeap::new();
        let mut truncated = false;

        let entries = match fs::read_dir(path) {
            Ok(entries) => entries,
            Err(err) => {
                tracing::warn!(error = %err, path = %path.display(), "file explorer read_dir failed");
                return Ok((Vec::new(), false));
            }
        };

        for entry in entries.flatten() {
            let entry_path = entry.path();
            let file_type = match entry.file_type() {
                Ok(ft) => ft,
                Err(_) => continue,
            };
            let name_os = entry.file_name();
            let name = match name_os.into_string() {
                Ok(name) => name,
                Err(_) => continue,
            };

            let node = if file_type.is_dir() {
                ExplorerNode::directory(self.alloc_id(), entry_path, name)
            } else {
                ExplorerNode::file(self.alloc_id(), entry_path, name)
            };

            heap.push(SortEntry::new(node));
            if heap.len() > MAX_CHILDREN_PER_DIR {
                heap.pop();
                truncated = true;
            }
        }

        let mut nodes: Vec<ExplorerNode> = heap
            .into_sorted_vec()
            .into_iter()
            .map(|entry| entry.node)
            .collect();
        nodes.shrink_to_fit();
        Ok((nodes, truncated))
    }

    fn find_node(&self, node_id: usize) -> io::Result<&ExplorerNode> {
        find_node(&self.root, node_id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "node not found"))
    }

    fn find_node_mut(&mut self, node_id: usize) -> io::Result<&mut ExplorerNode> {
        find_node_mut(&mut self.root, node_id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "node not found"))
    }

    fn alloc_id(&mut self) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
}

fn display_name_for_root(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| path.display().to_string())
}

#[derive(Clone)]
struct ExplorerNode {
    id: usize,
    name: String,
    path: PathBuf,
    kind: ExplorerNodeKind,
}

impl ExplorerNode {
    fn directory(id: usize, path: PathBuf, name: String) -> Self {
        Self {
            id,
            name,
            path,
            kind: ExplorerNodeKind::Directory(DirectoryState::default()),
        }
    }

    fn file(id: usize, path: PathBuf, name: String) -> Self {
        Self {
            id,
            name,
            path,
            kind: ExplorerNodeKind::File,
        }
    }

    fn set_expanded(&mut self, expanded: bool) {
        if let ExplorerNodeKind::Directory(dir) = &mut self.kind {
            dir.expanded = expanded;
        }
    }

    fn reset_cache_recursive(&mut self) {
        match &mut self.kind {
            ExplorerNodeKind::File => {}
            ExplorerNodeKind::Directory(dir) => {
                dir.children_loaded = false;
                dir.truncated = false;
                for child in dir.children.iter_mut() {
                    child.reset_cache_recursive();
                }
                dir.children.clear();
            }
        }
    }
}

#[derive(Clone)]
enum ExplorerNodeKind {
    File,
    Directory(DirectoryState),
}

#[derive(Clone, Default)]
struct DirectoryState {
    expanded: bool,
    children_loaded: bool,
    truncated: bool,
    children: Vec<ExplorerNode>,
}

struct SortEntry {
    node: ExplorerNode,
    lowercase: String,
    sort_rank: u8,
}

impl SortEntry {
    fn new(node: ExplorerNode) -> Self {
        let lowercase = node.name.to_ascii_lowercase();
        let sort_rank = match node.kind {
            ExplorerNodeKind::Directory(_) => 0,
            ExplorerNodeKind::File => 1,
        };
        Self {
            node,
            lowercase,
            sort_rank,
        }
    }
}

impl Eq for SortEntry {}

impl PartialEq for SortEntry {
    fn eq(&self, other: &Self) -> bool {
        self.sort_rank == other.sort_rank
            && self.lowercase == other.lowercase
            && self.node.name == other.node.name
    }
}

impl Ord for SortEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        let ordering = self
            .sort_rank
            .cmp(&other.sort_rank)
            .then_with(|| self.lowercase.cmp(&other.lowercase))
            .then_with(|| self.node.name.cmp(&other.node.name));
        ordering.reverse()
    }
}

impl PartialOrd for SortEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn find_node<'a>(node: &'a ExplorerNode, node_id: usize) -> Option<&'a ExplorerNode> {
    if node.id == node_id {
        return Some(node);
    }
    match &node.kind {
        ExplorerNodeKind::File => None,
        ExplorerNodeKind::Directory(dir) => dir
            .children
            .iter()
            .find_map(|child| find_node(child, node_id)),
    }
}

fn find_node_mut<'a>(node: &'a mut ExplorerNode, node_id: usize) -> Option<&'a mut ExplorerNode> {
    if node.id == node_id {
        return Some(node);
    }
    match &mut node.kind {
        ExplorerNodeKind::File => None,
        ExplorerNodeKind::Directory(dir) => dir
            .children
            .iter_mut()
            .find_map(|child| find_node_mut(child, node_id)),
    }
}

fn find_parent_id(
    node: &mut ExplorerNode,
    target_id: usize,
    parent: Option<usize>,
) -> Option<usize> {
    if node.id == target_id {
        return parent;
    }
    match &mut node.kind {
        ExplorerNodeKind::File => None,
        ExplorerNodeKind::Directory(dir) => {
            for child in dir.children.iter_mut() {
                if let Some(found) = find_parent_id(child, target_id, Some(node.id)) {
                    return Some(found);
                }
            }
            None
        }
    }
}

pub(crate) struct FileExplorerWidget<'a> {
    state: &'a FileExplorerState,
    focused: bool,
}

impl<'a> WidgetRef for FileExplorerWidget<'a> {
    fn render_ref(&self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }

        for y in area.y..area.bottom() {
            fill_line(buf, area.x, y, area.width, Style::default());
        }

        let header_style = if self.focused {
            Style::default().fg(Color::Cyan).bold()
        } else {
            Style::default().fg(Color::DarkGray).bold()
        };
        let header_text = format!("Files · {}", self.state.root_name());
        render_line(
            buf,
            area.x,
            area.y,
            area.width,
            &header_text,
            header_style,
            header_style,
        );

        let mut row_y = area.y.saturating_add(1);
        let rows_available = area.height.saturating_sub(1);
        if rows_available == 0 {
            return;
        }

        let visible = self.state.visible_items();
        let selected = self.state.selected_index();
        let mut rows_rendered = 0u16;

        for (idx, entry) in visible.iter().enumerate() {
            if rows_rendered >= rows_available {
                break;
            }
            let text = render_entry_label(entry);
            let is_selected = idx == selected && !entry.is_placeholder;
            let (fill_style, text_style) = if is_selected {
                if self.focused {
                    (
                        Style::default().bg(Color::Rgb(14, 40, 55)),
                        Style::default()
                            .fg(Color::Cyan)
                            .bg(Color::Rgb(14, 40, 55))
                            .bold(),
                    )
                } else {
                    (
                        Style::default().bg(Color::Rgb(25, 25, 25)),
                        Style::default()
                            .fg(Color::DarkGray)
                            .bg(Color::Rgb(25, 25, 25)),
                    )
                }
            } else if entry.is_placeholder {
                let style = Style::default().fg(Color::DarkGray).italic();
                (Style::default(), style)
            } else if entry.is_dir {
                let style = Style::default().fg(Color::LightCyan);
                (Style::default(), style)
            } else {
                (Style::default(), Style::default())
            };

            render_line(
                buf, area.x, row_y, area.width, &text, text_style, fill_style,
            );
            row_y = row_y.saturating_add(1);
            rows_rendered = rows_rendered.saturating_add(1);
        }

        if visible.is_empty() {
            let style = Style::default().fg(Color::DarkGray).italic();
            render_line(
                buf,
                area.x,
                area.y.saturating_add(1),
                area.width,
                "(no files)",
                style,
                Style::default(),
            );
        }
    }
}

impl<'a> Widget for FileExplorerWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.render_ref(area, buf);
    }
}

fn render_entry_label(entry: &VisibleEntry) -> String {
    if entry.is_placeholder {
        let mut text = String::new();
        text.push_str(&"  ".repeat(entry.depth as usize));
        text.push_str("... more items");
        return text;
    }

    let mut text = String::new();
    text.push_str(&"  ".repeat(entry.depth as usize));
    if entry.is_dir {
        if entry.is_expanded {
            text.push_str("- ");
        } else {
            text.push_str("+ ");
        }
    } else {
        text.push_str("  ");
    }
    text.push_str(&entry.name);
    text
}

fn render_line(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    width: u16,
    text: &str,
    text_style: Style,
    fill_style: Style,
) {
    if width == 0 {
        return;
    }
    fill_line(buf, x, y, width, fill_style);
    let max_chars = width as usize;
    let total_chars = text.chars().count();
    let mut clipped: String = text.chars().take(max_chars).collect();
    if total_chars > max_chars {
        if max_chars >= 3 {
            let new_len = clipped.chars().count().saturating_sub(3);
            clipped = clipped.chars().take(new_len).collect();
            clipped.push_str("...");
        }
    }
    buf.set_stringn(x, y, clipped, max_chars, text_style);
}

fn fill_line(buf: &mut Buffer, x: u16, y: u16, width: u16, style: Style) {
    if width == 0 {
        return;
    }
    let blank = " ".repeat(width as usize);
    buf.set_stringn(x, y, blank, width as usize, style);
}

#[cfg(test)]
mod tests {
    use super::*;
    use insta::assert_snapshot;
    use ratatui::layout::Rect;
    use tempfile::tempdir;

    #[test]
    fn widget_renders_basic_tree() {
        let tmp = tempdir().expect("temp dir");
        let workspace = tmp.path().join("workspace");
        std::fs::create_dir(&workspace).expect("workspace dir");
        std::fs::create_dir(workspace.join("src")).expect("src dir");
        std::fs::write(workspace.join("src/main.rs"), "fn main() {}").unwrap();
        std::fs::write(workspace.join("README.md"), "hello").unwrap();

        let state = FileExplorerState::new(workspace);
        let widget = state.widget(true);
        let area = Rect::new(0, 0, 32, 6);
        let mut buf = Buffer::empty(area);
        widget.render_ref(area, &mut buf);

        let mut rows = Vec::new();
        for y in area.y..area.bottom() {
            let mut line = String::new();
            for x in area.x..area.x + area.width {
                line.push(buf[(x, y)].symbol().chars().next().unwrap_or(' '));
            }
            rows.push(line.trim_end().to_string());
        }

        assert_snapshot!("file_explorer_widget_basic", rows.join("\n"));
    }

    #[test]
    fn collapsing_moves_selection_to_parent() {
        let tmp = tempdir().expect("temp dir");
        let workspace = tmp.path().join("workspace");
        std::fs::create_dir(&workspace).expect("workspace dir");
        std::fs::create_dir(workspace.join("pkg")).expect("pkg dir");
        std::fs::write(workspace.join("pkg/lib.rs"), "").unwrap();

        let mut state = FileExplorerState::new(workspace);
        let src_index = state
            .visible_items()
            .iter()
            .enumerate()
            .find(|(_, entry)| entry.name == "pkg")
            .map(|(idx, _)| idx)
            .expect("pkg visible");
        state.select_index(src_index);
        state.toggle_expanded().expect("expand pkg");

        let lib_index = state
            .visible_items()
            .iter()
            .enumerate()
            .find(|(_, entry)| entry.name == "lib.rs")
            .map(|(idx, _)| idx)
            .expect("lib visible");
        state.select_index(lib_index);

        state.collapse_selected().expect("collapse");

        let selected = state.selected_entry().expect("selection");
        assert!(selected.is_dir);
        assert_eq!(selected.name, "pkg");
    }
}
