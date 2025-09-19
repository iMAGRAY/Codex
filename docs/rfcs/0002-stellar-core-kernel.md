# RFC 0002: Stellar Core Kernel

**Status**: Draft  
**Author**: GPT-5-codex  
**Created**: 2025-09-18  
**Updated**: 2025-09-18  
**Reviewers**: Core, UX, Accessibility, DX  
**Target Release**: M1 / Week 1–3

---

## 1. Goal & Non-Goals
- **Goal**: Deliver a unified Stellar TUI/CLI kernel featuring Command Router, Keymap Engine, FlexGrid layout runtime, and Input Guard that satisfies REQ-UX-01/02 and REQ-ACC-01 from `MaxThink-Stellar.md` (#1, #2, #4, #63).
- **Success Metrics**: METRIC-APDEX ≤ 180 мс; METRIC-CSAT ≥ 4.5; zero accessibility blockers at launch.
- **Non-Goals**: Resilience caching, security hardening, and delivery pipeline decisions (handled in RFC 0003/0004/0005); new plugin marketplace capabilities (REQ-DX-01) beyond kernel hooks.

## 2. Scope & Personas
- **Operator & Assistive Tech**: Navigate Insight Canvas with keyboard-first, high-contrast UI (`stellar-tui-vision.md`).
- **SRE & Platform Engineer**: Require CLI/TUI parity for automation and layout DSL for dashboards.
- **Partner Developer**: Consumes SDK APIs for widgets; integration details deferred to DX RFC.

## 3. Architecture Overview
```
┌──────────────┐
│ Input Stream │
└──────┬───────┘
       │ key events
┌──────▼───────┐    ┌──────────────┐
│ Command      │    │ Accessibility│
│ Router (#1)  │───▶│ Bridge (#6)  │
└──────┬───────┘    └────┬─────────┘
       │                 │
┌──────▼────────┐   ┌────▼─────────┐
│ Keymap Engine │   │ Insight      │
│ (#2/#3)       │   │ Canvas View  │
└──────┬────────┘   │ Layout (#4)  │
       │            └────┬─────────┘
┌──────▼────────┐        │
│ FlexGrid      │        │ render tree
│ Runtime (#4)  │◀───────┘
└──────┬────────┘
       │ diff frames
┌──────▼────────┐
│ Render Driver │ (Async pipeline, REQ-PERF-01)
└───────────────┘
```

## 4. Detailed Design
### 4.1 Command Router & Keymap Engine
- Normalize input events (keyboard/mouse/assistive) into semantic actions; maintain keymap profiles per role.
- DSL-driven routing table allowing contextual overrides (modal flows, inline overlays).
- Accessibility hooks ensure focus cycles and escape sequences abide by REQ-ACC-01.

### 4.2 FlexGrid Layout Runtime
- Declarative layout spec describing Insight Canvas, Telemetry side-panels, runbook drawers.
- Diff-oriented rendering using Smart Diff (REQ-PERF-01 #16) to reduce repaint cost.
- Integration with `codex-tui` stylings (`Stylize`, `wrapping.rs`) for cognitive ease.

### 4.3 Input Guard & Validation
- Synchronous guard rails blocking unsafe commands; integrates with Security RFC for policy queries.
- Rate-limits high-cost commands to maintain APDEX target; fallbacks for offline mode (resilience hooks).

### 4.4 CLI/TUI Bridge
- Shared core crate `codex-core::stellar_kernel` powering both CLI command handlers and TUI components.
- Bridge layer exposes JSON events for automation and transcript logging (DX alignment).

## 5. Constraints & Dependencies
- Must not regress existing TUI command latency or keyboard navigation.
- Depends on `codex-tui` style guidelines and `textwrap` utilities.
- Security policies resolved asynchronously; guard layer must fail closed.

## 6. Trace Anchors
| Requirement | Implementation Artifact | Validation |
| ----------- | ------------------------ | ---------- |
| REQ-UX-01 (#1, #2) | Command Router, Keymap Engine modules | Unit tests (`command_router_test.rs`), APDEX telemetry |
| REQ-UX-02 (#4, #63) | FlexGrid runtime, layout specs | Snapshot tests (Insta) |
| REQ-ACC-01 (#6) | Accessibility bridge hooks | Accessibility test suite, assistive tech scripts |
| REQ-PERF-01 (#16) | Render pipeline diffing | Benchmark harness (`bench_stellar_render.rs`) |

## 7. Validation Plan
- `cargo test -p codex-core --features stellar_kernel`
- `cargo test -p codex-tui` with Insta snapshots updated.
- Accessibility regression suite (screen reader playback, contrast checklists).
- Instrument APDEX metrics via OpenTelemetry export; baseline captured in Validate phase.

## 8. Open Questions & Risks
- Need confirmation on keyboard macro customization scope for partner developers.
- Async render driver may require bounded executor; decision deferred to ADR-CORE-001.
- Ensure Input Guard integrates cleanly with forthcoming security policies without double prompts.
