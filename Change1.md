# Change1: Zero-Touch MCP Intake Engine

## 1. Executive Summary
- Цель: превратить добавление MCP‑серверов в Codex в операцию «указал источник → получил рабочий конфиг» без ручного ввода и последующих правок.
- Решение: внедряем модуль Zero-Touch Intake Engine (ZTIE), который нормализует источник (директория, архив, manifest), извлекает сигналы из разных экосистем, вычисляет прозрачные подсказки, мгновенно заполняет wizard CLI/TUI и гарантирует безопасность (read-only, sandboxed распаковка, объяснение каждого решения).
- Результат: пользователи получают готовую конфигурацию ≤1 секунды, ревьюерам остаётся читать детерминированный diff, разработчики поддерживают расширяемый движок, UX становится «drag/drop → готово» без потери гибкости.

## 2. Проблема и метрики успеха
- Ручной ввод `command`, `args`, `env`, `auth`, `health` остаётся самой дорогой частью онбординга MCP; ошибки вносят регрессии и требуют ревью.
- Существующий wizard делает валидацию, но целиком полагается на пользователя.
- Метрики успеха:
  1. ≥95% типовых Node/Python MCP на тестовом наборе настраиваются без ручной правки.
  2. TUI декларация с drag & drop укладывается в 800 мс p95 на macOS/Linux и 1.2 с на Windows.
  3. Отказы детектора имеют объяснение (reason codes) и fallback к ручному режиму ≤100 мс.
  4. Нулевая исполняемость: Intake Engine не запускает сторонний код и не модифицирует файлы источника.

## 3. Инварианты и явные non-goals
- Read-only анализ: никаких `npm install`, `python setup.py`, `make` и т. п.
- Никаких новых пользовательских форматов; переиспользуем текущий `McpServerConfig`, TemplateCatalog.
- Никакой сети/телеметрии: все эвристики локальные.
- Drag & drop поддерживаем в TUI; CLI — путь/архив/manifest.
- Производительность: одно сканирование ≤ 150 мс для каталогов до 200 файлов (кеш на медленных дисках допускается).
- Поддерживаем Linux/macOS/Windows без POSIX-специфики.

## 4. Матрица идей (100 вариантов + критика)

### A. Source Input Experience
1. **A1** Автораспознавание по простому `--path` без доп. флагов. *Критика:* нет поддержки архивов/manifest.
2. **A2** Разрешить URL (HTTP/S) как источник. *Критика:* нарушает zero-egress.
3. **A3** Drag & drop файлов в CLI (через TTY протокол). *Критика:* требуются нестабильные терминальные возможности.
4. **A4** Автоматический монитор `fsnotify` выбранной папки. *Критика:* фоновые воркеры, сложный lifecycle.
5. **A5** Диалог выбора каталога (кроссплатформенный файл-пикер). *Критика:* зависит от GUI, ломается в headless.
6. **A6** Добавление источников из истории предыдущих импортов. *Критика:* потенциальная путаница и устаревшие пути.
7. **A7** Qt/WebView wizard. *Критика:* огромный dependency footprint.
8. **A8** Поддержка tar.gz помимо zip. *Критика:* требует внешние бинарии или сложный парсер (Windows).
9. **A9** Поддержка password-protected архивов. *Критика:* UI/секреты, редко используется.
10. **A10** «Source bundles» (zip + metadata json). *Критика:* новый формат, когнитивная нагрузка.

### B. Metadata Extraction Strategies
11. **B1** Чисто по названию каталога. *Критика:* коллизии.
12. **B2** Токенизация README в поиске команд. *Критика:* false positive/negative.
13. **B3** AST анализ `package.json` + `src`. *Критика:* тяжёлый парсинг без выгоды.
14. **B4** Heuristic на `.mcp/config.yaml`. *Критика:* формат не стандартизирован.
15. **B5** Поиск `*.service.json` в repo. *Критика:* нет общего соглашения.
16. **B6** Использование git submodules metadata. *Критика:* git обязателен не всегда.
17. **B7** Интеграция с `codemeta.json`. *Критика:* редко встречается.
18. **B8** Чтение comment headers в исходниках. *Критика:* слишком вариативно.
19. **B9** Набор предзагруженных signature-хэшей (fingerprint). *Критика:* необходимо поддерживать вручную.
20. **B10** Lightweight, но версионированные «knowledge packs» с pattern’ами. *Критика:* требует инфраструктуры доставки обновлений.

