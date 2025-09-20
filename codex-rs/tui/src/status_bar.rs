use chrono::DateTime;
use chrono::Utc;
use codex_core::auth::account_pool_state;
use codex_core::telemetry::TelemetryHub;
use codex_core::telemetry::TelemetrySnapshot;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use ratatui::widgets::WidgetRef;

use crate::progressive_disclosure::{
    helpers, DisclosureContext, DisclosureItem, DisclosureManager, InfoPriority,
};
use crate::ui_consts::{LayoutMode, MIN_TERMINAL_WIDTH};
use std::cell::RefCell;
use std::time::Duration;

pub struct StatusBar {
    disclosure_manager: RefCell<DisclosureManager>,
}

impl Default for StatusBar {
    fn default() -> Self {
        Self::new()
    }
}

impl StatusBar {
    pub fn new() -> Self {
        Self {
            disclosure_manager: RefCell::new(DisclosureManager::new()),
        }
    }

    /// Update the disclosure context (e.g., when user starts typing, task begins, etc.)
    pub fn set_context(&self, context: DisclosureContext) {
        self.disclosure_manager.borrow_mut().set_context(context);
    }

    /// Add a temporary notification to the status bar
    pub fn add_notification(&self, message: &str, duration: Duration) {
        let item = helpers::notification_item(message, duration);
        self.disclosure_manager.borrow_mut().add_item(item);
    }

    /// Add an error message to the status bar
    pub fn add_error(&self, message: &str) {
        let item = helpers::error_item(message);
        self.disclosure_manager.borrow_mut().add_item(item);
    }

}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusPriority {
    Critical, // Account issues, auth failures
    High,     // Task status, active operations
    Medium,   // Performance metrics
    Low,      // Additional telemetry
}

#[derive(Debug, Clone)]
struct StatusSegment {
    spans: Vec<Span<'static>>,
    priority: StatusPriority,
    min_width: u16,
}

impl StatusBar {
    /// Render the status bar with progressive disclosure
    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width < 10 {
            return;
        }

        let layout_mode = LayoutMode::from_width(area.width);
        let horizontal_padding = layout_mode.horizontal_padding().min(area.width);
        let content_width_limit = layout_mode
            .max_content_width(area.width.max(MIN_TERMINAL_WIDTH))
            .min(area.width.saturating_sub(horizontal_padding));
        let snapshot = TelemetryHub::global().snapshot();

        // Add current system status to disclosure manager
        self.update_system_status(snapshot);

        // Get progressive disclosure items
        let disclosure_lines = self
            .disclosure_manager
            .borrow()
            .get_visible_items(layout_mode, area.width);

        // Fallback to static content if no disclosure items
        let content = if !disclosure_lines.is_empty() {
            disclosure_lines
        } else {
            let segments = Self::build_status_segments(snapshot);
            let filtered_spans = Self::adaptive_filter(segments, area.width);
            if !filtered_spans.is_empty() {
                vec![Line::from(filtered_spans)]
            } else {
                vec![]
            }
        };

