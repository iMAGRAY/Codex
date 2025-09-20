//! Bottom pane: shows the ChatComposer or a BottomPaneView, if one is active.
use std::path::PathBuf;

use crate::app_event_sender::AppEventSender;
use crate::tui::FrameRequester;
use crate::ui_consts::{LayoutMode, SmartSpacing, MIN_TERMINAL_HEIGHT};
use crate::user_approval_widget::ApprovalRequest;
pub(crate) use bottom_pane_view::BottomPaneView;
use codex_core::protocol::TokenUsageInfo;
use codex_file_search::FileMatch;
use crossterm::event::KeyEvent;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::WidgetRef;
use std::time::Duration;

mod approval_modal_view;
mod bottom_pane_view;
mod chat_composer;
mod chat_composer_history;
mod command_popup;
mod file_search_popup;
mod list_selection_view;
mod paste_burst;
mod popup_consts;
mod scroll_state;
mod selection_popup_common;
mod textarea;

pub(crate) use scroll_state::ScrollState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CancellationEvent {
    Handled,
    NotHandled,
}

pub(crate) use chat_composer::ChatComposer;
pub(crate) use chat_composer::InputResult;
use codex_protocol::custom_prompts::CustomPrompt;

use crate::status_indicator_widget::StatusIndicatorWidget;
use approval_modal_view::ApprovalModalView;
pub(crate) use list_selection_view::SelectionAction;
pub(crate) use list_selection_view::SelectionItem;

/// Pane displayed in the lower half of the chat UI.
pub(crate) struct BottomPane {
    /// Composer is retained even when a BottomPaneView is displayed so the
    /// input state is retained when the view is closed.
    composer: ChatComposer,

    /// If present, this is displayed instead of the `composer` (e.g. modals).
    active_view: Option<Box<dyn BottomPaneView>>,

    app_event_tx: AppEventSender,
    frame_requester: FrameRequester,

    has_input_focus: bool,
    is_task_running: bool,
    ctrl_c_quit_hint: bool,
    esc_backtrack_hint: bool,

    /// Inline status indicator shown above the composer while a task is running.
    status: Option<StatusIndicatorWidget>,
    /// Queued user messages to show under the status indicator.
    queued_user_messages: Vec<String>,
}

pub(crate) struct BottomPaneParams {
    pub(crate) app_event_tx: AppEventSender,
    pub(crate) frame_requester: FrameRequester,
    pub(crate) has_input_focus: bool,
    pub(crate) enhanced_keys_supported: bool,
    pub(crate) placeholder_text: String,
    pub(crate) disable_paste_burst: bool,
}

impl BottomPane {
    pub fn new(params: BottomPaneParams) -> Self {
        let enhanced_keys_supported = params.enhanced_keys_supported;
        Self {
            composer: ChatComposer::new(
                params.has_input_focus,
                params.app_event_tx.clone(),
                enhanced_keys_supported,
                params.placeholder_text,
                params.disable_paste_burst,
            ),
            active_view: None,
            app_event_tx: params.app_event_tx,
            frame_requester: params.frame_requester,
            has_input_focus: params.has_input_focus,
            is_task_running: false,
            ctrl_c_quit_hint: false,
            status: None,
            queued_user_messages: Vec::new(),
            esc_backtrack_hint: false,
        }
    }

    pub(crate) fn set_input_focus(&mut self, has_focus: bool) {
        if self.has_input_focus == has_focus {
            return;
        }
        self.has_input_focus = has_focus;
        self.composer.set_has_focus(has_focus);
    }

