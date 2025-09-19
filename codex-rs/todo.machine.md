# MCP Management Overhaul – Phase 3 (Wizard & UX)

## Objective
Ship a feature-flagged MCP wizard and dashboard that meet these success criteria:
- ≥95% of guided CLI/TUI sessions reach a valid config within 5 minutes (measured via telemetry or manual timing).
- Health check command returns a structured result for every configured server.
- CLI/TUI/JSON outputs stay in sync (no divergence bugs in manual QA).
- No plaintext secrets written to disk during wizard flows.

## Preconditions
- Schema extensions, migrations, and CLI migrate command ✅
- `experimental.mcp_overhaul` flag wired into config ✅

## Deliverables
1. **Templates & Registry (Success when…)**
   - `resources/mcp_templates/*.json` exist with schema validation.
   - `codex_core::mcp::registry` supports create/update/delete/list using templates.
   - Policy hooks (command allowlist stub + env warning) execute during registry ops.

2. **CLI Wizard (Success when…)**
   - `codex mcp wizard` (flagged) walks through template selection, validation, preview, final apply.
   - `codex mcp add --template … --set …` works headless and writes identical config.
   - `codex mcp list/get` show health summary when flag on.
   - Non-interactive wizard: `codex mcp wizard --name foo --command bar --apply` persists entry via registry.
   - Interactive wizard after confirmation writes entry and re-runs summary on success.
   - `--json` path returns machine summary without side effects.

3. **TUI Panel (Success when…)**
   - New panel lists servers + status.
   - Wizard modal mirrors CLI flow; snapshot tests updated.

4. **Health Probe Stub (Success when…)**
   - `codex mcp test <name> [--json]` returns cached/placeholder status without panic.

5. **Automation Hooks (Success when…)**
   - `codex mcp plan --json` emits validation summary suitable for CI.

6. **Documentation & Guardrails (Success when…)**
   - `docs/config.md` updated with flag instructions + wizard quickstart.
   - CLI help (`--help`) references experimental gate.
   - Running wizard/test without flag yields explicit guidance.

## Validation Checklist
- Unit tests: template parsing, registry validation, wizard step transitions.
- CLI integration tests: `codex mcp wizard --json`, apply path writes config.
- TUI snapshot: panel, wizard flows.
- Manual QA: CLI happy path + failure, TUI flow, automation commands.
- Telemetry/mock timing: confirm ≤5 min setup goal (manual timing if telemetry absent).
- Secrets audit: ensure wizard never leaves secrets in plain config.

## Progress — 2025-09-17
- TUI manager modal + wizard flow implemented (`codex-rs/tui/src/mcp/*`).
- App events for open/apply/reload/remove integrated with `McpRegistry` in `tui/src/app.rs`.
- Snapshot test scaffolding for manager & wizard views added (pending `cargo insta accept`).
- Health status stub exposed via `codex_core::mcp::health` and surfaced in manager list.

- id: codex-gui-mvp
  title: Codex Desktop GUI Shell MVP
  type: feature
  status: done
  priority: P0
  size_points: 13
  scope_paths:
    - path: codex-rs/gui
    - path: codex-rs/Cargo.toml
    - path: docs/codex-gui.md
  spec:
    given: CLI-only Codex interaction with REQ-UX-01 and REQ-ACC-01 still unmet for desktop shell
    when: пользователь запускает бинарь `codex-gui` с включённым experimental десктопным режимом
    then: окно в стиле белой минималистичной панели отображает историю, редактор и панель команд, проксируя действия в codex-core
  budgets:
    latency_ms_p95: 120
    memory_mb_peak: 512
  risks:
    - gui-framework-learning-curve
    - integration-regressions-across-platforms
    - accessibility-parity-vs-tui
  dependencies:
    - codex-core-stable-api
  tests_required:
    - cargo test -p codex-gui
    - cargo run -p codex-gui -- --smoke
  verify_commands:
    - cargo check -p codex-gui
    - cargo test -p codex-gui
  rollback:
    commands:
      - git revert <commit>
  docs_updates:
    - docs/codex-gui.md
    - docs/stellar-quickstart.md
  artifacts:
    - codex-gui binary
  audit:
    created_at: 2025-09-19T00:00:00Z
    created_by: GPT-5-codex
    updated_at: 2025-09-19T00:30:00Z
    updated_by: GPT-5-codex

