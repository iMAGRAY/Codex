# Stellar Definition of Done (TUI/CLI)

## Scope
Applies to Stellar TUI and CLI features across milestones M1–M6. Anchored to `docs/future/MaxThink-Stellar.md` requirements and roadmap metrics.

## Checklist
- [ ] **Unit Coverage** — Critical logic has ≥90% line coverage with failure mode tests (REQ-UX-01, REQ-REL-01, REQ-SEC-03). Include `pretty_assertions::assert_eq` in Rust tests.
- [ ] **Snapshot Integrity** — TUI components capture updated Insta snapshots; review pending snapshots (`cargo insta pending-snapshots -p codex-tui`) (REQ-UX-02, REQ-ACC-01).
- [ ] **Security Controls** — Secrets redaction, RBAC enforcement, signed artifacts validated; run static analysis and dependency audit (REQ-SEC-01/02/03, #9, #27, #57).
- [ ] **Observability Hooks** — Emit OpenTelemetry spans and metrics for latency, cache hit, error rate; dashboards updated (REQ-OBS-01, REQ-PERF-01, #8, #17).
- [ ] **Accessibility Compliance** — Screen reader landmarks, keyboard paths, contrast verification, focus order tests executed (`REQ-ACC-01`, `stellar-tui-vision.md`).
- [ ] **Performance Guardrails** — Benchmark ensures APDEX ≤ 180 мс, latency p95 ≤ 200 мс where relevant (METRIC-APDEX, METRIC-LATENCY).
- [ ] **Documentation & Trace** — Update RFC/ADR status, link requirement trace tables, refresh runbooks/checklists (REQ-OPS-01, REQ-DX-01, #24, #79).
- [ ] **Rollback Ready** — Provide rollback steps or feature flag toggles with verification plan (REQ-REL-01, REQ-OPS-01).

## Evidence Guidelines
- Attach test reports (unit, snapshot, security, observability) to fast-review package.
- Record metrics deltas in `docs/future/stellar/metrics.md` (created during Validate step).
- Ensure task tracker entries link to EPIC IDs from `backlog.md` and requirement IDs.