    pub fn desired_height(&self, width: u16) -> u16 {
        let layout_mode = LayoutMode::from_width(width);

        // Adaptive spacing based on terminal size and focus state
        let top_margin = SmartSpacing::section_spacing(layout_mode, true);
        let bottom_padding = SmartSpacing::bottom_padding(layout_mode, self.has_input_focus);

        // Base height depends on whether a modal/overlay is active.
        let base = match self.active_view.as_ref() {
            Some(view) => view.desired_height(width),
            None => {
                let composer_height = self.composer.desired_height(width);
                let status_height = self.status
                    .as_ref()
                    .map_or(0, |status| status.desired_height(width));

                // Ensure minimum interactive height for usability
                let min_height = SmartSpacing::min_interactive_height(layout_mode);
                (composer_height + status_height).max(min_height)
            },
        };

        base.saturating_add(bottom_padding)
            .saturating_add(top_margin)
    }

    fn layout(&self, area: Rect) -> [Rect; 2] {
        let layout_mode = LayoutMode::from_width(area.width);

        // Adaptive margins based on terminal size and available space
        let (top_margin, bottom_margin) = self.calculate_adaptive_margins(area, layout_mode);

        let content_area = Rect {
            x: area.x,
            y: area.y + top_margin,
            width: area.width,
            height: area.height.saturating_sub(top_margin + bottom_margin),
        };

        match self.active_view.as_ref() {
            Some(_) => [Rect::ZERO, content_area],
            None => {
                let status_height = self
                    .status
                    .as_ref()
                    .map_or(0, |status| status.desired_height(content_area.width));

                let mut remaining = content_area.height;
                let mut next_y = content_area.y;

                let status_rect = if status_height == 0 || remaining == 0 {
                    Rect::ZERO
                } else {
                    let height = status_height.min(remaining);
                    let rect = Rect {
                        x: content_area.x,
                        y: next_y,
                        width: content_area.width,
                        height,
                    };
                    remaining = remaining.saturating_sub(height);
                    next_y = next_y.saturating_add(height);
                    rect
                };

                let spacing = if status_rect.height > 0 && layout_mode.show_detailed_info() { 1 } else { 0 };
                let applied_spacing = spacing.min(remaining);
                if applied_spacing > 0 {
                    remaining = remaining.saturating_sub(applied_spacing);
                    next_y = next_y.saturating_add(applied_spacing);
                }

                let composer_height = remaining;
                let composer_rect = if composer_height == 0 {
                    Rect::ZERO
                } else {
                    Rect {
                        x: content_area.x,
                        y: next_y,
                        width: content_area.width,
                        height: composer_height,
                    }
                };

                [status_rect, composer_rect]
            }
        }
    }

    /// Calculate adaptive margins based on available space and layout mode
    fn calculate_adaptive_margins(&self, area: Rect, layout_mode: LayoutMode) -> (u16, u16) {
        let base_top = SmartSpacing::section_spacing(layout_mode, true);
        let base_bottom = SmartSpacing::bottom_padding(layout_mode, self.has_input_focus);

        // At very small heights, reduce or eliminate margins to preserve functionality
        if area.height <= MIN_TERMINAL_HEIGHT / 2 {
            (0, 0)
        } else if area.height <= MIN_TERMINAL_HEIGHT {
            (base_top.min(1), base_bottom.min(1))
        } else {
            (base_top, base_bottom)
        }
    }