        // Render content with enhanced visual hierarchy
        if !content.is_empty() {
            let enhanced_content = Self::apply_visual_hierarchy(content, layout_mode);
            for (i, line) in enhanced_content.iter().enumerate() {
                if i < area.height as usize && content_width_limit > 0 {
                    let line_area = Rect {
                        x: area.x + horizontal_padding,
                        y: area.y + i as u16,
                        width: content_width_limit,
                        height: 1,
                    };
                    Paragraph::new(line.clone()).render_ref(line_area, buf);
                }
            }
        }
    }

    /// Update system status in the disclosure manager
    fn update_system_status(&self, snapshot: TelemetrySnapshot) {
        let mut manager = self.disclosure_manager.borrow_mut();

        if matches!(manager.current_context(), DisclosureContext::TaskRunning) {
            manager.add_item(helpers::task_status_item("Task active"));
        }

        // Critical: Account issues (always shown)
        let account_state = account_pool_state();
        if account_state.available_accounts == 0 && account_state.total_accounts > 1 {
            let item = helpers::error_item("No accounts available");
            manager.add_item(item);
        }

        // High priority: Performance warnings
        if snapshot.latency_p95_ms > 500.0 {
            let item = helpers::performance_item(
                "Latency",
                &format!("{:.0}ms", snapshot.latency_p95_ms),
                false,
            );
            manager.add_item(item);
        }

        // High priority: Audit fallbacks (serious issues)
        if snapshot.audit_fallback_count > 0 {
            let item = helpers::error_item(&format!(
                "Audit fallbacks: {}",
                snapshot.audit_fallback_count
            ));
            manager.add_item(item);
        }

        // Medium priority: Good performance metrics (when space allows)
        if snapshot.latency_p95_ms <= 200.0 && snapshot.cache_hit_ratio > 0.9 {
            let item = helpers::performance_item(
                "Performance",
                &format!(
                    "{:.0}ms | {:.0}% cache",
                    snapshot.latency_p95_ms,
                    snapshot.cache_hit_ratio * 100.0
                ),
                true,
            );
            manager.add_item(item);
        }

        // Low priority: Detailed telemetry (shown only when lots of space)
        if snapshot.apdex > 0.95 {
            let item = DisclosureItem::new(
                vec![format!("Apdex {:.2}", snapshot.apdex).green()],
                InfoPriority::Low,
            )
            .with_context_relevance(DisclosureContext::Idle, 0.3)
            .with_min_width(12);
            manager.add_item(item);
        }
    }

    /// Apply enhanced visual hierarchy to content
    fn apply_visual_hierarchy(
        content: Vec<Line<'static>>,
        layout_mode: LayoutMode,
    ) -> Vec<Line<'static>> {
        content
            .into_iter()
            .map(|mut line| {
                // Add subtle visual enhancements based on layout mode
                if layout_mode.show_detailed_info() {
                    // Add visual separators for better readability
                    let mut enhanced_spans = Vec::new();

                    for (i, span) in line.spans.iter().enumerate() {
                        if i > 0 {
                            enhanced_spans.push(" │ ".dim());
                        }
                        enhanced_spans.push(span.clone());
                    }

                    line.spans = enhanced_spans;
                }
                line
            })
            .collect()
    }

    fn build_status_segments(snapshot: TelemetrySnapshot) -> Vec<StatusSegment> {
        let mut segments = Vec::new();

        // Critical: Account status (if issues)
        let account_spans = account_spans();
        if !account_spans.is_empty() {
            let has_critical_issue = account_spans
                .iter()
                .any(|span| span.style.fg == Some(ratatui::style::Color::Red));

            segments.push(StatusSegment {
                spans: account_spans,
                priority: if has_critical_issue {
                    StatusPriority::Critical
                } else {
                    StatusPriority::High
                },
                min_width: 20,
            });
        }

        // High: Performance indicators (only critical metrics)
        let critical_telemetry = critical_telemetry_spans(snapshot);
        if !critical_telemetry.is_empty() {
            segments.push(StatusSegment {
                spans: critical_telemetry,
                priority: StatusPriority::High,
                min_width: 25,
            });
        }

        // Medium: Additional telemetry
        let detailed_telemetry = detailed_telemetry_spans(snapshot);
        if !detailed_telemetry.is_empty() {
            segments.push(StatusSegment {
                spans: detailed_telemetry,
                priority: StatusPriority::Medium,
                min_width: 35,
            });
        }

        // Low: Help hint
        segments.push(StatusSegment {
            spans: vec!["Ctrl+O Observability".cyan()],
            priority: StatusPriority::Low,
            min_width: 18,
        });

        segments
    }

    fn adaptive_filter(segments: Vec<StatusSegment>, available_width: u16) -> Vec<Span<'static>> {
        let mut result_spans = Vec::new();
        let mut used_width = 0u16;

        // Sort by priority (Critical first)
        let mut sorted_segments = segments;
        sorted_segments.sort_by_key(|s| match s.priority {
            StatusPriority::Critical => 0,
            StatusPriority::High => 1,
            StatusPriority::Medium => 2,
            StatusPriority::Low => 3,
        });

        for segment in sorted_segments {
            let segment_width = segment.min_width + if !result_spans.is_empty() { 2 } else { 0 }; // +2 for spacing

            if used_width + segment_width <= available_width {
                if !result_spans.is_empty() {
                    result_spans.push("  ".into()); // Add spacing
                    used_width += 2;
                }

                result_spans.extend(segment.spans);
                used_width += segment.min_width;
            }
        }

        result_spans
    }
}

