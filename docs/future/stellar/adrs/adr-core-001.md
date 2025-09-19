# ADR-CORE-001: Stellar Kernel Render Pipeline

## Status
Draft

## Context
- Stellar kernel must deliver async rendering without regressing APDEX targets (REQ-UX-01, REQ-PERF-01).
- Need unified pipeline for CLI/TUI surfaces while supporting assistive tech focus updates.

## Decision
Adopt a diff-driven render pipeline that batches layout updates on a bounded tokio executor, emitting frame deltas through a shared `RenderDriver` trait consumed by both TUI and CLI adapters.

## Consequences
- **Positive**: Consistent rendering semantics, easier snapshot testing, and shared instrumentation for latency metrics.
- **Negative**: Requires executor tuning to avoid starvation; introduces dependency on async runtime for CLI bridge.
- **Operational**: Must expose configuration knob for frame budget and integrate with observability spans.

## Alignment
- Requirements: REQ-UX-01, REQ-UX-02, REQ-PERF-01.
- Metrics: METRIC-APDEX, METRIC-LATENCY.
- Linked Artifacts: `docs/rfcs/0002-stellar-core-kernel.md`, unit & snapshot tests, render benchmark harness.
