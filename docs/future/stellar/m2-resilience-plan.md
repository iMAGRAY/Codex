# Stellar Resilience Plan (M2) - Cache, Conflicts, Prefetch

## Inputs
- `docs/rfcs/0003-stellar-resilience.md` - архитектурные рамки Local Resilience Cache, Conflict Resolver, Weighted Confidence, Predictive Prefetch (REQ-REL-01, REQ-DATA-01, REQ-PERF-01; #14, #15, #32, #35, #67).
- `docs/future/MaxThink-Stellar.md` - метрики и ответственность команд (Resilience Squad/SRE, METRIC-AVAIL, METRIC-LATENCY).
- `docs/future/stellar/adrs/adr-res-001.md` - выбор backend’а (`sled`) и требования по восстановлению.
- `docs/future/stellar/backlog.md` - EPIC-RESILIENCE, trace к #14/#15/#35/#67.

## Checklist
- [x] CACHE - Спроектировать Local Resilience Cache и Resilient Transport с TTL/eviction и offline-очередями (REQ-REL-01; #14, #35, #71).
- [x] CONFLICT - Описать Intent Conflict Resolver API и контракт TUI/CLI (REQ-DATA-01; #15).
- [x] CONFIDENCE - Согласовать Weighted Confidence scoring и telemetry trace (REQ-DATA-01, REQ-PERF-01; #16, #67).

## Outputs
- Cache/transport архитектура с классами, интерфейсами и политиками TTL/eviction + интеграцией с chaos профилями.
- API-спецификация Conflict Resolver (data model, events, UI hooks, CLI parity).
- Weighted Confidence scoring матрица, источники сигналов и telemetry hook-схема.
- План тестов: unit, integration, chaos, benchmarks.

---

## 1. Local Resilience Cache & Transport (REQ-REL-01; #14, #35, #71)

### 1.1 Компоненты
| Component | Responsibility | Key Interfaces | Notes |
| --- | --- | --- | --- |
| `resilience::cache::CacheStore` | TTL storage, snapshotting, encryption(ADR-RES-001) | `get`, `put`, `evict`, `snapshot`, `hydrate` | Backend default `sled`, pluggable trait. |
| `resilience::transport::RetryQueue` | Очередь команд/инсайтов в offline-режиме | `enqueue(cmd)`, `drain(policy)` | Policies: exponential backoff, max staleness. |
| `resilience::metrics::CacheStats` | Cache hit/miss, latency, durability metrics | OTLP events, inline panel for SRE | Feeds METRIC-AVAIL, METRIC-LATENCY. |

### 1.2 Политики
- **TTL**: конфигурируется (default 15мин); eviction по LRU + high-watermark (disk usage). Trace → #35.
- **Snapshots**: `snapshot()` создаёт deterministic dump → восстановление при старте (`hydrate`).
- **Chaos Hooks**: `ChaosProfile` (network drop, disk corruption) → автоматический failover на in-memory fallback, метрики писаются в OTEL (#91).
- **Transport Retry**: strategy = `base_delay=500ms`, `factor=2.0`, `jitter=true`, расходы ограничены `max_attempts=5`. Queue flush запускается по восстановлению соединения либо ручному триггеру CLI (`codex resilience flush`).

### 1.3 Интеграции
- TUI: StellarKernel подписывается на cache events (`CacheEvent::Hydrated`, `CacheEvent::ResyncRequired`) чтобы обновлять UI состояния.
- CLI: `codex stellar cache status` (добавить позже) будет использовать тот же API `CacheStats::snapshot`.

## 2. Conflict Resolver API (REQ-DATA-01; #15)

### 2.1 Data Model
```
pub struct ConflictEntry {
    pub id: ConflictId,
    pub key: String,
    pub sources: Vec<SourceValue>,
    pub resolution: ResolutionState,
    pub confidence: f32,
    pub reason_codes: Vec<String>,
    pub last_updated: DateTime<Utc>,
}
```
- `sources`: origin metadata (cache, remote, local override) + timestamp.
- `resolution`: `Pending`, `AutoResolved`, `UserAccepted`, `UserRejected`.

### 2.2 API Surface
| Method | Description | Consumer |
| --- | --- | --- |
| `list_pending(limit)` | Возвращает N конфликтов для UI | TUI overlay, CLI `codex stellar conflicts list` |
| `apply_resolution(id, decision)` | Применить выбор пользователя | TUI action footer, CLI `codex stellar conflicts resolve` |
| `subscribe_updates()` | Stream для UI | TUI overlay refresh |

### 2.3 UX Contract
- TUI: Insight Canvas показывает inline сообщения (`ConflictBadge`) + раскрытие overlay (`Ctrl+Shift+C`).
- CLI: JSON события транслируются через `StellarCliEvent` (`action=core.conflict.resolve`).
- Accessibility: ответы читаются screen reader’ом (ARIA-like hints).

## 3. Weighted Confidence Scoring (REQ-DATA-01, REQ-PERF-01; #16, #67)

### 3.1 Factors & Weights
| Factor | Signal | Default Weight | Notes |
| --- | --- | --- | --- |
| Freshness | `now - source.timestamp` | 0.35 | Decays exponentially; floor 0.1. |
| Source Trust | `source.trust_score` | 0.30 | Derived from policy metadata (SecOps). |
| Schema Validity | Validation results | 0.20 | Fails = negative penalty. |
| Telemetry Alignment | Latency/APDEX guardrails | 0.10 | Uses metrics from §1.2. |
| User Overrides | manual accept/reject | 0.05 | Reinforces accepted sources. |

Confidence = sum(weights * normalized factors). Lower bound 0.0, upper 1.0.

### 3.2 Telemetry & Trace
- Emit `confidence.scored` event with factor breakdown (OTLP span attributes).
- Cache metrics write to `CacheStats` → aggregator updates Weighted Confidence history.
- Baseline metrics captured via benchmark harness (`cargo bench -p codex-core --bench resilience_prefetch`).

### 3.3 UI Mapping
- Insight Canvas Confidence Bar: thresholds (>=0.75 green, 0.4-0.74 amber, <0.4 red).
- CLI: `codex stellar insight confidence` outputs JSON with factor array.

## 4. Test & Validation Matrix
| Scope | Tooling | Target |
| --- | --- | --- |
| Cache unit tests | `cargo test -p codex-core --lib resilience::*` | TTL, hydration, eviction correctness |
| Integration (CLI/TUI) | `cargo test -p codex-tui --test all`, `cargo run -p codex-cli -- stellar ...` | CLI parity, conflict flows |
| Chaos | `scripts/chaos/resilience.sh` (network drop, disk corruption) | Recovery <= 3s, zero data loss |
| Benchmarks | `cargo bench -p codex-core --bench resilience_prefetch` | Prefetch latency <= 80 мс median |
| Metrics | OTEL export via `resilience::metrics::export()` | METRIC-AVAIL >= 99.3%, METRIC-LATENCY p95 <= 200 мс |

## 5. Next Build Actions
1. Имплементировать `resilience::cache` и `resilience::transport` модули (sled backend, retry queue).
2. Подключить Conflict Resolver API и UI (overlay + CLI команды).
3. Реализовать Weighted Confidence калькулятор и интеграцию со Stellar Kernel.
4. Добавить хаос-скрипты и Criterion бенчмарки для cache/prefetch.
5. Обновить метрики и документацию (metrics-baseline.md, runbooks).

## 6. Implementation Snapshot — 2025-09-18
- ✅ **Local Resilience Cache & Queue** (`codex-rs/core/src/resilience/cache.rs`, `transport.rs`) реализованы с шифрованием, TTL и snapshot API. Trace → REQ-REL-01.
- ✅ **Chaos Loop Harness** — `scripts/chaos/resilience_loop.sh` гоняет `cargo test -p codex-core --test resilience_chaos` в цикле (≥10 мин) и пишет логи в `target/chaos/` (30/30 PASS за 603 с 2025-09-18).
- ✅ **Detector Registry & Signal Cache** (`codex-rs/core/src/mcp/intake/detectors.rs`, `cache.rs`) поддерживают hot reload и fingerprint (path + mtime). Trace → REQ-DATA-01.
- ✅ **Weighted Confidence & Conflict UX** (`codex-rs/core/src/stellar/state.rs`, `tui/src/stellar/view.rs`) подсвечивают risk alerts, p95 latency и конфликты в TUI/CLI. Trace → REQ-DATA-01, REQ-PERF-01.
- ✅ **CLI/TUI coverage** — `cargo test -p codex-core`, `cargo test -p codex-tui`, `cargo test -p codex-cli` (2025-09-18) зелёные; снапшоты обновлены (insta).
- ✅ **Criterion Benchmark Harness** — `cargo bench -p codex-core --bench resilience_prefetch` измеряет cache put/get и snapshot hydrate латентность (целевой p95 ≤ 80 мс). Итерация 2025-09-18: put/get ≈ 335 µs, snapshot hydrate ≈ 19 мс, prefetch record/top ≈ 152 нс.
- ℹ️ **Pending**: chaos сценарии и Criterion бенчмарки (см. §4.4) запланированы к включению в Validate фазу.