### C. Command / Runtime Identification
21. **C1** Всегда требовать ручной ввод команды. *Критика:* нулевая автоматизация.
22. **C2** Хардкод `node server.js`. *Критика:* узкая экосистема.
23. **C3** Парс `package.json.bin`. *Критика:* покрывает только CLI-пакеты.
24. **C4** `npm run mcp` если существует. *Критика:* требует npm scripts.
25. **C5** Поиск shebang `#!/usr/bin/env node/python` в `bin/`. *Критика:* может найти несвязанный скрипт.
26. **C6** TOML entrypoints `[project.scripts]`. *Критика:* разные схемы (poetry, hatch).
27. **C7** Detected docker compose service. *Критика:* container-specific.
28. **C8** Machine learning классификация по файлам. *Критика:* требует модели/данных.
29. **C9** Приоритетный порядок: manifest → scripts → bin fallback. *Критика:* нужно управлять конфликтами.
30. **C10** Комбинированное голосование нескольких провайдеров с confidence score. *Критика:* сложность реализации, но даёт объяснимость.

### D. Environment & Secrets Handling
31. **D1** Игнорировать env полностью. *Критика:* пользователи всё равно вручную дописывают.
32. **D2** Перекладывать `.env` значения как есть. *Критика:* риск утечки секретов.
33. **D3** Создавать `.env.template`. *Критика:* побочный файл, меняет repo.
34. **D4** Предлагать `KEY=` без значений, с описанием из README. *Критика:* извлечение описаний ненадёжно.
35. **D5** Детектировать `process.env` / `os.environ` употребления. *Критика:* нужна статическая проверка кода.
36. **D6** Использовать `env.example` как источник ключей. *Критика:* если файл отсутствует — бесполезно.
37. **D7** Поддержка Secret Manager (AWS/GCP). *Критика:* требует сетевых привилегий.
38. **D8** Automatic fallback на global profile env. *Критика:* риск пересечений.
39. **D9** Интерактивный опрос значений. *Критика:* нарушает zero-touch.
40. **D10** Регистратор env ключей с описанием и уровнем критичности (scorecard). *Критика:* нужно формировать словарь типовых ключей.

### E. Health & Monitoring Integration
41. **E1** Оставлять health пустым. *Критика:* без сигналов мониторинга.
42. **E2** Всегда ставить `health.kind = "stdio"`. *Критика:* не все сервера поддерживают.
43. **E3** Ищем `healthcheck.js`. *Критика:* naming нестандартный.
44. **E4** Пытаемся ping `http://localhost:service`. *Критика:* нельзя выполнять сетевые операции.
45. **E5** Разрешаем пользовательские preset’ы (опциональные). *Критика:* нужно UI.
46. **E6** Автоматически используем metadata из manifest. *Критика:* если нет manifest — fallback.
47. **E7** Генерируем `none`, но добавляем TODO. *Критика:* когнитивная нагрузка.
48. **E8** Обогащаем TemplateCatalog health defaults. *Критика:* требует ручного пополнения шаблонов.
49. **E9** Строим health pipeline из workflow files. *Критика:* сложно и ненадёжно.
50. **E10** Вычисляем «готовность» и предлагаем пользователю принять/отклонить преднастройку в wizard (с пояснением). *Критика:* надо уметь объяснять источник данных.

### F. UX / Interaction Patterns
51. **F1** Показывать длинный checklist. *Критика:* когнитивный шум.
52. **F2** Отображать diff config vs auto-suggestion. *Критика:* полезно, но требует понятного layout.
53. **F3** Анимированный walkthrough. *Критика:* удлиняет сценарий.
54. **F4** Только CLI без TUI. *Критика:* теряем визуальный опыт.
55. **F5** «Wizard Stories» с историей изменений. *Критика:* усложняет поддержку.
56. **F6** Smart defaults + пояснения (tooltip, footnote). *Критика:* требует доп. UI, но снижает когнитивную нагрузку.
57. **F7** Обязательный просмотр исходников. *Критика:* нарушает zero-touch.
58. **F8** Навигация через hotkeys (accept/override). *Критика:* нужно подсветить сочетания.
59. **F9** Авто-коммит конфигурации. *Критика:* нежелательно менять репо автоматически.
60. **F10** Карточка источника (иконки языка, команда, env count, health) в wizard summary. *Критика:* надо следить за консистентностью и цветами.