    pub fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        // Hide the cursor whenever an overlay view is active (e.g. the
        // status indicator shown while a task is running, or approval modal).
        // In these states the textarea is not interactable, so we should not
        // show its caret.
        if self.active_view.is_some() {
            None
        } else {
            let layout_areas = self.layout(area);
            // Use the last area which should be the composer area
            let composer_area = layout_areas[layout_areas.len() - 1];
            self.composer.cursor_pos(composer_area)
        }
    }

    /// Forward a key event to the active view or the composer.
    pub fn handle_key_event(&mut self, key_event: KeyEvent) -> InputResult {
        if let Some(mut view) = self.active_view.take() {
            view.handle_key_event(self, key_event);
            if !view.is_complete() {
                self.active_view = Some(view);
            } else {
                self.on_active_view_complete();
            }
            self.request_redraw();
            InputResult::None
        } else {
            // If a task is running and a status line is visible, allow Esc to
            // send an interrupt even while the composer has focus.
            if matches!(key_event.code, crossterm::event::KeyCode::Esc)
                && self.is_task_running
                && let Some(status) = &self.status
            {
                // Send Op::Interrupt
                status.interrupt();
                self.request_redraw();
                return InputResult::None;
            }
            let (input_result, needs_redraw) = self.composer.handle_key_event(key_event);
            if needs_redraw {
                self.request_redraw();
            }
            if self.composer.is_in_paste_burst() {
                self.request_redraw_in(ChatComposer::recommended_paste_flush_delay());
            }
            input_result
        }
    }

    /// Handle Ctrl-C in the bottom pane. If a modal view is active it gets a
    /// chance to consume the event (e.g. to dismiss itself).
    pub(crate) fn on_ctrl_c(&mut self) -> CancellationEvent {
        let mut view = match self.active_view.take() {
            Some(view) => view,
            None => {
                return if self.composer_is_empty() {
                    CancellationEvent::NotHandled
                } else {
                    self.set_composer_text(String::new());
                    self.show_ctrl_c_quit_hint();
                    CancellationEvent::Handled
                };
            }
        };

        let event = view.on_ctrl_c(self);
        match event {
            CancellationEvent::Handled => {
                if !view.is_complete() {
                    self.active_view = Some(view);
                } else {
                    self.on_active_view_complete();
                }
                self.show_ctrl_c_quit_hint();
            }
            CancellationEvent::NotHandled => {
                self.active_view = Some(view);
            }
        }
        event
    }

    pub fn handle_paste(&mut self, pasted: String) {
        if self.active_view.is_none() {
            let needs_redraw = self.composer.handle_paste(pasted);
            if needs_redraw {
                self.request_redraw();
            }
        }
    }

    pub(crate) fn insert_str(&mut self, text: &str) {
        self.composer.insert_str(text);
        self.request_redraw();
    }

    /// Replace the composer text with `text`.
    pub(crate) fn set_composer_text(&mut self, text: String) {
        self.composer.set_text_content(text);
        self.request_redraw();
    }

    /// Get the current composer text (for tests and programmatic checks).
    #[cfg(test)]
    pub(crate) fn composer_text(&self) -> String {
        self.composer.current_text()
    }

    /// Update the animated header shown to the left of the brackets in the
    /// status indicator (defaults to "Working"). No-ops if the status
    /// indicator is not active.
    pub(crate) fn update_status_header(&mut self, header: String) {
        if let Some(status) = self.status.as_mut() {
            status.update_header(header);
            self.request_redraw();
        }
    }

    pub(crate) fn show_ctrl_c_quit_hint(&mut self) {
        self.ctrl_c_quit_hint = true;
        self.composer
            .set_ctrl_c_quit_hint(true, self.has_input_focus);
        self.request_redraw();
    }

    pub(crate) fn clear_ctrl_c_quit_hint(&mut self) {
        if self.ctrl_c_quit_hint {
            self.ctrl_c_quit_hint = false;
            self.composer
                .set_ctrl_c_quit_hint(false, self.has_input_focus);
            self.request_redraw();
        }
    }

    #[cfg(test)]
    pub(crate) fn ctrl_c_quit_hint_visible(&self) -> bool {
        self.ctrl_c_quit_hint
    }

    pub(crate) fn show_esc_backtrack_hint(&mut self) {
        self.esc_backtrack_hint = true;
        self.composer.set_esc_backtrack_hint(true);
        self.request_redraw();
    }

    pub(crate) fn clear_esc_backtrack_hint(&mut self) {
        if self.esc_backtrack_hint {
            self.esc_backtrack_hint = false;
            self.composer.set_esc_backtrack_hint(false);
            self.request_redraw();
        }
    }

    // esc_backtrack_hint_visible removed; hints are controlled internally.

    pub fn set_task_running(&mut self, running: bool) {
        self.is_task_running = running;
        self.composer.set_task_running(running);

        if running {
            if self.status.is_none() {
                self.status = Some(StatusIndicatorWidget::new(
                    self.app_event_tx.clone(),
                    self.frame_requester.clone(),
                ));
            }
            if let Some(status) = self.status.as_mut() {
                status.set_queued_messages(self.queued_user_messages.clone());
            }
            self.request_redraw();
        } else {
            // Hide the status indicator when a task completes, but keep other modal views.
            self.status = None;
        }
    }

    /// Show a generic list selection view with the provided items.
    pub(crate) fn show_selection_view(
        &mut self,
        title: String,
        subtitle: Option<String>,
        footer_hint: Option<String>,
        items: Vec<SelectionItem>,
    ) {
        let view = list_selection_view::ListSelectionView::new(
            title,
            subtitle,
            footer_hint,
            items,
            self.app_event_tx.clone(),
        );
        self.active_view = Some(Box::new(view));
        self.request_redraw();
    }

    pub(crate) fn show_custom_view(&mut self, view: Box<dyn BottomPaneView>) {
        self.active_view = Some(view);
        self.request_redraw();
    }

    /// Update the queued messages shown under the status header.
    pub(crate) fn set_queued_user_messages(&mut self, queued: Vec<String>) {
        self.queued_user_messages = queued.clone();
        if let Some(status) = self.status.as_mut() {
            status.set_queued_messages(queued);
        }
        self.request_redraw();
    }

    /// Update custom prompts available for the slash popup.
    pub(crate) fn set_custom_prompts(&mut self, prompts: Vec<CustomPrompt>) {
        self.composer.set_custom_prompts(prompts);
        self.request_redraw();
    }

    pub(crate) fn composer_is_empty(&self) -> bool {
        self.composer.is_empty()
    }

    pub(crate) fn is_task_running(&self) -> bool {
        self.is_task_running
    }

    /// Return true when the pane is in the regular composer state without any
    /// overlays or popups and not running a task. This is the safe context to
    /// use Esc-Esc for backtracking from the main view.
    pub(crate) fn is_normal_backtrack_mode(&self) -> bool {
        !self.is_task_running && self.active_view.is_none() && !self.composer.popup_active()
    }

    /// Update the *context-window remaining* indicator in the composer. This
    /// is forwarded directly to the underlying `ChatComposer`.
    pub(crate) fn set_token_usage(&mut self, token_info: Option<TokenUsageInfo>) {
        self.composer.set_token_usage(token_info);
        self.request_redraw();
    }

    /// Called when the agent requests user approval.
    pub fn push_approval_request(&mut self, request: ApprovalRequest) {
        let request = if let Some(view) = self.active_view.as_mut() {
            match view.try_consume_approval_request(request) {
                Some(request) => request,
                None => {
                    self.request_redraw();
                    return;
                }
            }
        } else {
            request
        };

        // Otherwise create a new approval modal overlay.
        let modal = ApprovalModalView::new(request, self.app_event_tx.clone());
        self.pause_status_timer_for_modal();
        self.active_view = Some(Box::new(modal));
        self.request_redraw()
    }

    fn on_active_view_complete(&mut self) {
        self.resume_status_timer_after_modal();
    }

    fn pause_status_timer_for_modal(&mut self) {
        if let Some(status) = self.status.as_mut() {
            status.pause_timer();
        }
    }

    fn resume_status_timer_after_modal(&mut self) {
        if let Some(status) = self.status.as_mut() {
            status.resume_timer();
        }
    }

    /// Height (terminal rows) required by the current bottom pane.
    pub(crate) fn request_redraw(&self) {
        self.frame_requester.schedule_frame();
    }

    pub(crate) fn request_redraw_in(&self, dur: Duration) {
        self.frame_requester.schedule_frame_in(dur);
    }

    // --- History helpers ---

    pub(crate) fn set_history_metadata(&mut self, log_id: u64, entry_count: usize) {
        self.composer.set_history_metadata(log_id, entry_count);
    }

    pub(crate) fn flush_paste_burst_if_due(&mut self) -> bool {
        self.composer.flush_paste_burst_if_due()
    }

    pub(crate) fn is_in_paste_burst(&self) -> bool {
        self.composer.is_in_paste_burst()
    }

    pub(crate) fn on_history_entry_response(
        &mut self,
        log_id: u64,
        offset: usize,
        entry: Option<String>,
    ) {
        let updated = self
            .composer
            .on_history_entry_response(log_id, offset, entry);

        if updated {
            self.request_redraw();
        }
    }

    pub(crate) fn on_file_search_result(&mut self, query: String, matches: Vec<FileMatch>) {
        self.composer.on_file_search_result(query, matches);
        self.request_redraw();
    }

    pub(crate) fn attach_image(
        &mut self,
        path: PathBuf,
        width: u32,
        height: u32,
        format_label: &str,
    ) {
        if self.active_view.is_none() {
            self.composer
                .attach_image(path, width, height, format_label);
            self.request_redraw();
        }
    }

    pub(crate) fn take_recent_submission_images(&mut self) -> Vec<PathBuf> {
        self.composer.take_recent_submission_images()
    }
}

