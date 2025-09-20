//! Progressive disclosure system for reducing cognitive load in the TUI.
//!
//! This module implements a smart information hierarchy that shows only
//! relevant information based on context and user needs, following principles
//! of progressive disclosure to create a cleaner, more focused interface.

use crate::ui_consts::LayoutMode;
use ratatui::style::{Color, Modifier, Stylize};
use ratatui::text::{Line, Span};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Information priority levels for progressive disclosure
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum InfoPriority {
    Critical = 0, // Always shown - errors, urgent warnings
    High = 1,     // Important context - active operations, key status
    Medium = 2,   // Helpful details - performance metrics, secondary info
    Low = 3,      // Nice to have - debug info, detailed telemetry
}

/// Context for determining what information to show
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DisclosureContext {
    Idle,        // User not actively working
    TaskRunning, // AI task in progress
    UserTyping,  // User composing message
    Error,       // Error state requiring attention
    Modal,       // Modal dialog active
}

/// A piece of information that can be progressively disclosed
#[derive(Debug, Clone)]
pub struct DisclosureItem {
    pub content: Vec<Span<'static>>,
    pub priority: InfoPriority,
    pub context_relevance: HashMap<DisclosureContext, f32>, // 0.0-1.0 relevance score
    pub min_width: u16,
    pub expires_at: Option<Instant>,
}

impl DisclosureItem {
    pub fn new(content: Vec<Span<'static>>, priority: InfoPriority) -> Self {
        Self {
            content,
            priority,
            context_relevance: HashMap::new(),
            min_width: 20,
            expires_at: None,
        }
    }

    /// Set relevance score for a specific context
    pub fn with_context_relevance(mut self, context: DisclosureContext, relevance: f32) -> Self {
        self.context_relevance
            .insert(context, relevance.clamp(0.0, 1.0));
        self
    }

    /// Set minimum width required to display this item
    pub fn with_min_width(mut self, width: u16) -> Self {
        self.min_width = width;
        self
    }

    /// Set expiration time for temporary information
    pub fn with_expiration(mut self, duration: Duration) -> Self {
        self.expires_at = Some(Instant::now() + duration);
        self
    }

    /// Check if this item is still valid (not expired)
    pub fn is_valid(&self) -> bool {
        self.expires_at
            .map_or(true, |expires| Instant::now() < expires)
    }

    /// Get relevance score for the current context
    pub fn relevance_for_context(&self, context: &DisclosureContext) -> f32 {
        self.context_relevance.get(context).copied().unwrap_or(0.5)
    }
}

/// Progressive disclosure manager that determines what information to show
pub struct DisclosureManager {
    items: Vec<DisclosureItem>,
    current_context: DisclosureContext,
    last_interaction: Instant,
    user_detail_preference: f32, // 0.0 = minimal, 1.0 = show everything
}