### G. Automation & Orchestration
61. **G1** Cron-задача, обновляющая MCP списки. *Критика:* избыточно.
62. **G2** Автообновления конфигов при изменении источника. *Критика:* нужно отслеживать версии.
63. **G3** Pipeline «source → PR в config repo». *Критика:* требует GitOps инфраструктуру.
64. **G4** Auto-labelling MCP entries. *Критика:* низкий приоритет.
65. **G5** Telemetry pipeline для heuristics. *Критика:* политика zero-egress.
66. **G6** Build cache эвристик (pre-computed indexes). *Критика:* нужно синхронизировать кеш.
67. **G7** Partial updates: wizard пересчитывает только прирост. *Критика:* сложнее код.
68. **G8** Retry-петля с backoff при IO ошибках. *Критика:* редко нужно.
69. **G9** Parallel scanning нескольких источников. *Критика:* UX усложняется.
70. **G10** Сценарий «bulk import» (несколько MCP). *Критика:* перегрузка интерфейса.

### H. Extensibility & Maintenance
71. **H1** Жёстко зашитый код без плагинов. *Критика:* трудно расширять.
72. **H2** Lua/JS плагины для детекторов. *Критика:* безопасность, sandboxing.
73. **H3** Rust trait `Detector` + registry. *Критика:* нужна координация версий, но расширяемость высокая.
74. **H4** YAML-конфигурация детекторов. *Критика:* двойной источник правды.
75. **H5** Auto-generated код из JSON схем. *Критика:* усложнение пайплайна.
76. **H6** FFI к Python плугинам. *Критика:* зависимости и performance.
77. **H7** Feature flags per detector. *Критика:* управление конфигурацией сложнее.
78. **H8** Detector marketplace. *Критика:* governance nightmare.
79. **H9** Version pinning per detector (semver). *Критика:* требует инфраструктуры релизов.
80. **H10** Knowledge pack bundles, подписанные и версионированные (индекс в repo). *Критика:* нужно CI для обновления.

### I. Safety & Security
81. **I1** Игнорировать path traversal. *Критика:* уязвимо.
82. **I2** Не ограничивать размер архивов. *Критика:* DoS.
83. **I3** Не чистить временные каталоги. *Критика:* переполнение диска.
84. **I4** Отсутствие audit trail. *Критика:* тяжело расследовать.
85. **I5** Без проверки прав доступа. *Критика:* может упасть на readonly FS.
86. **I6** Подписи архивов (PGP). *Критика:* высокая стоимость внедрения.
87. **I7** Mandatory antivirus scan. *Критика:* требует внешних сервисов.
88. **I8** Allow-list расширений (`.js`, `.py`). *Критика:* MCP может быть на других языках.
89. **I9** Sandbox распаковки (внутренний temp + canonicalize). *Критика:* нужно аккуратно реализовать, но даёт безопасность.
90. **I10** Жёсткое логирование происхождения каждой подсказки (explainability). *Критика:* надо хранить reason codes и строки.

### J. Deployment & Distribution
91. **J1** Ship как отдельный бинарь. *Критика:* дробление.
92. **J2** Распространять как npm пакет. *Критика:* Node-only, нет Rust integration.
93. **J3** На уровне Codex CLI feature flag. *Критика:* нужна миграция конфигов.
94. **J4** Поставлять knowledge packs через Git submodule. *Критика:* обновления тяжёлые.
95. **J5** CI step обновляющий packs при release. *Критика:* дополнительная логистика.
96. **J6** Auto-update через CDN. *Критика:* egress.
97. **J7** Встроить packs в binary (include_bytes!). *Критика:* увеличивает размер, требуется пересборка на обновления.
98. **J8** Поставлять packs в `~/.codex/knowledge-packs`. *Критика:* нужен контроль целостности.
99. **J9** Версионировать packs через Cargo feature. *Критика:* рост матрицы сборки.
100. **J10** Гибрид: core heuristics в коде, пакеты расширения — optional, подписанные TOML в repo. *Критика:* надо следить за совместимостью форматов.

