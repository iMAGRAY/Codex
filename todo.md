# Stellar TUI Delivery Roadmap

## Roles
- Владелец: Пользователь
- Реализатор: GPT-5-codex | GPT-5

## Quick Start
1. Держим фокус: завершаем Workstream целиком прежде чем двигаться дальше.
2. Для каждой задачи двигаемся по схеме Inputs → Checklist → Outputs.
3. Любое изменение = обновлённый чекбокс и ссылка на артефакт (код, тест, документ).

## North Star — когнитивный экзоскелет для агента
- MCP должен автоматически предлагать нужные инструменты, подсказки и чеклисты.
- Любой сигнал (метрика, аудит, предупреждение) — контекстный и молчит, если всё хорошо.
- Документация и интерфейс живут в одном пространстве: агент «рождается» внутри MCP.
- Без регресса: каждое улучшение проходит полный DoD и усиливает текущий функционал.

## Quality Guardrails
- Каждый артефакт опирается на утверждённые RFC/ADR и имеет привязку к требованию из `MaxThink-Stellar.md` или `stellar-tui-vision.md`.
- Любая фича сопровождается unit, snapshot, security и observability проверками, перечисленными в Definition of Done.
- Метрики (APDEX, LATENCY, SEC-INC, MTTR и т.д.) фиксируются до начала работ и проверяются на фазе Validate.
- Fast-review пакет содержит ссылку на артефакты, чек-лист DoD и выписку метрик.

## Milestone Snapshot
| Milestone | Window | Focus | Exit Criteria | Primary Metrics |
| --- | --- | --- | --- | --- |
| M0 Alignment & RFC Kickoff | Week 0–1 | Зафиксировать сценарии, роли и архитектурные рамки (REQ-UX-01, REQ-ACC-01, REQ-SEC-01) | Одобрены RFC для ядра, кеша, секретов и pipeline; backlog пронумерован с trace на требования | METRIC-APDEX baseline, METRIC-CSAT baseline |
| M1 Stellar Core Kernel | Week 1–3 | Command Router, Keymap Engine, FlexGrid layout, Input Guard | Демонстрация навигации и Insight Canvas в TUI/CLI; unit+snapshot тесты зелёные | METRIC-APDEX ≤ 180 мс, METRIC-CSAT ≥ 4.5 |
| M2 Resilience & Data Intelligence | Week 3–5 | Local Resilience Cache, Conflict Resolver, Weighted Confidence, Predictive Prefetch | Chaos тесты без деградации, конфликты разрешаются интерактивно | METRIC-AVAIL ≥ 99.3%, METRIC-LATENCY 95p ≤ 200 мс |
| M3 Security & Sandbox Hardening | Week 4–6 | RBAC, Secure Signing, Dynamic Secrets, Sandbox Runner | Threat model закрыт, security review пройден, sandbox runner покрыт тестами | METRIC-SEC-INC = 0 критических, METRIC-AUDIT-OK ≥ 95% |
| M4 Observability & Support | Week 5–7 | Observability Mesh, Telemetry Overlay, Debug Orchestrator, Incident Timeline | Inline overlay и таймлайн работают в TUI, dry-run simulator даёт рекомендации | METRIC-MTTD ≤ 2 мин, METRIC-MTTR ≤ 15 мин |
| M5 Delivery & Governance | Week 6–8 | Trusted Pipeline, Governance Portal, Policy Validator, Marketplace guardrails | Подписанный pipeline выпускает модули, Governance portal показывает состояние | METRIC-EXT-ADOPT ≥ 60%, METRIC-AVAIL ≥ 99.5% |
| M6 Launch & Adoption | Week 8–10 | Полный regression, документация, rollout & training | Документация и training path опубликованы, release notes доставлены | SLA adherence ≥ 99%, Review Effort ↓ 30% |

## Workstream 0 – Alignment & RFCs (Week 0–1)
**Objective**: Выстроить единый каркас архитектуры и качества перед разработкой ядра.

**Inputs**: `MaxThink-Stellar.md`, `stellar-tui-vision.md`, существующие сценарии, требования REQ-UX-01/REQ-ACC-01/REQ-SEC-01.

**Outputs**: Утверждённые RFC (Core, Resilience, Security, Delivery), шаблон ADR, обновлённый backlog с trace.

