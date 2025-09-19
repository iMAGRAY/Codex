# Stellar Quickstart — Observability Overlay

Trace: REQ-OBS-01, REQ-OPS-01 (see `docs/future/MaxThink-Stellar.md` §"Observability & SRE Controls") · aligns with `docs/future/stellar-tui-vision.md` overlay narrative (#8, #20).

## Plan
- Confirm telemetry storage path (`$CODEX_HOME/telemetry`) is writable; OTLP/Prom endpoints optional.
- Capture current METRIC-LATENCY p95 baseline (table in `docs/future/stellar/metrics-baseline.md`).
- Decide persona focus (Operator, SRE, SecOps, Platform, Partner, Assistive) to tailor investigate hints.

## Build
- Launch TUI (`codex`) and press `Ctrl+O` to toggle the Observability Overlay.
- Read latency/audit/cache indicators in the status bar (color-coded, `StatusBar` widget; REQ-OBS-01).
- From the overlay, use `[ Investigate ]` hint → `Ctrl+R` to open persona-specific runbooks (REQ-OPS-01).
- Configure OTLP export by setting `CODEX_TELEMETRY_OTLP_ENDPOINT` and optional `CODEX_TELEMETRY_OTLP_HEADERS` (Sigstore/Vault ready).
- Expose Prometheus metrics by setting `CODEX_TELEMETRY_PROMETHEUS_ADDR=127.0.0.1:9464` (sled-backed buffer ensures no data loss offline).

## Validate
- `cargo test -p codex-tui stellar::ctrl_o_toggles_telemetry_overlay` (ensures UI toggle + snapshot coverage).
- `cargo test -p codex-core telemetry_exporter::tests::` (OTLP flush + Prometheus endpoint smoke per REQ-OBS-01/REQ-OPS-01).
- Record updated latency/audit/cache readings post-run in `docs/future/stellar/metrics-baseline.md` (target p95 ≤ 200 мс, METRIC-AUDIT-OK ≥ 95%).

# Stellar Quickstart — Signed Pipeline Lite

Trace: REQ-OPS-01, REQ-INT-01, REQ-DX-01 (ref. `docs/rfcs/0005-stellar-delivery.md`, `docs/future/stellar/adrs/adr-del-001.md`). Implements Workstream 5 checklist.

## Plan
- Export/seed `CODEX_PIPELINE_SIGNING_KEY` with a base64url Ed25519 secret (Vault integration ready). Document signer alias (e.g. `vault:pipeline/insight`) and required approval chain.
- Define semantic versioning policy (`MAJOR.MINOR.PATCH`) and rollback matrix for each knowledge pack.
- Ensure audit ledger writable (`$CODEX_HOME/audit-ledger`) and pipeline store baseline (`$CODEX_HOME/pipeline`) has 1.5× free disk space of bundle size.

## Build
- Подпишите knowledge pack каталог:
  - ``codex pipeline sign --name insight --version 1.4.0 --source packs/insight --signer vault:pipeline/insight`` → bundle и манифест появляются в `$CODEX_HOME/pipeline` + аудит `supply_chain` (REQ-OPS-01).
- Проверьте и установите бандл на приёмнике:
  - ``codex pipeline verify dist/insight-1.4.0.tar.gz --expect-fingerprint <HEX> --install`` → валидация подписи, дифф к активной версии, развёртывание в `installed/` (REQ-INT-01).
- При необходимости откатитесь:
  - ``codex pipeline rollback insight 1.3.5`` → смещает активную версию, фиксируя событие в immutable audit ledger (REQ-DX-01).

## Validate
- `cargo test -p codex-core pipeline::tests::sign_verify_install_and_rollback_flow` (unit coverage sign/verify/rollback, bundle diff).
- `cargo test -p codex-cli` (CLI smoke; автоматическая проверка парсинга и вывода новых подкоманд).
- Док-трейс: обновите `docs/future/stellar/metrics-baseline.md` (раздел Delivery) фактами по bundle fingerprint/rollback, заархивируйте diff в fast-review пакете и отметьте прогресс в `todo.md` (Workstream 5, оба агента).

# Stellar Quickstart — Weekly Triage

Trace: Continuous Quality Safeguards · REQ-ACC-01/REQ-OPS-01 alignment with Workstream 6 outputs.

## Plan
- Зафиксируйте целевые значения: APDEX ≥ 0.85, latency p95 ≤ 200 мс, audit fallback = 0, review effort ≤ 4.5 ч.
- Перед запуском убедитесь, что TelemetryHub содержит свежие снапшоты (прогоните представительные команды/тесты).

## Build
- Сгенерируйте еженедельный отчёт: ``codex orchestrator triage --persona operator --review-hours 5.0`` — команда выводит статус по каждому метрику и предлагает обновления чеклистов.
- При необходимости скорректируйте цели (`--apdex-target`, `--latency-target-ms`, `--audit-target`, `--review-target-hours`).
- Зафиксируйте изменения чеклистов/действий в governance portal и `todo.md`.

## Validate
- Архивируйте вывод triage в `docs/future/stellar/metrics-baseline.md` (раздел Continuous Quality) и отметьте чекбокс в `todo.md`.
- При статусе Yellow/Red запланируйте `codex orchestrator investigate` и добавьте action items в weekly review.

# Stellar Quickstart — Desktop GUI Preview

Trace: REQ-UX-01, REQ-ACC-01, REQ-DX-01. Инкрементирует Workstream 0→1 для десктопного канала.

## Plan
- Убедитесь, что `rustup` настроен на toolchain 2024 (`rustup show active-toolchain`).
- Подготовьте чистую рабочую директорию (`git status` без незакоммиченных правок) и отключите внешние окна — eframe запускает нативное приложение.
- Выполните `cargo run -p codex-gui -- --dry-run-ui` — проверяет, что runtime и конфигурация собираются без открытия окна.
- Убедитесь, что авторизация Codex активна (`codex login status` или `codex login` при необходимости).

## Build
- `cargo run -p codex-gui` — запускает белый минималистичный интерфейс с live-подключением к Codex.
- В интерфейсе создайте/откройте сессию, отправьте промпт (⌘/Ctrl + Enter) и дождитесь потокового ответа ассистента.
- Запустите команду из подсказки (например, попросите проанализировать `git status`) и убедитесь, что stdout/stderr появляются в истории с пометками.
- Проверьте визуальные элементы: белый фон, подсказки горячих клавиш, статус-баннеры, плавные карточки.

## Validate
- `cargo test -p codex-gui` (smoke `tests/smoke.rs` + backend unit).
- `cargo fmt -p codex-gui` (форматирование перед PR) или `just fmt` для всего workspace.
- На окружении Codex CLI команды `cargo` могут завершаться сигналом; при локальной проверке убедитесь, что окно поднимается < 200 мс, RAM < 512 МБ, а команды/ответы транслируются без задержек > 1 с.