impl WidgetRef for &BottomPane {
    fn render_ref(&self, area: Rect, buf: &mut Buffer) {
        let [status_area, content] = self.layout(area);

        // When a modal view is active, it owns the whole content area.
        if let Some(view) = &self.active_view {
            view.render(content, buf);
        } else {
            // No active modal:
            // If a status indicator is active, render it above the composer.
            if let Some(status) = &self.status {
                status.render_ref(status_area, buf);
            }

            // Render the composer in the remaining area.
            self.composer.render_ref(content, buf);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_event::AppEvent;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use tokio::sync::mpsc::unbounded_channel;

    fn exec_request() -> ApprovalRequest {
        ApprovalRequest::Exec {
            id: "1".to_string(),
            command: vec!["echo".into(), "ok".into()],
            reason: None,
        }
    }

    #[test]
    fn ctrl_c_on_modal_consumes_and_shows_quit_hint() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut pane = BottomPane::new(BottomPaneParams {
            app_event_tx: tx,
            frame_requester: FrameRequester::test_dummy(),
            has_input_focus: true,
            enhanced_keys_supported: false,
            placeholder_text: "Ask Codex to do anything".to_string(),
            disable_paste_burst: false,
        });
        pane.push_approval_request(exec_request());
        assert_eq!(CancellationEvent::Handled, pane.on_ctrl_c());
        assert!(pane.ctrl_c_quit_hint_visible());
        assert_eq!(CancellationEvent::NotHandled, pane.on_ctrl_c());
    }

    // live ring removed; related tests deleted.

    #[test]
    fn overlay_not_shown_above_approval_modal() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut pane = BottomPane::new(BottomPaneParams {
            app_event_tx: tx,
            frame_requester: FrameRequester::test_dummy(),
            has_input_focus: true,
            enhanced_keys_supported: false,
            placeholder_text: "Ask Codex to do anything".to_string(),
            disable_paste_burst: false,
        });

        // Create an approval modal (active view).
        pane.push_approval_request(exec_request());

        // Render and verify the top row does not include an overlay.
        let area = Rect::new(0, 0, 60, 6);
        let mut buf = Buffer::empty(area);
        (&pane).render_ref(area, &mut buf);

        let mut r0 = String::new();
        for x in 0..area.width {
            r0.push(buf[(x, 0)].symbol().chars().next().unwrap_or(' '));
        }
        assert!(
            !r0.contains("Working"),
            "overlay should not render above modal"
        );
    }