impl DisclosureManager {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            current_context: DisclosureContext::Idle,
            last_interaction: Instant::now(),
            user_detail_preference: 0.5, // Start with balanced view
        }
    }

    /// Update the current context
    pub fn set_context(&mut self, context: DisclosureContext) {
        if self.current_context != context {
            self.current_context = context;
            self.last_interaction = Instant::now();
            let wants_more_detail = matches!(
                context,
                DisclosureContext::TaskRunning | DisclosureContext::Error
            );
            self.adjust_detail_preference(wants_more_detail);
        }
    }

    /// Add an item to be managed by progressive disclosure
    pub fn add_item(&mut self, item: DisclosureItem) {
        // Remove expired items first
        self.items.retain(|item| item.is_valid());
        self.items.push(item);
    }

    pub fn current_context(&self) -> DisclosureContext {
        self.current_context
    }

    /// Get items to display based on current context and available width
    pub fn get_visible_items(
        &self,
        layout_mode: LayoutMode,
        available_width: u16,
    ) -> Vec<Line<'static>> {
        let mut visible_items: Vec<Line<'static>> = Vec::new();
        let mut used_width = 0u16;

        // Clean up expired items (non-mutating filter)
        let valid_items: Vec<_> = self.items.iter().filter(|item| item.is_valid()).collect();

        // Calculate context-aware priorities
        let mut scored_items: Vec<_> = valid_items
            .iter()
            .map(|item| {
                let base_priority = 4 - item.priority as u8; // Higher number = higher priority
                let context_boost = item.relevance_for_context(&self.current_context);
                let time_decay = self.calculate_time_decay();
                let user_preference_boost = self.user_detail_preference;

                let final_score =
                    base_priority as f32 + context_boost + time_decay + user_preference_boost;
                (item, final_score)
            })
            .collect();

        // Sort by final score (highest first)
        scored_items.sort_by(|a, b| b.1.total_cmp(&a.1));

        // Select items that fit within available width
        for (item, _score) in scored_items {
            let item_width = item.min_width + if !visible_items.is_empty() { 2 } else { 0 }; // +2 for spacing

            if used_width + item_width <= available_width {
                if !visible_items.is_empty() {
                    // Add spacing between items
                    if let Some(last_line) = visible_items.last_mut() {
                        last_line.spans.push("  ".into());
                    }
                }

                let styled_spans = self.apply_context_styling(&item.content);
                visible_items.push(Line::from(styled_spans));
                used_width += item_width;
            }
        }

        // If we have space and the user prefers details, show help hints
        if layout_mode.show_detailed_info() && used_width + 20 <= available_width {
            if let Some(hint) = self.get_contextual_hint() {
                if !visible_items.is_empty() {
                    if let Some(last_line) = visible_items.last_mut() {
                        last_line.spans.push("  ".into());
                    }
                }
                visible_items.push(Line::from(vec![hint]));
            }
        }

        visible_items
    }

    /// Calculate time-based decay for less important information
    fn calculate_time_decay(&self) -> f32 {
        let time_since_interaction = self.last_interaction.elapsed().as_secs_f32();
        // After 10 seconds of inactivity, start preferring less detailed info
        if time_since_interaction > 10.0 {
            -0.5 * (time_since_interaction / 30.0).min(1.0)
        } else {
            0.0
        }
    }

    /// Apply context-appropriate styling to spans
    fn apply_context_styling(&self, spans: &[Span<'static>]) -> Vec<Span<'static>> {
        spans
            .iter()
            .map(|span| {
                let mut styled_span = span.clone();

                // Apply context-based styling
                match self.current_context {
                    DisclosureContext::Error => {
                        styled_span.style = styled_span.style.add_modifier(Modifier::BOLD);
                    }
                    DisclosureContext::TaskRunning => {
                        styled_span.style = styled_span.style.fg(Color::Cyan);
                    }
                    DisclosureContext::UserTyping => {
                        styled_span.style = styled_span.style.dim();
                    }
                    _ => {}
                }

                styled_span
            })
            .collect()
    }

    /// Get contextual help hint based on current state
    fn get_contextual_hint(&self) -> Option<Span<'static>> {
        match self.current_context {
            DisclosureContext::Idle => Some("Type to start, Ctrl+O for details".dim()),
            DisclosureContext::TaskRunning => Some("Esc to interrupt".yellow()),
            DisclosureContext::UserTyping => None, // Don't distract while typing
            DisclosureContext::Error => Some("Check logs with Ctrl+L".red()),
            DisclosureContext::Modal => None,
        }
    }

    /// Adjust user preference based on their behavior
    pub fn adjust_detail_preference(&mut self, wants_more_detail: bool) {
        if wants_more_detail {
            self.user_detail_preference = (self.user_detail_preference + 0.1).min(1.0);
        } else {
            self.user_detail_preference = (self.user_detail_preference - 0.1).max(0.0);
        }
    }

    #[cfg(test)]
    pub(crate) fn detail_preference(&self) -> f32 {
        self.user_detail_preference
    }
}

impl Default for DisclosureManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper functions for creating common disclosure items
pub mod helpers {
    use super::*;

    /// Create an error disclosure item
    pub fn error_item(message: &str) -> DisclosureItem {
        DisclosureItem::new(vec![format!("⚠ {}", message).red()], InfoPriority::Critical)
            .with_context_relevance(DisclosureContext::Error, 1.0)
            .with_context_relevance(DisclosureContext::Idle, 0.8)
            .with_expiration(Duration::from_secs(60))
    }

    /// Create a task status disclosure item
    pub fn task_status_item(status: &str) -> DisclosureItem {
        DisclosureItem::new(vec![format!("⚡ {}", status).cyan()], InfoPriority::High)
            .with_context_relevance(DisclosureContext::TaskRunning, 1.0)
            .with_context_relevance(DisclosureContext::Idle, 0.3)
    }