**Checklist**
- [x] PLAN — Подтвердить пользовательские сценарии, RBAC-матрицу и assistive требования.
- [x] PLAN — Разбить backlog на EPIC-и с trace к #1, #4, #14, #27, #79 и ключевым REQ.
- [x] PLAN — Подготовить `RFC-STELLAR-CORE`, `RFC-STELLAR-RESILIENCE`, `RFC-STELLAR-SECURITY`, `RFC-STELLAR-DELIVERY` с ADR-черновиками.
- [x] BUILD — Настроить единый шаблон RFC/ADR (goal, scope, constraints, metrics, trace anchors).
- [x] BUILD — Согласовать Definition of Done для TUI/CLI фич (unit, snapshot, accessibility, security, observability чек-листы).
- [x] VALIDATE — Провести совместное ревью с Core, SecOps, SRE, DX; зафиксировать протокол рисков.
- [x] VALIDATE — Обновить `docs/future/` индексом на утверждённые RFC и отметить baseline метрик.

## Workstream 1 – Stellar Core Kernel & UX (Week 1–3)
**Objective**: Доставить управляемое ядро TUI/CLI с Insight Canvas и базовыми защитами ввода.

**Inputs**: RFC-STELLAR-CORE, keymap draft, FlexGrid спецификация, REQ-UX-01/02, REQ-ACC-01.

**Outputs**: Рабочий Stellar TUI Core kernel, Insight Canvas, CLI паритет, baseline APDEX.

