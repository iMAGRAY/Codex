# Codex Desktop GUI Shell MVP

## Отсылка к требованиям
- REQ-UX-01 — «интерактивный опыт без когнитивной перегрузки» из `MaxThink-Stellar.md` реализуем через панель "Insight Canvas" с белой минималистичной темой.
- REQ-ACC-01 — поддерживаем клавиатурную навигацию и доступность: каждый интерактивный элемент дублируется клавишами, цвета соответствуют WCAG AA.
- REQ-DX-01 — ускоряем разработку: GUI переиспользует `codex_core::session` и шину событий TUI, предоставляя единый API.

## Цели M0→M1
1. Вывести Codex из режима CLI/TUI в полноэкранное десктопное приложение (бинарь `codex-gui`).
2. Сохранить функциональный паритет с текущим TUI для сценария «чат + выполнение команд».
3. Обеспечить расширяемость: каждая панель UI = независимый модуль, которого можно тестировать автономно.

## Ограничения
- Не ломаем публичные контракты CLI/TUI.
- Остаёмся в Rust-экосистеме, без Node/Chromium (приоритет производительности и размера).
- Архитектура должна укладываться в Definition of Done для Workstream 0/1: unit + snapshot тесты, документированный пайплайн.

## Архитектурный эскиз
```
+--------------------+        +------------------------+
| eframe::NativeOptions |     | AppServiceHandle       |
|  (тема, окна)      |        | - spawn_session()      |
+---------+----------+        | - send_prompt(...)     |
          |                   | - stream_events(...)   |
          v                   +-----------+------------+
+--------------------+                    |
| DesktopShell (UI) |<--------------------+
| - панель истории  |
| - редактор ввода  |    +----------------v----------------+
| - правый Inspector |    | codex_core::session::Manager   |
| - командная шторка |    | (существующая логика CLI/TUI)  |
+--------------------+    +--------------------------------+
```

### Интерфейс `AppServiceHandle`
```rust
pub trait AppServiceHandle: Send + Sync {
    fn spawn_session(&self) -> eyre::Result<SessionId>;
    fn send_prompt(&self, session: SessionId, payload: PromptPayload) -> eyre::Result<()>;
    fn subscribe(&self, session: SessionId) -> eyre::Result<SessionStream>;
    fn list_history(&self, session: SessionId) -> eyre::Result<Vec<HistoryItem>>;
}
```
Реализация по умолчанию — адаптер поверх `codex_core::session::Manager`, переиспользующий существующие тесты.

### UI-слой
- **Панель истории** — вертикальная лента карточек, каждая карточка = markdown View, стилизованная под белый Notion-блок.
- **Composer** — многострочный редактор с подсветкой клавиатурных шорткатов (⌘⏎ / Ctrl⏎).
- **Command Drawer** — выезжающее справа полотно с подсказками slash-команд.
- **Insight Sidebar** — структурированная информация о текущем файле/контексте, повторяющая TUI Insight Canvas.

## Потоки данных
1. Пользователь вводит промпт → `DesktopShell` вызывает `AppServiceHandle::send_prompt`.
2. Backend возвращает поток событий → UI подписан через канал, сообщения буферизуются в `HistoryModel`.
3. Сайдбар получает "observability" события (mcp, pipeline) и обновляет индикаторы.

## Тестирование
- `cargo test -p codex-gui` с юнитами на модель состояния и моками AppServiceHandle.
- Снапшоты eframe через `eframe::epaint::ahash` стабильный дамп (TODO: codex-gui-snapshots).
- `cargo run -p codex-gui -- --smoke` — headless прогон, убеждаемся, что окно поднимается < 120 мс.

## Метрики
- APDEX (латентность запуска окна) ≤ 180 мс.
- Peak RAM ≤ 512 МБ (eframe + codex_core).
- `Review Effort` ↓ ≥ 30% за счёт GUI (метрика из `stellar-vision`).

## Открытые вопросы
1. Нужно ли выдавать offline-bundle ассетов? (пока откладываем).
2. Нужно ли тащить wasm для web-версии? (не в MVP).
3. Какие платформы считаем must-have? (приоритет macOS, Windows, Linux X11/Wayland).

## Реализация MVP — 2025-09-19
- Добавлен workspace-крейт `codex-gui` с бинарём `codex-gui` (см. `codex-rs/gui/Cargo.toml`).
- Изначально бекенд (`codex-rs/gui/src/backend/mod.rs`) работал на моковых данных, обеспечив API для интеграции с `codex_core::ConversationManager`.
- UI (`codex-rs/gui/src/ui/mod.rs`) построен на `eframe/egui`: панель сессий, лента истории, белый composer с подсказками, Notion-подобное оформление.
- `lib.rs` предоставляет `bootstrap()` для запуска eframe с белой темой, `main.rs` имеет режим `--dry-run-ui` для headless-проверок.
- Юнит-тест `backend::tests::spawn_and_push_messages` проверяет моковый сценарий; smoke-тест размещён в `codex-rs/gui/tests/smoke.rs`.
- Ограничение среды: команды `cargo check`/`run`/`test` завершались сигналом в песочнице CLI, ручная проверка требуется локально (см. раздел Validate).

## GA Integration — 2025-09-19
- Реализован `CodexBackend` (`codex-rs/gui/src/backend/codex.rs`), подключающийся к `codex_core::ConversationManager` и проксирующий события Codex (сообщения, reasoning, команды).
- Добавлен модуль `runtime` (`codex-rs/gui/src/runtime/mod.rs`), который поднимает `tokio`-runtime, загружает конфигурацию `Config::load_with_cli_overrides`, инициализирует `AuthManager` и создаёт production `AppServiceHandle`.
- UI обрабатывает потоковые события: дельты ассистента, статусы задач, команды (`ExecCommand*`), отображая stdout/stderr в истории и статусы в отдельном баннере.
- Composer по умолчанию создаёт свежую сессию и держит live-подключение к Codex, поддерживает Ctrl/⌘+Enter.
- Семантический поиск документации доступен через `codex doc ...`, использует EmbeddingGemma (см. `scripts/docsearch`).
- Для тестов сохранён моковый сервис `AppServiceHandle::mock()`; smoke-тест переключён на него для детерминированности.

## Этап GA — задачи
- Заменить моковый `AppServiceHandle` на адаптер `codex_core::ConversationManager`, обеспечив те же методы.
- Инициализировать AuthManager/Config аналогично TUI (`codex-rs/tui/src/lib.rs`) и подключить telemetry.
- Поддержать потоковую доставку событий (статусы выполнения, streaming ответов, tool usage) с подсветкой в UI.
- Добавить панель команд/файловых операций (вписать slash-команды, просмотр git-диффов, запуск exec).
- Настроить сохранение истории/rollout в `$CODEX_HOME/conversations` с возможностью быстрого доступа.
- Обновить тесты: моковый backend оставить для unit, плюс контрактные тесты с `ConversationManager` под fake auth.