    #[test]
    fn composer_shown_after_denied_while_task_running() {
        let (tx_raw, rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut pane = BottomPane::new(BottomPaneParams {
            app_event_tx: tx,
            frame_requester: FrameRequester::test_dummy(),
            has_input_focus: true,
            enhanced_keys_supported: false,
            placeholder_text: "Ask Codex to do anything".to_string(),
            disable_paste_burst: false,
        });

        // Start a running task so the status indicator is active above the composer.
        pane.set_task_running(true);

        // Push an approval modal (e.g., command approval) which should hide the status view.
        pane.push_approval_request(exec_request());

        // Simulate pressing 'n' (No) on the modal.
        use crossterm::event::KeyCode;
        use crossterm::event::KeyEvent;
        use crossterm::event::KeyModifiers;
        pane.handle_key_event(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));

        // After denial, since the task is still running, the status indicator should be
        // visible above the composer. The modal should be gone.
        assert!(
            pane.active_view.is_none(),
            "no active modal view after denial"
        );

        std::thread::sleep(Duration::from_millis(120));
        let area = Rect::new(0, 0, 40, 6);
        let mut buf = Buffer::empty(area);
        (&pane).render_ref(area, &mut buf);

        let [status_rect, composer_rect] = pane.layout(area);
        assert!(status_rect.height > 0, "expected status rect to be visible");

        let mut header_line = String::new();
        for x in 0..status_rect.width {
            header_line.push(
                buf[(status_rect.x + x, status_rect.y)]
                    .symbol()
                    .chars()
                    .next()
                    .unwrap_or(' '),
            );
        }
        assert!(
            header_line.contains("Working"),
            "expected Working header after denial: {header_line:?}"
        );

        assert!(composer_rect.height > 0, "expected composer rect after denial");
        let mut found_composer = false;
        for y in 0..composer_rect.height {
            let mut row = String::new();
            for x in 0..composer_rect.width {
                row.push(
                    buf[(composer_rect.x + x, composer_rect.y + y)]
                        .symbol()
                        .chars()
                        .next()
                        .unwrap_or(' '),
                );
            }
            if row.contains("Ask Codex") {
                found_composer = true;
                break;
            }
        }
        assert!(
            found_composer,
            "expected composer visible under status line"
        );

        drop(rx);
    }

