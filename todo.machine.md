- id: repo-health-audit
  title: Проверка здоровья репозитория Codex
  type: investigation
  status: done
  priority: normal
  size_points: 3
  scope_paths:
    - .
  spec:
    Given: Репозиторий codex в /mnt/c/Users/1/Documents/GitHub/codex
    When: Выполнены аудит состояния и ключевые проверки работоспособности
    Then: Получен отчёт о текущем состоянии, включая выявленные проблемы и рекомендации
  budgets:
    time_hours: 4.0
  risks:
    - description: Тестовые команды могут длительно выполняться или требовать специфических окружений
      impact: medium
      mitigation: Запускать только доступные локально проверки и фиксировать пропуски
  dependencies: []
  tests_required:
    - cargo nextest run --no-fail-fast
  verify_commands:
    - git status --short
    - cargo nextest run --no-fail-fast
  rollback:
    commands: []
  docs_updates: []
  artifacts: []
  audit:
    created_at: 2025-09-20T00:00:00Z
    created_by: codex-agent
    updated_at: 2025-09-20T12:03:15Z
    updated_by: codex-agent

- id: gui-backend-sessions-fix
  title: Исправление list_sessions в GUI backend
  type: bugfix
  status: done
  priority: P0
  size_points: 1
  scope_paths:
    - path: codex-rs/gui/src/backend/mod.rs
  spec:
    given: Тест codex-rs/gui/src/backend/mod.rs::spawn_and_push_messages падает из-за Result без распаковки
    when: Исправлено обращение к list_sessions с обработкой ошибок
    then: cargo test -p codex-gui выполняется успешно без регрессий
  budgets:
    latency_ms_p95: 0
  risks:
    - отсутствие дополнительных багов не подтверждено
  dependencies: []
  tests_required:
    - cargo test -p codex-gui
  verify_commands:
    - cargo test -p codex-gui
  rollback:
    commands:
      - git checkout -- codex-rs/gui/src/backend/mod.rs
  docs_updates: []
  artifacts: []
  audit:
    created_at: 2025-09-20T11:01:18Z
    created_by: codex-agent
    updated_at: 2025-09-20T11:10:52Z
    updated_by: codex-agent
