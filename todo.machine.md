# todo.machine.md — Enhancex ⇄ upstream/main Conflict Resolution Playbook (v4)

> **F (Success Function)** = 0.35·Correctness + 0.20·Simplicity + 0.20·Performance + 0.15·Cost + 0.10·Risk. Target ≥4.6/5 via upstream-quality sync of `Enhancex` onto `upstream/main` with zero regressions, coverage ≥85%, and reproducible audit trail.

---

## 0. Hard Gates & Non-Negotiables
- Maintain flagship+++ standards: 0 legacy/dead code, 0 hacks, 0 performance regressions; honour DDD, Ports & Adapters, Modular Monolith, ACL boundaries.
- Respect architectural layers: domain → app → adapters → infra → shared; forbid cycles, god objects, deep relative imports.
- Ensure DB side-effects go through CLAIM-first semantics with CLAIM|OUTBOX exactly-once strategy, idempotency key (`idempotency_key WHERE NOT NULL`), `RETURNING` rowcount==1.
- Coverage guard: statements & lines ≥85%; `make agent` and `cargo test --workspace --all-features` must pass post-resolution.
- No destructive FS ops without backup; honour `.flagship/ops.log` logging.
- CLI/Docs must reflect public contract changes; publish JSON Schemas for events, update `/metrics` spec with TLS/auth requirements.

---

## 1. Assumptions (TTL=1h)
- **A1**: Upstream remote `upstream` points to https://github.com/openai/codex and is fetchable; if not, pause and repair remote.
- **A2**: Current rebase base is `upstream/main` @ commit recorded in `reports/merge-state.log`; confirm via `git rev-parse HEAD^` after conflicts resolve.
- **A3**: Toolchain versions: rustc ≥1.82, cargo ≥1.82, just ≥1.25, pnpm ≥9. Verify in §2 before proceeding; failing any drops F by ≥1 point until remedied.
- **A4**: No pending unpublished schema migrations outside repo; otherwise document and gate merge.

Assumption expiry triggers re-validation before Phase 2.

---

## 2. Strategy Validation Snapshot
Seven execution families were stress-tested across algorithmic principle, dependency footprint, and scaling strategy. MCDA (weights from F) yields:

| ID | Family (Principle · Deps · Scaling) | F-score | Notes |
|----|-------------------------------------|---------|-------|
| S1 | Baseline 3-way merge · Full deps · Serial | 4.10 | Simple but underfits metadata migrations.
| S2 | Minimalist patch queues · Core-only deps · Serial | 3.85 | Low risk but stalls on CLI auth flows.
| S3 | Scalable staged rebase · Existing deps · Batched tests | 4.45 | Good throughput but higher coordination overhead.
| S4 | Frugal cherry-pick segments · Core deps · Serial | 3.70 | Reintroduces divergence risk.
| S5 | Low-deps rewrite stubs · Mock transports · Parallel | 2.95 | Violates no-legacy rule.
| S6 | Radical schema fork · New feature flags · Parallel | 2.50 | Breaks compatibility + cost spike.
| **S7** | Verified rebase with domain playbooks · Existing deps · Deterministic serial | **4.78** | Meets all gates, preserves auditability.

**Selected**: S7. Governing adjustments: enforce deterministic conflict pipeline, embed migrations verification early, serialize domain merges to keep claim-first invariants intact.

---

## 3. Execution Blueprint Overview
- **Phase 0 — Preflight**: Snapshot state, validate toolchain, confirm rebase context, prime logging.
- **Phase 1 — Diff Intelligence**: Materialize upstream vs Enhancex artifacts for each conflicting file, annotate differences, register risks.
- **Phase 2 — Conflict Integration**: Resolve per-domain conflicts (Core config types, Core config orchestration, CLI MCP command, Docs) with explicit sub-steps.
- **Phase 3 — Schema & Migration Assurance**: Run migrations, ensure idempotency and CLAIM semantics, reconcile tests.
- **Phase 4 — End-to-End Verification**: Format, lint, test per crate + workspace, regenerate docs, validate metrics + security flags.
- **Phase 5 — Rebase Continuation & Finalization**: `git rebase --continue`, audit tree, produce summary artifacts, prep PR.

Each task below lists **Inputs → Action → Outputs → Gate**; mark completion in `reports/progress.log`. IDs follow `<Phase>-<Domain>-<Seq>`.

---

