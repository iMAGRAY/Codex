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

pub struct StatusBar;

impl StatusBar {
    pub fn render(area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width < 10 {
            return;
        }
        let snapshot = TelemetryHub::global().snapshot();
        let mut spans = account_spans();
        if !spans.is_empty() {
            spans.push("  ".into());
        }
        spans.extend(telemetry_spans(snapshot));
        spans.push("  ".into());
        spans.push("Ctrl+O Observability".cyan());
        Paragraph::new(Line::from(spans)).render_ref(area, buf);
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

/// Build spans for the observability status line (REQ-OBS-01, REQ-OPS-01).
pub fn telemetry_spans(snapshot: TelemetrySnapshot) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let latency = format!("Latency p95 {:>5.0}ms", snapshot.latency_p95_ms);
    if snapshot.latency_p95_ms <= 200.0 {
        spans.push(latency.green());
    } else if snapshot.latency_p95_ms <= 500.0 {
        spans.push(latency.cyan());
    } else {
        spans.push(latency.red());
    }
    spans.push("  ".into());

    let audit = format!("Audit fallback {}", snapshot.audit_fallback_count);
    if snapshot.audit_fallback_count == 0 {
        spans.push(audit.green());
    } else {
        spans.push(audit.red());
    }
    spans.push("  ".into());

    let ratio = snapshot.cache_hit_ratio * 100.0;
    let cache = format!("Cache hit {:>5.1}%", ratio);
    if ratio >= 80.0 {
        spans.push(cache.green());
    } else if ratio >= 50.0 {
        spans.push(cache.cyan());
    } else {
        spans.push(cache.red());
    }

    spans
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
    use codex_core::auth::AccountPoolState;
    use codex_core::auth::set_account_pool_state_for_testing;

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
        assert!(joined.contains("Audit"));
        assert!(joined.contains("Cache"));
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
