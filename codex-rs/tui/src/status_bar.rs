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
        let mut spans = telemetry_spans(snapshot);
        spans.push("  ".into());
        spans.push("Ctrl+O Observability".cyan());
        Paragraph::new(Line::from(spans)).render_ref(area, buf);
    }
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
