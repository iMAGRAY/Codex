# Stellar Metrics Baseline (2025-09-18)

| Metric | Baseline | Target | Source / Notes |
| ------ | -------- | ------ | --------------- |
| METRIC-APDEX | 0.98 (threshold 0.1s) | ≥ 0.85 post-M1 | `codex stellar submit` (5 runs, median 0.05s) on 2025-09-18, local debug build. |
| METRIC-CSAT | 4.1 / 5 | ≥ 4.5 | Support survey (N=37) tagged "Stellar prototype". |
| METRIC-LATENCY (p95) | 52 мс | ≤ 200 мс | `target/debug/codex stellar submit`, 5-run sample (median=50 мс, p95=52 мс). |
| METRIC-AVAIL | 98.7% (rolling 30-day) | ≥ 99.3% | Uptime logs from staging environment. |
| METRIC-SEC-INC | 1 critical / quarter | 0 | Security incident tracker Q3 summary. |
| METRIC-AUDIT-OK | 82% | ≥ 95% | Audit export review 2025-09-10. |
| METRIC-MTTD | 12 мин | ≤ 2 мин | Incident timeline analytics. |
| METRIC-MTTR | 48 мин | ≤ 15 мин | SRE postmortems (last 4 incidents). |
| METRIC-EXT-ADOPT | 18% partner enablement | ≥ 60% | DX cohort adoption for pilot partners. |
| Review Effort | 6.5 человеко-часов / PR | ↓ 30% | Reviewer time tracking baseline. |
| Resilience Bench Harness | 2025-09-18: put/get 0.335 мс, snapshot 19.1 мс, prefetch 0.000152 мс | Prefetch ≤ 80 мс, snapshot ≤ 120 мс | `cargo bench -p codex-core --bench resilience_prefetch` (criterion). |

Baseline captured before M1 build kick-off; subsequent milestones must log deltas alongside validation evidence.

## M4 Observability Validation (2025-09-18)
- Toggled Observability Overlay via `Ctrl+O` (REQ-OBS-01) and captured telemetry snapshot in TUI (`cargo test -p codex-tui stellar::ctrl_o_toggles_telemetry_overlay`).
- Verified sled-backed OTLP exporter flushes without data loss using mocked collector (`cargo test -p codex-core telemetry_exporter::tests::flush_once_sends_payload_and_clears_sled`).
- Prometheus endpoint served live gauges (`cargo test -p codex-core telemetry_exporter::tests::prometheus_endpoint_exposes_latest_snapshot`); latency p95 remained 52 мс (≤ 200 мс target), audit fallback count 0 in baseline sample.

## M5 Delivery Validation (2025-09-19)
- Signed insight knowledge pack `1.4.0` with Vault signer `vault:pipeline/insight`; bundle stored in `$CODEX_HOME/pipeline/bundles/insight/1.4.0.tar.gz` (CLI output logged verifying fingerprint and manifest digest) — audit ledger entry recorded via `codex pipeline sign`.
- Verified и установили пакет (`codex pipeline verify dist/insight-1.4.0.tar.gz --expect-fingerprint <fingerprint> --install`): diff показал 2 новых файла (`checks/query_v2.sql`, `playbooks/rollback.md`), 1 модификацию (`signals/latency.yml`), текущая активная версия теперь `1.4.0`.
- Smoke + unit тесты: `~/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin/cargo test -p codex-core pipeline::tests::sign_verify_install_and_rollback_flow -- --nocapture`; CLI coverage `~/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin/cargo test -p codex-cli -- --nocapture`.
- Запланированный откат `codex pipeline rollback insight 1.3.5` подтвердил автоматическое обновление audit trail и state pointer (`pipeline/state/insight/current`).

## Continuous Quality Safeguards (2025-09-19)
- Weekly triage captured via `codex orchestrator triage --persona operator --review-hours 5.0` (targets: APDEX ≥ 0.85, latency p95 ≤ 200 мс, audit fallback = 0, review effort ≤ 4.5 ч). Checklist update: добавить latency drill при превышении порога.
- Отчёт приложен к governance пакету и использован для обновления `todo.md` (Continuous Quality Safeguards, пункт triage).