## Phase 0 — Preflight Integrity
- **P0-01** Input: current repo. Action: `git status -sb` → ensure only listed conflicts. Output: snapshot to `reports/preflight/status.txt`. Gate: no unexpected files.
- **P0-02** Toolchain capture: run `rustc --version`, `cargo --version`, `just --version`, `pnpm --version`, `python3 --version`; log under `reports/preflight/toolchain.md`.
- **P0-03** Remote sanity: `git remote -v`, confirm upstream/origin; record commit SHAs (`git rev-parse upstream/main`, `git rev-parse HEAD`) in `reports/preflight/refs.md`.
- **P0-04** Logging: append session header to `.flagship/ops.log` with ISO timestamp + branch info.

---

## Phase 1 — Diff Intelligence & Risk Register
- **P1-01** Materialize 3-way snapshots: for each conflicted path produce upstream/enhancex/current copies under `tmp/{upstream,enhancex,current}/<path>.{up,enh,cur}`.
- **P1-02** Annotate schema deltas in `notes/risk-register.md`: columns {Subsystem, Change, Impact, Mitigation, Owner, Status}.
- **P1-03** Register migration-specific risks (lossy conversion, auth defaults) with severity assessment ≥ medium flagged for extra tests in Phase 3.
- **P1-04** Confirm assumption freshness; if any expired, refresh before proceeding.

---

## Phase 2 — Conflict Integration (Deterministic Serial)

### Block C — `codex-rs/core/src/config_types.rs`
- **P2-CT-01** Inputs: snapshots. Action: build structured diff focusing on struct definitions (`RawMcpServerConfig`, `McpServerConfig`, metadata structs). Output: table in `notes/core-config-types.md`. Gate: complete coverage of fields and serde tags.
- **P2-CT-02** Merge field sets: ensure final structs include upstream transport schema + Enhancex metadata (`display_name`, `category`, `tags`, `tool_timeout_sec`, auth, healthcheck, templates). Output: resolved file with serde defaults backstopping older configs. Gate: `cargo fmt -- codex-rs/core/src/config_types.rs` clean.
- **P2-CT-03** Implement helper structs (`McpAuthConfig`, `McpAuthProvider`, `McpHealthcheckConfig`, `McpTemplate`, `McpTemplateDefaults`) ensuring `#[serde(default)]`, `#[serde(skip_serializing_if = "Option::is_none")]`. Gate: rustc type-check via `cargo check -p codex-core --lib`.
- **P2-CT-04** Update conversions: in `impl TryFrom<RawMcpServerConfig> for McpServerConfig` map milliseconds to seconds (ceil), fill metadata defaults, propagate auth/health/template data. Gate: add unit tests covering conversions (legacy 0-ms, with metadata, invalid cases) in Phase 3.
- **P2-CT-05** Verify default impls: ensure `McpServerConfig::default()` reflects minimal safe config and does not emit new metadata unexpectedly. Document rationale in `notes/core-config-types.md`.

### Block CR — `codex-rs/core/src/config.rs`
- **P2-CR-01** Map dependencies: note usages of `McpServerConfig` fields across config loading/writing. Output: dependency list with line refs.
- **P2-CR-02** Reconcile struct fields for `Config`: reintroduce `mcp_templates`, `mcp_schema_version`, `experimental_mcp_overhaul`, transport-aware data. Gate: compile check.
- **P2-CR-03** Update serialization/deserialization pipelines: replace direct `.command/.args/.env` writes with transport switch (process/socket/http). Ensure metadata persists. Gate: unit tests for read/write cycle (Phase 3).
- **P2-CR-04** Reinstate migration hooks: wire `config::migrations::mcp::run` into load path, respecting CLAIM-first semantics and outbox pattern for side-effects. Gate: tests verifying migration idempotency.
- **P2-CR-05** Clean imports and remove stale helpers from Enhancex or upstream as needed; ensure layering is respected (no infra deps inside domain).

### Block CLI — `codex-rs/cli/src/mcp_cmd.rs`
- **P2-CLI-01** Align Clap command definitions: merge upstream OAuth login/logout with Enhancex migrate/wizard. Gate: `cargo check -p codex-cli --bin codex-cli`.
- **P2-CLI-02** Update JSON output schemas for `list`/`get`: include metadata fields (display_name, category, tags, auth, healthcheck, templates). Provide docstrings referencing docs contract.
- **P2-CLI-03** Reconcile execution paths for `serve` subcommand to ensure new transport options align with config types; respect EXACTLY-ONCE gating via claim/outbox for spawn operations.
- **P2-CLI-04** Harmonize error handling and logging with upstream style (colorized output, exit codes). Add integration tests or update snapshots Phase 3.

