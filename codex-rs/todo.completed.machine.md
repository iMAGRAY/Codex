- id: codex-gui-mvp
  completed_at: 2025-09-19T00:30:00Z
  commits: []
  measures:
    latency_ms_p95: null
    memory_mb_peak: null
  test_summary:
    cargo_check: "не запускалось — окружение CLI завершает cargo сигналом"
    cargo_run_dry: "не запускалось — окружение CLI завершает cargo сигналом"
    cargo_test: "не запускалось — окружение CLI завершает cargo сигналом"
  verify_commands_ran: []
  docs_updated:
    - docs/codex-gui.md
    - docs/stellar-quickstart.md
  handoff:
    verify_steps:
      - cargo check -p codex-gui
      - cargo run -p codex-gui -- --dry-run-ui
      - cargo test -p codex-gui
    rollback_steps:
      - git revert <commit_sha>
  notes: |
    Моковый backend и eframe UI добавлены. Локально требуется прогнать команды проверки,
    так как в среде Codex CLI `cargo` завершается сигналом.
- id: codex-gui-ga
  completed_at: 2025-09-19T02:30:00Z
  commits: []
  measures:
    latency_ms_p95: null
    memory_mb_peak: null
  test_summary:
    cargo_check: "не запускалось — среда CLI завершается сигналом"
    cargo_run: "не запускалось — среда CLI завершается сигналом"
    cargo_test: "не запускалось — среда CLI завершается сигналом"
  verify_commands_ran: []
  docs_updated:
    - docs/codex-gui.md
    - docs/stellar-quickstart.md
  handoff:
    verify_steps:
      - cargo check -p codex-gui
      - cargo run -p codex-gui -- --dry-run-ui
      - cargo run -p codex-gui
      - cargo test -p codex-gui
    rollback_steps:
      - git revert <commit_sha>
  notes: |
    CodexBackend подключён к codex-core, runtime поднимает полноценный tokio + auth.
    UI отображает потоковые ответы, статусы задач и вывод exec-команд. Локально
    требуется подтвердить запуск и тесты из verify_steps.
- id: codex-tui-file-explorer
  completed_at: 2025-09-19T20:30:00Z
  commits: []
  measures:
    latency_ms_p95: null
    memory_mb_peak: null
  test_summary:
    cargo_fmt: "не запускалось — cargo fmt завершается сигналом OUT OF MEMORY"
    cargo_check: "не запускалось — среда CLI завершает cargo сигналом"
    cargo_test: "не запускалось — среда CLI завершает cargo сигналом"
  verify_commands_ran: []
  docs_updated: []
  handoff:
    verify_steps:
      - just fmt
      - cargo check -p codex-tui
      - cargo test -p codex-tui
    rollback_steps:
      - git revert <commit_sha>
  notes: |
    Добавлен IDE-подобный файловый навигатор слева с клавишей F2 для фокуса,
    стрелочной навигацией, PgUp/PgDn/Home/End, Enter/Space для раскрытия и
    предпросмотром файлов в оверлее. Дополнены юнит-тесты и снапшот рендера.
    Локально необходимо выполнить команды из verify_steps: в среде Codex CLI
    `cargo fmt`, `cargo check -p codex-tui` и `cargo test -p codex-tui` завершаются
    сигналом (подозрение на ограничение памяти/времени).
- id: codex-docsearch
  completed_at: 2025-09-19T03:10:00Z
  commits: []
  measures:
    latency_ms_p95: null
    memory_mb_peak: null
  test_summary:
    python_docsearch_help: "python -m scripts.docsearch.query --help"
  verify_commands_ran:
    - python -m scripts.docsearch.query --help
  docs_updated:
    - docs/getting-started.md
    - docs/codex-gui.md
  handoff:
    verify_steps:
      - python -m pip install -r requirements-docsearch.txt
      - codex doc index --docs-root docs --recursive
      - codex doc search "как аутентифицироваться"
    rollback_steps:
      - git revert <commit_sha>
  notes: |
    Добавлены Python-скрипты индексации/поиска и CLI-обёртка `codex doc`. Требуется локальная установка
    зависимостей из requirements-docsearch.txt перед использованием.
- id: gui-backend-sessions-fix
  completed_at: 2025-09-20T11:11:10Z
  commits: []
  measures:
    latency_ms_p95: null
    memory_mb_peak: null
  test_summary:
    cargo_test_p_codex_gui: pass
    cargo_nextest_run_no_fail_fast: 'fail — множественные снапшот-разницы в codex-tui и pipeline::tests (предсуществующие изменения)'
  verify_commands_ran:
    - cargo test -p codex-gui
    - cargo nextest run --no-fail-fast
  docs_updated: []
  handoff:
    verify_steps:
      - cargo test -p codex-gui
      - cargo nextest run --no-fail-fast
    rollback_steps:
      - git checkout -- codex-rs/gui/src/backend/mod.rs
  notes: |
    Исправлен вызов list_sessions в тесте GUI backend: теперь unwrap через expect
    предотвращает обращение к методам Result. Тесты GUI проходят.
    Полный nextest падает на существующих снапшотах TUI/Core; требуется
    локальное review и обновление/фиксы вне текущей задачи.
- id: repo-health-audit
  completed_at: 2025-09-20T12:03:32Z
  commits: []
  measures: {}
  test_summary:
    cargo_test_pipeline_sign_verify_install_and_rollback_flow: pass
    cargo_test_p_codex_core_suite_compact: pass
    cargo_test_p_codex_mcp_server_patch_approval_triggers_elicitation: pass
    cargo_insta_test_p_codex_tui: pass
    cargo_nextest_run_no_fail_fast: pass
  verify_commands_ran:
    - cargo test pipeline::tests::sign_verify_install_and_rollback_flow
    - cargo test -p codex-core suite::compact
    - cargo test -p codex-mcp-server codex_tool::test_patch_approval_triggers_elicitation
    - cargo insta test -p codex-tui
    - cargo nextest run --no-fail-fast
  docs_updated: []
  handoff:
    verify_steps:
      - cargo nextest run --no-fail-fast
    rollback_steps:
      - git checkout -- codex-rs/core/src/security/mod.rs codex-rs/core/tests/suite/compact.rs codex-rs/mcp-server/tests/suite/codex_tool.rs codex-rs/tui/src/bottom_pane/mod.rs codex-rs/tui/src/status_bar.rs
  notes: |
    Устранены системные регрессии: исправлен повторное открытие аудит-леджера при блокировке,
    обновлены тесты автокомпакта под новый шаблон, актуализированы запросы на одобрение патчей
    и переработаны TUI снапшоты/расстановки, после чего весь `cargo nextest run --no-fail-fast`
    проходит без сбоев.
