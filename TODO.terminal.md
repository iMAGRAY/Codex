# Agent Workspace Overhaul — Roadmap

### North Star
Создать "рабочий стол" агента: все команды уходят в фон, их состояние и логи доступны через удобную панель/CLI, без лишнего шума. Агент быстро запускает/останавливает процессы, просматривает логи, экспериментирует с кодом.

---

## Iteration 1 — Background Jobs MVP
**Цель:** командный интерфейс `jobs`/`logs` + фоновые процессы по умолчанию.

### Tasks
- [ ] Exec backend: запуск команд без `wait`, запись метаданных (id, cmd, cwd, pid, start_time, status, paths).
- [ ] Директория `~/.codex/jobs/<id>/` (stdout.log, stderr.log, job.json).
- [ ] CLI `codex jobs list|stop|wait` (`codex-rs/cli/src/jobs_cmd.rs`).
- [ ] CLI `codex logs <id> [--follow|--tail N]` (`codex-rs/cli/src/logs_cmd.rs`).
- [ ] Интеграция с exec (`codex-rs/core/src/exec.rs`): после запуска возвращать агенту сообщение вида `job #ID`.
- [ ] Обновить документацию (`docs/getting-started.md`) и todo tracker.

### Acceptance
- `codex jobs list` показывает актуальные процессы.
- `codex logs <id> --follow` выводит STDOUT/STDERR.
- `codex jobs stop <id>` корректно останавливает процесс.
- Агент в чате видит короткую подсказку вместо «вечного ожидания».


## Iteration 2 — TUI/GUI Job Panel
**Цель:** удобный визуальный просмотр job’ов.

### Tasks
- [ ] Панель `Background Jobs` в TUI (горячая клавиша `Ctrl+J`).
- [ ] Навигация: `↑/↓` — выбрать, `Enter` — открыть лог, `S` — stop, `R` — restart.
- [ ] Вызов панели из GUI (например, отдельная вкладка/overlay).
- [ ] Небольшой индикатор статуса (running, success, failed, waiting).

### Acceptance
- Агент может без командных строк управлять процессами прямо в TUI.
- Отсутствует «шум»: панель отображается только при вызове.


## Iteration 3 — Scratchpad & Eval
**Цель:** быстрые эксперименты с кодом.

### Tasks
- [ ] Команда `codex eval <lang> <code>` → job’ы типа `eval` (stdout/stderr).
- [ ] Scratchpad панель в TUI: список последних eval’ов, возможность повторного запуска.
- [ ] Сохранение контекста (опционально: совместное окружение для eval).

### Acceptance
- Агент быстро тестирует фрагменты кода, видит результат в UI/CLI без копипаста.


## Iteration 4 — Polishing
**Цель:** сделать инструмент когнитивно лёгким и надёжным.

### Tasks
- [ ] Timeout по умолчанию (конфиг `exec.max_command_secs`).
- [ ] Детектор тишины (нет вывода > N минут → уведомить/остановить).
- [ ] Настройки `codex.toml`: лимиты, директории логов, автозавершение.
- [ ] Документация и UX-примеры.

### Acceptance
- Долгие процессы не висят вечно.
- Агент понимает, как настроить тайм‑ауты и где искать логи.

---

### Инструменты/директории
- code: `codex-rs/core/src/exec.rs`, `codex-rs/cli/src/*`, TUI — `codex-rs/tui/src/*`.
- data: `~/.codex/jobs/<id>/` (метаданные/логи).
- docs: `docs/getting-started.md`, `docs/codex-gui.md`.

---

### Definition of Done
- Все итерации завершены, документация обновлена.
- Агент может управлять job’ами и логами без зависаний.
- Scratchpad/фоновый режим не создаёт лишних уведомлений.
