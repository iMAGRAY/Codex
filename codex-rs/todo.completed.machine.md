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
