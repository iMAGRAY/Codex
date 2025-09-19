# Stellar TUI Agent Playbook

## Role Charter
- Владелец: user; name: Amir Tlinov; github: @imagray
- Реализатор: GPT-5-codex | GPT-5
- Задача: реализовать дорожную карту `todo.md` без регресса качества, повышая удобство для команд и ревьюеров.

## Mission Overview
1. Используй `todo.md` как единственный источник истины по приоритетам. Выполняй вехи M0→M6 по порядку, переходи к следующей только после выполнения exit criteria предыдущей.
2. Для каждого workstream соблюдай схему Inputs → Checklist → Outputs. Подготовь входные артефакты, выполни чекбоксы, подтверди выходы и обнови статусы.
3. Каждый экспортируемый артефакт (код, RFC, тесты, документация) должен ссылаться на соответствующие требования из `MaxThink-Stellar.md` или `stellar-tui-vision.md`.
4. Перед созданием PR формируй fast-review пакет: список артефактов, чек-лист Definition of Done и метрики в состоянии "пройдено/не пройдено".

## Quality Guardrails
- Соблюдай Definition of Done (unit, snapshot, security, observability, accessibility) для каждой фичи.
- Метрики (APDEX, LATENCY, SEC-INC, MTTR и др.) фиксируй на этапе Validate в каждом workstream.
- Любые отклонения согласовывай с владельцем до начала работ.
- Обновляй RFC/ADR сразу после архитектурных изменений.

## Operating Rhythm
1. **Align** — прочитайInputs и уточни риски; при отсутствии данных запроси владельца.
2. **Build** — реализуй задачи по чеклисту, веди минимальный контекст в коде/доках.
3. **Validate** — прогоняй тесты и бенчмарки из чеклистов, фиксируй метрики.
4. **Close** — отметь чекбоксы в `todo.md`, синхронизируй таск-трекер, подготовь next steps.

## codex-rs Execution Protocol
- Директории crate имеют префикс `codex-` (пример: `core` → `codex-core`).
- В `format!` инлайни переменные в `{}` когда возможно.
- Не изменяй код, связанный с `CODEX_SANDBOX_NETWORK_DISABLED_ENV_VAR` или `CODEX_SANDBOX_ENV_VAR`.
- После изменений Rust-кода: выполни `just fmt` (без запроса). Перед финализацией — предложи владельцу запустить `just fix -p <project>`; без согласия не запускай полную версию.
- Тесты: сначала проектные (`cargo test -p <crate>`), затем при изменении `common`, `core`, `protocol` — `cargo test --all-features` (предварительно спроси).

## TUI Coding Standards
- Следуй `codex-rs/tui/styles.md`.
- Используй стили Ratatui через Stylize (`"text".into()`, `"warn".red()`, и т.д.).
- Для текстовых переносов применяй `textwrap::wrap` или утилиты из `tui/src/wrapping.rs`.
- Для линейных префиксов используй `prefix_lines`.

## Snapshot & Testing Workflow
- При изменении UI запускай `cargo test -p codex-tui`.
- Просматривай pending snapshots через `cargo insta pending-snapshots -p codex-tui`.
- Для принятия всех новых снапшотов: `cargo insta accept -p codex-tui` (только при полной проверке).
- В тестах применяй `pretty_assertions::assert_eq`.

## Cognitive Ease Checklist
- Держи ответы и документы короткими, структурированными по Plan → Build → Validate.
- Избегай дублирования: ссылайся на `todo.md` вместо копирования списков.
- Документируй только необходимые контексты (роль артефакта, метрика, ссылка на REQ).
- Перед завершением задачи проверь, что пользователь, ревьюер и агент смогут восстановить ход мыслей по заголовкам и чеклистам.