    /// Create a performance metric disclosure item
    pub fn performance_item(metric: &str, value: &str, is_good: bool) -> DisclosureItem {
        let color = if is_good { Color::Green } else { Color::Yellow };
        DisclosureItem::new(
            vec![format!("{}: {}", metric, value).fg(color)],
            InfoPriority::Medium,
        )
        .with_context_relevance(DisclosureContext::Idle, 0.7)
        .with_context_relevance(DisclosureContext::TaskRunning, 0.2)
        .with_min_width(15)
    }

    /// Create a temporary notification item
    pub fn notification_item(message: &str, duration: Duration) -> DisclosureItem {
        DisclosureItem::new(vec![format!("ℹ {}", message).blue()], InfoPriority::High)
            .with_context_relevance(DisclosureContext::Idle, 0.8)
            .with_context_relevance(DisclosureContext::TaskRunning, 0.3)
            .with_expiration(duration)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disclosure_item_expiration() {
        let item = DisclosureItem::new(vec!["test".into()], InfoPriority::Medium)
            .with_expiration(Duration::from_millis(1));

        assert!(item.is_valid());
        std::thread::sleep(Duration::from_millis(2));
        assert!(!item.is_valid());
    }

    #[test]
    fn context_relevance_scoring() {
        let item = DisclosureItem::new(vec!["test".into()], InfoPriority::Medium)
            .with_context_relevance(DisclosureContext::Error, 0.9)
            .with_context_relevance(DisclosureContext::Idle, 0.1);

        assert_eq!(item.relevance_for_context(&DisclosureContext::Error), 0.9);
        assert_eq!(item.relevance_for_context(&DisclosureContext::Idle), 0.1);
        assert_eq!(
            item.relevance_for_context(&DisclosureContext::UserTyping),
            0.5
        ); // default
    }

    #[test]
    fn disclosure_manager_prioritization() {
        let mut manager = DisclosureManager::new();
        manager.set_context(DisclosureContext::TaskRunning);

        // Add items with different priorities and relevance
        manager.add_item(
            DisclosureItem::new(vec!["low priority".into()], InfoPriority::Low)
                .with_context_relevance(DisclosureContext::TaskRunning, 0.1),
        );
        manager.add_item(
            DisclosureItem::new(vec!["high priority".into()], InfoPriority::High)
                .with_context_relevance(DisclosureContext::TaskRunning, 0.9),
        );

        let items = manager.get_visible_items(LayoutMode::Standard, 100);

        // Should show high priority item first
        assert!(items.len() >= 1);
        assert!(items[0]
            .spans
            .iter()
            .any(|span| span.content.contains("high priority")));
    }

    #[test]
    fn width_constraint_filtering() {
        let mut manager = DisclosureManager::new();

        manager.add_item(
            DisclosureItem::new(vec!["small".into()], InfoPriority::High).with_min_width(10),
        );
        manager.add_item(
            DisclosureItem::new(vec!["large".into()], InfoPriority::Medium).with_min_width(50),
        );

        let items = manager.get_visible_items(LayoutMode::Compact, 30);

        // Should only show the small item
        assert_eq!(items.len(), 1);
        assert!(items[0]
            .spans
            .iter()
            .any(|span| span.content.contains("small")));
    }

    #[test]
    fn time_decay_effect() {
        let mut manager = DisclosureManager::new();
        manager.last_interaction = Instant::now() - Duration::from_secs(20);

        let decay = manager.calculate_time_decay();
        assert!(decay < 0.0); // Should prefer less detail after time passes
    }

    #[test]
    fn adjust_detail_preference_bounds() {
        let mut manager = DisclosureManager::new();
        manager.adjust_detail_preference(true);
        assert!(manager.detail_preference() <= 1.0);
        manager.adjust_detail_preference(false);
        manager.adjust_detail_preference(false);
        assert!(manager.detail_preference() >= 0.0);
    }

    #[test]
    fn task_status_helper_sets_task_context_bias() {
        let item = helpers::task_status_item("Deploying");
        assert!(item.relevance_for_context(&DisclosureContext::TaskRunning) >= 1.0);
    }

}