### Dual Cynic Review
- **Циник А:** «Если Intake Engine ошибётся, доверие потеряно. Любая магия должна быть объяснена чётко: что прочли, почему выбрали, чем подкрепили.»
- **Циник B:** «Иначе UX снова превратится в ручной ввод. Значит, все подсказки обязаны иметь confidence score, ссылку на файл/строку и возможность мгновенного редактирования. Также нужен аварийный выход без артефактов.»
- **Оба:** Заканчивают выбором архитектуры, где каждый шаг детерминирован, логируем источник данных, пользователю показываем карточку с происхождением и даём одну кнопку “Принять всё” или “Редактировать пункт”.

## 5. Выбранная архитектура: Zero-Touch Intake Engine

### 5.1 Архитектурные слои
1. **Normalizer**
   - Принимает `SourceHandle` (directory / zip / manifest file).
   - Выполняет проверку размера, path traversal, распаковку zip в `~/.codex/mcp_sources/<hash>`.
   - Формирует `IntakeWorkspace` с read-only дескрипторами.
2. **Signal Collectors** (реализуют trait `Detector`)
   - `ManifestDetector` (ищет `mcp.json`, `codex-mcp.json`, `mcp.config.toml`).
   - `NodeDetector` (analyze `package.json`, scripts, bin, tsconfig).
   - `PythonDetector` (`pyproject.toml`, `setup.cfg`, entrypoints, venv markers).
   - `ExecutableDetector` (shebang, chmod+x, Windows `.cmd/.exe`).
   - `EnvDetector` (`.env`, `.env.example`, `config/*.env`).
   - `HealthDetector` (manifest defaults, `healthcheck.*`, `scripts/*health*`).
   - Каждый детектор возвращает `Signal` со score (0..100), reason и patch (`ConfigDelta`).
3. **Aggregator**
   - Сливает `ConfigDelta` в `DraftConfig` по приоритетам: manifest > template > detector score.
   - Конфликты решает голосованием + explanation (например, Node и Python предложили разные команды).
   - Вычисляет `ConfidenceReport` (по полям: command/env/health/tags/cwd).
4. **Presentation Layer**
   - CLI: расширяем `WizardOutcome` (fields: `source`, `draft`, `confidence`, `signals`).
   - TUI: карточка источника, таблица предложений, подсветка по confidence (зеленый ≥80, жёлтый 50..79, красный <50).
   - JSON output включает trace (`signals[]` с `detector`, `file`, `line`, `value`).
5. **Knowledge Packs**
   - Компактные TOML в `resources/mcp_knowledge/<ecosystem>.toml` (поставляем с бинарём, версионируем).
   - Обновляются через обычный release (нет сети).

### 5.2 User Flow (CLI и TUI)
1. Пользователь запускает `codex mcp wizard --source /path/to/server` или дропает каталог в TUI.
2. Normalizer валидирует вход, распаковывает при необходимости, формирует workspace.
3. Коллекторы сканируют workspace, возвращают сигналы (<150 мс).
4. Aggregator строит draft, подсчитывает confidence, логирует в trace.
5. Wizard (CLI/TUI) выводит один компактный блок с ключевыми полями (команда, аргументы, env, health) в виде табличной строки и небольшого бокового пояснения, без карточек и лишних цветов. Клавиши действий остаются привычными: `Enter` подтверждает, `e` открывает редактирование конкретного поля. Обоснование подсказок отображается в минималистичном футере ("command ↦ NodeDetector › package.json › bin.codex-mcp"), который можно раскрыть/скрыть клавишей `?`.
6. Пользователь жмёт `--apply` / `Enter`, конфиг сохраняется атомарно в Codex config.
7. Тrace пишется в `.agent_logs/<timestamp>.json` (для поддержки).