**Checklist**
- [x] PLAN — Утвердить keymap и командные маршруты с привязкой к персонae.
- [x] PLAN — Зафиксировать FlexGrid макеты и fallback для узких терминалов (#4, #63).
- [x] PLAN — Проработать UX microcopy и Golden Path/Progressive Disclosure хинты.
- [x] BUILD — Реализовать Command Router, Keymap Engine и state machine для экранов/hotkeys (#1, #2, #3).
- [x] BUILD — Добавить Input Guard с валидацией и безопасными автодополнениями (REQ-UX-01, #56, #18) + CLI эквивалент.
- [x] BUILD — Собрать Insight Canvas (поле, предложение, confidence, reason) с Progressive Disclosure (`Enter`/`i`), Confidence Bar, Field Lock, Undo/Redo.
- [x] BUILD — Добавить Golden Path footer (≤ 2 действия) и microcopy coach.
- [x] VALIDATE — Выполнить `cargo test -p codex-tui`, unit тесты keymap/undo/redo/field lock.
- [x] VALIDATE — Снять snapshot тесты (широкий/узкий терминал), accessibility smoke (screen reader текст, контраст FlexGrid).
- [x] VALIDATE — Зафиксировать baseline APDEX/latency и обновить метрики.

## Workstream 2 – Resilience & Data Intelligence (Week 3–5)
**Objective**: Обеспечить устойчивость офлайн, конфликт-резолвер и объяснимый скоуп доверия данных.

**Inputs**: RFC-STELLAR-RESILIENCE, REQ-REL-01, REQ-DATA-01, кеш/очереди дизайн.

**Outputs**: Local Resilience Cache, Resilient Transport, Weighted Confidence ядро, Intent Conflict Resolver UI.

**Checklist**
- [x] PLAN — Спроектировать Local Resilience Cache (TTL, eviction) и Resilient Transport (retry/queue) (#14, #35, #71).
- [x] PLAN — Описать Intent Conflict Resolver API и TUI/CLI контракт (#15).
- [x] PLAN — Согласовать схему Weighted Confidence scoring с trace на reason codes/telemetry (#16, #67).
- [x] BUILD — Имплементировать кеш, offline очереди и Predictive Prefetch для каталогов и knowledge packs (#32).
- [x] BUILD — Собрать Weighted Confidence ядро с объяснениями и интегрировать в Insight Canvas/CLI.
- [x] BUILD — Подключить модульный реестр детекторов с hot reload; обновить Signal Cache (path+mtime).
- [x] BUILD — Встроить Inline Risk Alerts и Conflict Resolver UI/CLI.
- [x] VALIDATE — Провести chaos тесты (network drop, высокий latency) и подтвердить устойчивость ≥10 мин офлайн.
- [x] VALIDATE — Запустить Criterion бенчмарки (50/200 файлов, архивы) и логировать cache hit/latency.
- [x] VALIDATE — Выполнить integration тесты `codex mcp wizard --source fixtures/*` (успех и ошибки).

## Workstream 3 – Security & Sandbox Hardening (Week 4–6)
**Objective**: Закрыть ключевые угрозы, укрепить sandbox и обеспечить аудит без когнитивного шума.

**Inputs**: RFC-STELLAR-SECURITY, `docs/future/stellar/m3-security-plan.md`, threat model, REQ-SEC-01/02/03, D5/A9.

**Outputs** (все выполнены, артефакты зафиксированы):
- RBAC + Secure Signing: `codex-rs/cli/src/mcp_cmd.rs`, тесты `codex-rs/cli/tests/*.rs`.
- Dynamic Secrets + Sandbox Runner + Resource Shield: `codex-rs/core/src/security/mod.rs`, `codex-rs/core/src/exec.rs`.
- Immutable Audit Ledger + export + policy evidence log: `codex-rs/cli/src/audit_cmd.rs`, документация `docs/future/stellar/m3-security-validation.md`.
- Полный DoD: `cargo test -p codex-core -- --test-threads=1`, `cargo test -p codex-cli`.

**Checklist**
- [x] PLAN — Threat modeling (RBAC, секреты, supply chain) и mitigations (#9, #11, #27, #70).
- [x] PLAN — Sandbox Runner + Manifest Write Shield дизайн (bubblewrap/nsjail) с guardrails.
- [x] PLAN — Compliance Pre-flight checklist и Consent Banner copy.
- [x] BUILD — RBAC фильтрация команд и Secure Command Signing (REQ-SEC-01, #74).
- [x] BUILD — Dynamic Secrets Injection, Secure Clipboard Redaction, Secret Scrubber, consent logging (#27, #88, #92).
- [x] BUILD — Sandbox Runner + Resource Shield (CPU/RAM/Time), Restricted Capability warnings, offline archive integrity check (A9, D5).
- [x] BUILD — Immutable Audit Ledger, Audit Export, Policy Evidence Log (TTL 24 ч) + fallback метрики.
- [x] VALIDATE — Unit + integration тесты и кросс-платформенный security review.
- [x] VALIDATE — Pen-test dry run (sandbox escape, подписанные команды, revoke).
- [x] VALIDATE — Compliance report и audit trail traceability (`docs/future/stellar/m3-security-validation.md`).

## Workstream 4 – Observability Lite (Week 5–7)
**Objective**: Статус-бар показывает главное, а агент одним жестом попадает в нужные инструменты.

**Inputs**: RFC-STELLAR-CORE/RESILIENCE/SECURITY, `docs/future/stellar/m3-security-validation.md`, REQ-OBS-01, REQ-PERF-01, REQ-OPS-01.

**Outputs**: минималистичный overlay, OTLP+Prometheus адаптер с буфером, горячая кнопка Investigate, обновлённые help/quickstart.

**Checklist (параллельные дорожки)**
- **Agent A — Overlay Lead**
  - [x] BUILD — Статус-бар overlay (latency p95, audit_fallback_count, cache hit %) с цветовой индикацией и клавишей Ctrl+O (codex-rs/tui/src/status_bar.rs, codex-rs/tui/src/chatwidget.rs — REQ-OBS-01/REQ-OPS-01).
  - [x] BUILD — Лёгкий OTLP адаптер с локальным sled-буфером и опциональным Prometheus endpoint (codex-rs/core/src/telemetry.rs, codex-rs/core/src/telemetry_exporter.rs — REQ-OBS-01, REQ-OPS-01).
  - [x] VALIDATE — Unit/snapshot тесты, baseline метрики (`cargo test -p codex-tui ctrl_o_toggles_telemetry_overlay`; `~/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin/cargo test -p codex-core telemetry_exporter::tests::flush_once_sends_payload_and_clears_sled telemetry_exporter::tests::prometheus_endpoint_exposes_latest_snapshot`; docs/future/stellar/metrics-baseline.md).
- **Agent B — UX Integrator**
  - [x] BUILD — Связка overlay ↔ orchestrатор (кнопка «Investigate», persona hints) (codex-rs/tui/src/stellar/view.rs, codex-rs/core/src/stellar/state.rs — REQ-OBS-01/REQ-OPS-01).
  - [x] BUILD — Обновить help, `docs/stellar-quickstart.md`, включить навигационные подсказки (docs/getting-started.md, docs/stellar-quickstart.md, codex-rs/tui/src/stellar/keymap.rs).
  - [x] VALIDATE — UX smoke (overlay toggle, help доступна из CLI/TUI) (`cargo test -p codex-tui ctrl_o_toggles_telemetry_overlay`; doc walkthrough docs/stellar-quickstart.md; CLI parity codex-rs/cli/src/stellar_cmd.rs).

## Workstream 5 – Signed Pipeline Lite (Week 6–8)
**Objective**: Любой knowledge pack проверен и безопасен без тяжёлой инфраструктуры.

**Inputs**: RFC-STELLAR-DELIVERY, `docs/stellar-quickstart.md`, REQ-OPS-01, REQ-INT-01, REQ-DX-01.

**Outputs**: CLI-подпись/проверка/rollback, запись в audit ledger, прозрачный diff версий.

**Checklist (параллельные дорожки)**
- **Agent A — Pipeline Core**
  - [x] BUILD — `codex pipeline sign` (Sigstore/cosign + Vault ключи) с audit записью (`codex-rs/core/src/pipeline/mod.rs`, `codex-rs/cli/src/pipeline_cmd.rs`; AuditEventKind::SupplyChain расширен, REQ-OPS-01/REQ-INT-01/REQ-DX-01).
  - [x] BUILD — `codex pipeline rollback --version` (реактивация установленной версии с аудитом) (`codex-rs/core/src/pipeline/mod.rs`, `codex-rs/cli/src/pipeline_cmd.rs`).
  - [x] VALIDATE — Unit + smoke тесты (`~/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin/cargo test -p codex-core pipeline::tests::sign_verify_install_and_rollback_flow -- --nocapture`; `~/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin/cargo test -p codex-cli -- --nocapture`).
- **Agent B — Verification & Docs**
  - [x] BUILD — `codex pipeline verify` при установке knowledge pack + diff версий (payload достоверность, diff added/removed/modified) (`codex-rs/core/src/pipeline/mod.rs`, `codex-rs/cli/src/pipeline_cmd.rs`).
  - [x] BUILD — Обновить quickstart/help + встроенную подсказку `?` (`docs/stellar-quickstart.md`, `docs/getting-started.md`; ссылка на новые команды и UX подсказки).
  - [x] VALIDATE — Док-трейс: подпись → verify → diff → rollback задокументированы (см. `docs/stellar-quickstart.md`, `docs/future/stellar/metrics-baseline.md`), fast-review пакет может ссылаться на эти артефакты и тесты.

## Workstream 6 – Guided Experience (Week 8–10)
**Objective**: Агент работает как дома: один сценарий расследования, быстрый старт и мгновенная обратная связь.

**Inputs**: Workstreams 0–5, REQ-ACC-01, SLA.

**Outputs**: Orchestrator `/investigate`, `/quickstart` и embedded help, `/feedback` с метриками.

**Checklist (параллельные дорожки)**
- **Agent A — Orchestrator**
  - [x] BUILD — Оркестратор `/investigate` (checklist, dry-run, summary, audit log) (`codex-rs/core/src/orchestrator/mod.rs`, `codex-rs/cli/src/orchestrator_cmd.rs`).
  - [x] VALIDATE — Интеграционный тест `overlay_investigate_flow` (`codex-rs/tui/tests/suite/overlay_investigate_flow.rs`).
- **Agent B — Onboarding & Feedback**
  - [x] BUILD — `/quickstart` + embedded help `?` (overlay/orchestrator/pipeline) (`codex-rs/tui/src/app.rs`, `codex-rs/core/src/orchestrator/mod.rs`).
  - [x] BUILD — `/feedback` с автоматическим приложением latency p95, audit_fallback_count, Review Effort (`codex-rs/core/src/orchestrator/mod.rs`, `codex-rs/cli/src/orchestrator_cmd.rs`).
  - [x] VALIDATE — Обновлённый `docs/stellar-quickstart.md`, `docs/getting-started.md`, `docs/future/stellar/metrics-baseline.md`; CLI/тесты (`~/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin/cargo test -p codex-cli orchestrator_cmd::tests`, `~/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin/cargo test -p codex-tui overlay_investigate_flow`).

## Continuous Quality Safeguards
- [ ] Еженедельный triage (APDEX, latency, audit_fallback_count, Review Effort) + обновление чеклистов.
- [ ] Автосмоки: insight review, sandbox exec, pipeline release, governance portal, overlay telemetry.
- [ ] Плановые обновления knowledge packs + policy sync с зафиксированным rollback и audit ссылками.
- [ ] Quarterly: pen-test, accessibility audit, performance benchmark (недели 10/12/15) + отчёты в portal.
- [ ] RFC/ADR hygiene: мгновенное обновление после архитектурных изменений, линк в `todo.md`.

## Traceability Index
- REQ-UX-01/02 → Workstreams 1, 4.
- REQ-ACC-01 → Workstreams 0, 1, 6.
- REQ-SEC-01/02/03 → Workstream 3.
- REQ-PERF-01 → Workstreams 1, 2, 4.
- REQ-REL-01 → Workstream 2.
- REQ-OBS-01 → Workstream 4.
- REQ-OPS-01 → Workstreams 4, 5.
- REQ-DATA-01 → Workstream 2.
- REQ-INT-01 → Workstream 5.
- REQ-DX-01 → Workstream 5.

Все пункты поддерживают когнитивно лёгкий обзор: Inputs → Checklist → Outputs + привязка к требованиям, что позволяет ИИ-агенту и ревьюерам моментально видеть статус и артефакты без потери качества и функциональности.