- id: codex-gui-ga
  title: Codex Desktop GUI GA Readiness
  type: feature
  status: done
  priority: P0
  size_points: 21
  scope_paths:
    - path: codex-rs/gui
    - path: codex-rs/core
    - path: docs/codex-gui.md
  spec:
    given: Codex GUI использует моковый backend и не покрывает весь сценарий CLI/TUI
    when: пользователь запускает десктопный клиент и входит под своей учётной записью
    then: реализованы живые ответы Codex, команда/файловые операции, конфигурация и сохранение истории софта
  budgets:
    latency_ms_p95: 180
    memory_mb_peak: 768
  risks:
    - async-runtime-integration
    - cross-platform-windowing
    - security-context-sharing
  dependencies:
    - codex-core session manager
    - auth/oauth flow
  tests_required:
    - cargo test -p codex-gui
    - cargo test -p codex-core conversation_manager::tests
  verify_commands:
    - cargo check -p codex-gui
    - cargo run -p codex-gui -- --dry-run-ui
  rollback:
    commands:
      - git revert <commit>
  docs_updates:
    - docs/codex-gui.md
    - docs/stellar-quickstart.md
  artifacts:
    - codex-gui binary
  audit:
    created_at: 2025-09-19T01:00:00Z
    created_by: GPT-5-codex
    updated_at: 2025-09-19T02:30:00Z
    updated_by: GPT-5-codex

- id: codex-tui-file-explorer
  title: Stellar TUI Explorer Panel
  type: feature
  status: done
  priority: P0
  size_points: 13
  scope_paths:
    - path: codex-rs/tui
  spec:
    given: Stellar TUI lacks a persistent project navigation surface (REQ-UX-01 from `docs/future/MaxThink-Stellar.md`)
    when: оператор запускает `codex tui` в рабочем каталоге с файлами
    then: слева отображается IDE-подобный файловый менеджер с возможностью навигации и предпросмотра
  budgets:
    latency_ms_p95: 80
    memory_mb_peak: 128
  risks:
    - tree-scan-latency-on-large-repos
    - focus-regression-between-panes
  dependencies:
    - codex-core session manager
  tests_required:
    - cargo test -p codex-tui
  verify_commands:
    - cargo check -p codex-tui
    - cargo test -p codex-tui
  rollback:
    commands:
      - git revert <commit>
  docs_updates:
    - docs/stellar-quickstart.md
    - docs/future/stellar-tui-vision.md
  artifacts:
    - codex-tui binary
  audit:
    created_at: 2025-09-19T15:15:00Z
    created_by: GPT-5-codex
    updated_at: 2025-09-19T20:30:00Z
    updated_by: GPT-5-codex

- id: apply-patch-ux-revamp
  title: Apply Patch Interactive UX Revamp
  type: feature
  status: blocked
  blocked_reason: rustc aborts with ENOMEM inside sandbox; cannot run cargo fmt/test yet
  blocked_recommendations:
    - bump sandbox memory limit or rerun outside sandbox, then run `cargo fmt` and `cargo test -p codex-apply-patch`
  priority: P0
  size_points: 21
  scope_paths:
    - path: codex-rs/apply-patch/src
    - path: codex-rs/apply-patch/tests
    - path: codex-rs/apply-patch/apply_patch_tool_instructions.md
  spec:
    given: линейный apply_patch без интерактивности и гибкого контроля
    when: оператор запускает обновлённый apply_patch в TTY окружении с патчем
    then: инструмент предлагает предпросмотр, выбор хунков, dry-run и post-hook, сохраняя совместимость со скриптами
  budgets:
    latency_ms_p95: 120
    memory_mb_peak: 64
  risks:
    - regression-in-noninteractive-workflows
    - cross-platform-terminal-behavior
    - partial-apply-state-drift
  dependencies:
    - codex-apply-patch-core
  tests_required:
    - cargo test -p codex-apply-patch
  verify_commands:
    - cargo test -p codex-apply-patch
    - cargo fmt
  rollback:
    commands:
      - git revert <commit>
  docs_updates:
    - codex-rs/apply-patch/apply_patch_tool_instructions.md
  artifacts:
    - apply_patch binary
  audit:
    created_at: 2025-09-19T16:00:00Z
    created_by: GPT-5-codex
    updated_at: 2025-09-19T17:45:00Z
    updated_by: GPT-5-codex
- id: codex-docsearch
  title: Docsearch powered by EmbeddingGemma
  type: tooling
  status: done
  priority: P1
  size_points: 8
  scope_paths:
    - path: scripts/docsearch
    - path: codex-rs/cli/src/doc_cmd.rs
    - path: docs/getting-started.md
  spec:
    given: Агент тратит время на поиск по документации через grep
    when: пользователь запускает `codex doc search`
    then: выдаются релевантные чанки документации на основе EmbeddingGemma
  budgets:
    latency_ms_p95: 500
    memory_mb_peak: 1024
  risks:
    - python-dependency-drift
    - large-docs-footprint
  dependencies:
    - embeddinggemma-300m-model
  tests_required:
    - python -m scripts.docsearch.query --help
  verify_commands:
    - codex doc index --help
    - codex doc search --help
  rollback:
    commands:
      - git revert <commit>
  docs_updates:
    - docs/getting-started.md
    - docs/codex-gui.md
  artifacts:
    - scripts/docsearch
  audit:
    created_at: 2025-09-19T03:10:00Z
    created_by: GPT-5-codex
    updated_at: 2025-09-19T03:10:00Z
    updated_by: GPT-5-codex