fn account_spans() -> Vec<Span<'static>> {
    let state = account_pool_state();
    if state.total_accounts <= 1 {
        return Vec::new();
    }

    let mut spans = Vec::new();
    let active_display = state
        .active_index
        .map(|idx| format!("{}/{}", idx + 1, state.total_accounts))
        .unwrap_or_else(|| format!("?/{}", state.total_accounts));
    let label = format!("Acct {active_display}");

    if state.available_accounts == 0 {
        spans.push(label.clone().red());
    } else if state.rotation_enabled {
        spans.push(label.clone().green());
    } else {
        spans.push(label.cyan());
    }

    if state.rotation_enabled {
        spans.push("  ".into());
        spans.push("Auto".cyan());
    } else {
        spans.push("  ".into());
        spans.push("Manual".yellow());
    }

    let standby = state.available_accounts.saturating_sub(1);
    if standby > 0 {
        spans.push("  ".into());
        spans.push(format!("Avail {}", standby).green());
    }

    if state.cooldown_accounts > 0 {
        spans.push("  ".into());
        let mut cooldown = format!("Cool {}", state.cooldown_accounts);
        if let Some(next_at) = state.next_available_at {
            cooldown.push_str(&format!(" {}", format_eta(next_at)));
        }
        spans.push(cooldown.red());
    }

    if state.inactive_accounts > 0 {
        spans.push("  ".into());
        spans.push(format!("Inactive {}", state.inactive_accounts).yellow());
    }

    if let Some(last_switch) = state.last_rotation_at {
        spans.push("  ".into());
        spans.push(format!("Switched {} ago", format_ago(last_switch)).yellow());
    }

    spans
}

/// Build critical telemetry spans (high priority)
fn critical_telemetry_spans(snapshot: TelemetrySnapshot) -> Vec<Span<'static>> {
    let mut spans = Vec::new();

    // Only show latency if it's concerning
    if snapshot.latency_p95_ms > 200.0 {
        let latency = format!("⚡{:.0}ms", snapshot.latency_p95_ms);
        if snapshot.latency_p95_ms <= 500.0 {
            spans.push(latency.yellow());
        } else {
            spans.push(latency.red());
        }
    }

    // Show audit fallbacks only if they exist
    if snapshot.audit_fallback_count > 0 {
        if !spans.is_empty() {
            spans.push(" ".into());
        }
        spans.push(format!("⚠️{}", snapshot.audit_fallback_count).red());
    }

    spans
}

/// Build detailed telemetry spans (medium priority)
fn detailed_telemetry_spans(snapshot: TelemetrySnapshot) -> Vec<Span<'static>> {
    let mut spans = Vec::new();

    // Always show latency in detailed view
    let latency = format!("Latency {:>3.0}ms", snapshot.latency_p95_ms);
    if snapshot.latency_p95_ms <= 200.0 {
        spans.push(latency.green());
    } else if snapshot.latency_p95_ms <= 500.0 {
        spans.push(latency.cyan());
    } else {
        spans.push(latency.red());
    }

    spans.push("  ".into());

    // Cache hit ratio
    let ratio = snapshot.cache_hit_ratio * 100.0;
    let cache = format!("Cache {:>3.0}%", ratio);
    if ratio >= 80.0 {
        spans.push(cache.green());
    } else if ratio >= 50.0 {
        spans.push(cache.cyan());
    } else {
        spans.push(cache.red());
    }

    spans
}