### Block TUI — `codex-rs/tui/src/*` (Status: QA HOLD)
- **P2-TUI-01** Resolve conflicts for `app.rs`, `app_event.rs`, `bottom_pane/mod.rs`, `chatwidget.rs`, `lib.rs`, integrating manager/wizard events. _Status: ✅ merged, awaiting extended tests_.
- **P2-TUI-02** Wire transport-aware types in `tui/src/mcp/{types.rs,manager_view.rs,wizard_view.rs}`; ensure Draft ↔ Config parity. _Status: ✅ complete._
- **P2-TUI-03** Generate/update snapshots under `tui/src/mcp/snapshots/` after UI changes. _Status: ✅ accepted via `cargo insta accept -p codex-tui`._
- **P2-TUI-04** Open follow-up for property/race tests (moved to Phase 3 tasks P3-03). _Status: 🔴 pending — see WS4 checklist._

### Block DOC — `docs/config.md`
- **P2-DOC-01** Merge tables describing MCP server configuration, ensuring new columns for metadata/auth/health/templates. Keep alphabetical order and anchors.
- **P2-DOC-02** Insert migration instructions referencing CLI `mcp migrate`, including backup reminders and CLAIM-first explanation. Gate: markdown lint (`just lint-docs`).

---

## Phase 3 — Schema, Tests, and Migration Assurance
- **P3-01** Add/Update unit tests:
  - `codex-rs/core/src/config_types.rs`: conversion tests for legacy + metadata cases using pretty_assertions.
  - `codex-rs/core/src/config.rs`: round-trip tests for `write_global_mcp_servers` covering transports and metadata.
  - `codex-rs/cli/src/mcp_cmd.rs`: CLI command tests verifying JSON schema.
- **P3-02** Migration validation: run `cargo test -p codex-core migration::tests::mcp_upgrade` (create if absent) ensuring idempotency + CLAIM semantics (UPDATE ... RETURNING).
- **P3-03** Race/Crash coverage: execute property/race tests per policy (threads ≥ max(32,2*CPU), repeats ≥3, deterministic seed). Document results in `reports/tests/race.log`.
- **P3-04** Snapshot management: if TUI touched indirectly, run `cargo test -p codex-tui`; review `.snap.new`, accept intentional via `cargo insta accept -p codex-tui` with notes.

---

## Phase 4 — End-to-End Verification & Quality Gates
- **P4-01** Formatting: `just fmt` in `codex-rs`.
- **P4-02** Targeted lint: `just fix -p codex-core`, `just fix -p codex-cli` (after user confirmation per policy) or document deferment.
- **P4-03** Testing cascade: `cargo test -p codex-core`, `cargo test -p codex-cli`, `cargo test -p codex-tui`; then (with approval) `cargo test --workspace --all-features`.
- **P4-04** Run `make agent` to satisfy flagship gates (fmt, lint, dup, complexity, security).
- **P4-05** Validate metrics endpoints docs `/metrics` for TLS/auth instructions; ensure config exposes `ALLOW_INSECURE_METRICS_DEV` guard.
- **P4-06** Security tooling: `cargo audit`, `cargo deny`, `codespell` if required by CI; log outputs.

---

## Phase 5 — Rebase Continuation & Finalization
- **P5-01** `git status` → ensure all conflicts resolved; stage resolved files with provenance notes in commit message templates.
- **P5-02** `git rebase --continue`; if new conflicts arise, loop back to Phase 1 for affected files.
- **P5-03** After rebase completes, run `git status -sb`, `git log --oneline -5` to confirm lineage.
- **P5-04** Documentation artifacts: `notes/merge-summary.md` (decisions, tests, metrics), `reports/post-merge/<timestamp>/commands.log` (commands + durations).
- **P5-05** Prepare PR summary referencing compliance with hard gates, risk register resolution, and outstanding TODOs (if any).

---

## Observability & Governance Addenda
- Update `.flagship/ops.log` after each major command with status PASS/FAIL.
- Maintain `reports/progress.log` with checkbox status for every task ID.
- Ensure no assumption exceeds TTL; refresh records or block merge.
- If new risks emerge, append to `notes/risk-register.md` with mitigation owner and due date.

---

## Post-Merge Monitoring Backlog
- Automate config diff against upstream nightly to preempt future conflicts.
- Instrument MCP migration usage metrics (`gateway_calls/logical_charges` ratio target 1.00±0.01 over 15m, `outbox_backlog` alert ≤1000 over 10m, `lock_wait_p99_ms` ≤50).
- Plan follow-up for telemetry opt-in UX and CLI help auto-generation alignment.