### 5.3 Zero Cognitive Load гарантии
- Вся MCP-настройка по-прежнему живёт в командном пространстве `codex mcp`: `codex mcp wizard` (интерактив), `codex mcp add` (ручной ввод), `codex mcp list/get/remove` — без новых подкоманд.
- Один вход `--source`; авто-подсказки подставляются как значения по умолчанию в тех же вопросах wizard-а, что уже существуют сегодня.
- Если confidence < 50 для ключевых полей (command/env), wizard автоматически фокусирует соответствующий шаг (CLI — повторно задаёт вопрос, TUI — открывает inline-редактор).
- При drag & drop в TUI открывается тот же минималистичный wizard с одной строкой summary; подтверждение — `Enter`, подробности — `?`.
- В CLI non-interactive `--apply` при confidence < 50 возвращает ошибку с объяснением, чтобы избежать скрытых фейлов.

### 5.4 Дополнительные механизмы
- **Caching:** hash пути + mtime. Повторные сканы используют кешированные сигналы (TTL 10 мин).
- **Auditing:** reason codes (`manifest:bin`, `python:entry_point`, `env:file`) + path. Логи доступны для диагностики.
- **Failures:** любые ошибки нормализатора возвращают точные коды (zip_too_large, path_traversal_detected, missing_source). Wizard fallback в ручной режим без артефактов.

## 6. План реализации (итеративно)

1. **Фаза Alpha (детекторы ядра)**
   - Реализовать модуль `codex_core::mcp::intake` (Normalizer + Detector trait + aggregator).
   - Детекторы: Manifest, Node, Python, Executable, Env, Health baseline.
   - Unit-тесты на каждую комбинацию (fake FS fixtures).
   - CLI интеграция (`--source`, расширенный JSON).
   - Включить behind feature flag `experimental.auto_mcp_intake`.

2. **Фаза Beta (UX + knowledge packs)**
   - TUI карточки, drag & drop событие `AppEvent::SourceDropped`.
   - Поддержка zip, кеширование workspace.
   - Knowledge packs для популярных серверов (Claude, OpenRouter, Jupyter, File Manager).
   - Snapshot тесты, документация (MCP-MANAGEMENT.md, docs/cli/mcp.md).

3. **Фаза GA (твёрдые гарантии)**
   - Confidence thresholds, auto-fallback логика.
   - Финальные security хардены (size limit, temp GC, audit logs).
   - Полный прогон `cargo test --all-features`, `just fmt`, `just fix -p codex-core`, `cargo test -p codex-tui`.
   - Обновление CHANGES.md, release notes.

## 7. Тестовая стратегия
- Unit: каждый детектор, нормализатор, агрегатор конфликтов.
- Integration: сценарии `wizard --source` для Node/Python/manifest, error cases (bad zip, missing command).
- Snapshot: TUI wizard с drag & drop, CLI JSON.
- Performance: бенчмарк (criterion) на средний каталог.
- Security: тест path traversal, zip-бомба (ограничение entries < 2000, размер < 200 MB).

## 8. Риски и смягчения
- **False positives:** mitigated by confidence scores и объяснения; auto-accept только ≥80.
- **Расширение экосистем:** добавляем новые detectors через knowledge packs (trait-based, unit-тесты).
- **Windows path vs symlink:** используем `camino::Utf8Path`, canonicalize с проверкой ошибок.
- **Объём архива:** лимит + прерывание с сообщением.

## 9. Success Criteria (Definition of Done)
- Все acceptance тесты зелёные; новый движок включён по умолчанию при `experimental.auto_mcp_intake=true`.
- Wizard CLI/TUI демонстрирует auto-filled конфиг на демо-наборе (видео/скринкаст для внутреннего релиза).
- Документация синхронизирована (MCP-MANAGEMENT.md, docs/cli/mcp.md, CHANGES.md).
- Паспортизированы trace reason codes (таблица в docs/intake/reasons.md).

## 10. Next Steps
- Утвердить архитектуру с владельцами CLI/TUI и security.
- Создать задачи по фазам Alpha/Beta/GA в tracker.
- Запустить реализацию Alpha (core module + базовые detectors).
- После Alpha — ревью и включение флага в dogfooding.