/// Build spans for the observability status line (REQ-OBS-01, REQ-OPS-01).
/// Legacy function for compatibility
pub fn telemetry_spans(snapshot: TelemetrySnapshot) -> Vec<Span<'static>> {
    detailed_telemetry_spans(snapshot)
}

fn format_ago(ts: DateTime<Utc>) -> String {
    let now = Utc::now();
    let delta = now.signed_duration_since(ts);
    if delta.num_seconds() <= 0 {
        return "0s".to_string();
    }
    let secs = delta.num_seconds();
    if secs < 60 {
        return format!("{}s", secs);
    }
    let mins = delta.num_minutes();
    if mins < 60 {
        return format!("{}m", mins);
    }
    let hours = delta.num_hours();
    if hours < 24 {
        return format!("{}h", hours);
    }
    let days = delta.num_days();
    if days < 7 {
        return format!("{}d", days);
    }
    format!("{}w", days / 7)
}

fn format_eta(ts: DateTime<Utc>) -> String {
    let now = Utc::now();
    if ts <= now {
        return "0s".to_string();
    }
    let delta = ts - now;
    let secs = delta.num_seconds();
    if secs < 60 {
        return format!("{}s", secs);
    }
    let mins = delta.num_minutes();
    if mins < 60 {
        let rem = secs - mins * 60;
        if rem == 0 {
            return format!("{}m", mins);
        }
        return format!("{}m{}s", mins, rem);
    }
    let hours = delta.num_hours();
    if hours < 24 {
        let rem = mins - hours * 60;
        if rem == 0 {
            return format!("{}h", hours);
        }
        return format!("{}h {}m", hours, rem);
    }
    let days = delta.num_days();
    if days < 7 {
        let rem = hours - days * 24;
        if rem == 0 {
            return format!("{}d", days);
        }
        return format!("{}d {}h", days, rem);
    }
    let weeks = days / 7;
    let rem = days % 7;
    if rem == 0 {
        return format!("{}w", weeks);
    }
    format!("{}w {}d", weeks, rem)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use codex_core::auth::set_account_pool_state_for_testing;
    use codex_core::auth::AccountPoolState;
    #[test]
    fn renders_snapshot_spans() {
        let spans = telemetry_spans(TelemetrySnapshot {
            latency_p95_ms: 120.0,
            audit_fallback_count: 0,
            cache_hit_ratio: 0.9,
            apdex: 0.97,
        });
        let joined: String = spans
            .iter()
            .map(|span| span.content.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(joined.contains("Latency"));
        assert!(joined.contains("Cache"));
    }

    #[test]
    fn telemetry_spans_emit_fallback_warning() {
        let spans = critical_telemetry_spans(TelemetrySnapshot {
            latency_p95_ms: 800.0,
            audit_fallback_count: 3,
            cache_hit_ratio: 0.4,
            apdex: 0.8,
        });
        let joined: String = spans
            .iter()
            .map(|span| span.content.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(joined.contains("⚡800"));
        assert!(joined.contains("⚠️3"));
    }

    #[test]
    fn account_spans_show_rotation_warning() {
        let mut state = AccountPoolState::default();
        state.total_accounts = 5;
        state.active_index = Some(2);
        state.rotation_enabled = true;
        state.available_accounts = 3;
        state.rate_limited_accounts = 2;
        state.cooldown_accounts = 2;
        state.inactive_accounts = 1;
        state.next_available_at = Some(Utc::now() + Duration::minutes(5));
        state.last_rotation_at = Some(Utc::now() - Duration::minutes(3));
        set_account_pool_state_for_testing(state);

        let spans = account_spans();
        let joined: String = spans
            .iter()
            .map(|span| span.content.to_string())
            .collect::<Vec<_>>()
            .join(" ");

        assert!(joined.contains("Acct 3/5"));
        assert!(joined.contains("Cool"));
        assert!(joined.contains("Switched"));
        assert!(joined.contains("Avail"));
    }
}