    #[test]
    fn status_indicator_visible_during_command_execution() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut pane = BottomPane::new(BottomPaneParams {
            app_event_tx: tx,
            frame_requester: FrameRequester::test_dummy(),
            has_input_focus: true,
            enhanced_keys_supported: false,
            placeholder_text: "Ask Codex to do anything".to_string(),
            disable_paste_burst: false,
        });

        // Begin a task: show initial status.
        pane.set_task_running(true);

        let area = Rect::new(0, 0, 40, 6);
        let mut buf = Buffer::empty(area);
        (&pane).render_ref(area, &mut buf);

        let [status_rect, _composer_rect] = pane.layout(area);
        assert!(status_rect.height > 0, "expected non-zero status rect");

        let mut header_line = String::new();
        for x in 0..status_rect.width {
            header_line.push(
                buf[(status_rect.x + x, status_rect.y)]
                    .symbol()
                    .chars()
                    .next()
                    .unwrap_or(' '),
            );
        }
        assert!(
            header_line.contains("Working"),
            "expected Working header: {header_line:?}"
        );
    }

    #[test]
    fn bottom_padding_present_with_status_above_composer() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut pane = BottomPane::new(BottomPaneParams {
            app_event_tx: tx,
            frame_requester: FrameRequester::test_dummy(),
            has_input_focus: true,
            enhanced_keys_supported: false,
            placeholder_text: "Ask Codex to do anything".to_string(),
            disable_paste_burst: false,
        });

        pane.set_task_running(true);

        let height = pane.desired_height(30);
        assert!(height >= 3, "expected at least 3 rows; got {height}");
        let area = Rect::new(0, 0, 30, height);
        let mut buf = Buffer::empty(area);
        (&pane).render_ref(area, &mut buf);

        let [status_rect, composer_rect] = pane.layout(area);
        assert!(status_rect.height > 0, "expected status rect to be visible");

        let mut header_line = String::new();
        for x in 0..status_rect.width {
            header_line.push(
                buf[(status_rect.x + x, status_rect.y)]
                    .symbol()
                    .chars()
                    .next()
                    .unwrap_or(' '),
            );
        }
        assert!(
            header_line.trim_start().starts_with("Working"),
            "expected status header to start with Working: {header_line:?}"
        );

        if composer_rect.height > 0 {
            let mut composer_line = String::new();
            for x in 0..composer_rect.width {
                composer_line.push(
                    buf[(composer_rect.x + x, composer_rect.y)]
                        .symbol()
                        .chars()
                        .next()
                        .unwrap_or(' '),
                );
            }
            assert!(
                composer_line.contains("Ask Codex") || composer_rect.height > 1,
                "expected composer content when space is available: {composer_line:?}"
            );
        }

        let mut r_last = String::new();
        for x in 0..area.width {
            r_last.push(buf[(x, area.y + area.height - 1)].symbol().chars().next().unwrap_or(' '));
        }
        assert!(
            r_last.trim().is_empty(),
            "expected last row blank padding: {r_last:?}"
        );
    }

    #[test]
    fn bottom_padding_shrinks_when_tiny() {
        let (tx_raw, _rx) = unbounded_channel::<AppEvent>();
        let tx = AppEventSender::new(tx_raw);
        let mut pane = BottomPane::new(BottomPaneParams {
            app_event_tx: tx,
            frame_requester: FrameRequester::test_dummy(),
            has_input_focus: true,
            enhanced_keys_supported: false,
            placeholder_text: "Ask Codex to do anything".to_string(),
            disable_paste_burst: false,
        });

        pane.set_task_running(true);

        // Height=2 → prioritise status visibility when space is scarce.
        let area2 = Rect::new(0, 0, 20, 2);
        let mut buf2 = Buffer::empty(area2);
        (&pane).render_ref(area2, &mut buf2);
        let [status_rect2, composer_rect2] = pane.layout(area2);
        assert!(status_rect2.height > 0, "expected status header at height=2");
        let mut status_line = String::new();
        for x in 0..status_rect2.width {
            status_line.push(
                buf2[(status_rect2.x + x, status_rect2.y)]
                    .symbol()
                    .chars()
                    .next()
                    .unwrap_or(' '),
            );
        }
        assert!(status_line.contains("Working"), "expected Working header at height=2: {status_line:?}");
        assert_eq!(composer_rect2.height, 0, "composer should collapse when only two rows are available");

        // Height=3 → status line plus composer.
        let area3 = Rect::new(0, 0, 20, 3);
        let mut buf3 = Buffer::empty(area3);
        (&pane).render_ref(area3, &mut buf3);
        let [status_rect3, composer_rect3] = pane.layout(area3);
        assert!(status_rect3.height > 0);
        assert!(composer_rect3.height > 0, "expected composer when height≥3");
        let mut composer_line = String::new();
        for x in 0..composer_rect3.width {
            composer_line.push(
                buf3[(composer_rect3.x + x, composer_rect3.y)]
                    .symbol()
                    .chars()
                    .next()
                    .unwrap_or(' '),
            );
        }
        assert!(
            composer_line.contains("Ask Codex") || composer_rect3.height > 1,
            "expected composer placeholder when height=3: {composer_line:?}"
        );

        // Height=1 → status indicator wins; composer is suppressed.
        let area1 = Rect::new(0, 0, 20, 1);
        let mut buf1 = Buffer::empty(area1);
        (&pane).render_ref(area1, &mut buf1);
        let [status_rect1, composer_rect1] = pane.layout(area1);
        assert!(status_rect1.height > 0, "status remains visible when height=1");
        assert_eq!(composer_rect1.height, 0, "composer collapses when only one row available");
        let mut status_line_one = String::new();
        for x in 0..status_rect1.width {
            status_line_one.push(
                buf1[(status_rect1.x + x, status_rect1.y)]
                    .symbol()
                    .chars()
                    .next()
                    .unwrap_or(' '),
            );
        }
        assert!(
            status_line_one.contains("Working"),
            "expected Working header when only one row remains: {status_line_one:?}"
        );
    }
}
