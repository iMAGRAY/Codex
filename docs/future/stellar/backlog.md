# Stellar Backlog Trace (M-Series)

## Method
- Epics group roadmap milestones with direct trace to blueprint initiatives (`docs/future/MaxThink-Stellar.md`).
- Each epic lists covered requirements (REQ-*) and blueprint references (#).
- Exit metrics echo primary targets from `todo.md` Milestone Snapshot and blueprint metrics.

## Epic Table
| Epic ID | Milestone | Description | Requirements | Blueprint Trace | Exit Metrics |
| ------- | --------- | ----------- | ------------ | ---------------- | ------------- |
| EPIC-CORE | M1 | Deliver Stellar Core kernel with Command Router, Keymap Engine, FlexGrid layout, Input Guard, keeping CLI/TUI parity | REQ-UX-01, REQ-UX-02, REQ-ACC-01 | #1, #2, #4, #63 | METRIC-APDEX ≤ 180 мс, METRIC-CSAT ≥ 4.5 |
| EPIC-RESILIENCE | M2 | Implement Local Resilience Cache, Conflict Resolver, Predictive Prefetch, chaos robustness | REQ-REL-01, REQ-DATA-01, REQ-PERF-01 | #14, #15, #35, #67 | METRIC-AVAIL ≥ 99.3%, METRIC-LATENCY 95p ≤ 200 мс |
| EPIC-SECURITY | M3 | Harden sandbox with RBAC, Secure Signing, Dynamic Secrets, audit ledger | REQ-SEC-01, REQ-SEC-02, REQ-SEC-03 | #9, #10, #11, #27, #57, #74, #88 | METRIC-SEC-INC = 0 критических, METRIC-AUDIT-OK ≥ 95% |
| EPIC-OBS | M4 | Build Observability Mesh, Telemetry Overlay, Debug Orchestrator, Incident Timeline | REQ-OBS-01, REQ-PERF-01, REQ-OPS-01 | #8, #20, #45, #52, #23 | METRIC-MTTD ≤ 2 мин, METRIC-MTTR ≤ 15 мин |
| EPIC-DELIVERY | M5 | Launch Trusted Pipeline, Governance Portal, Policy Validator, Marketplace guardrails | REQ-OPS-01, REQ-INT-01, REQ-DX-01 | #24, #42, #55, #75, #79, #85, #99 | METRIC-EXT-ADOPT ≥ 60%, METRIC-AVAIL ≥ 99.5% |
| EPIC-LAUNCH | M6 | Final regression, training path, release readiness, reviewer enablement | REQ-ACC-01, REQ-OPS-01 | #43, #58, #87 | SLA adherence ≥ 99%, Review Effort ↓ 30% |

## Milestone Ordering Check
- M0 prerequisites satisfied by alignment artifacts (this document, `alignment.md`, RFC/ADR templates, DoD).
- M1–M6 epics locked to milestone windows (Week 1–10), enabling incremental delivery while preserving trace continuity.

## Trace Summary
- #1 → EPIC-CORE (Command Router) → REQ-UX-01.
- #4 → EPIC-CORE (FlexGrid) → REQ-UX-02.
- #14 → EPIC-RESILIENCE (Local Resilience Cache) → REQ-REL-01.
- #27 → EPIC-SECURITY (Dynamic Secrets Injection) → REQ-SEC-03.
- #79 → EPIC-DELIVERY (Trusted Pipeline) → REQ-OPS-01.

## Next Actions
- Break down each epic into feature stories and technical tasks during respective milestones.
- Maintain trace updates alongside ADR revisions.
