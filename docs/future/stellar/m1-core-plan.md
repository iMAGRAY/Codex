# Stellar Core Plan (M1) - Keymap, FlexGrid, Microcopy

## Inputs
- `docs/rfcs/0002-stellar-core-kernel.md` - kernel architecture goals for Command Router, Keymap Engine, FlexGrid, Input Guard (REQ-UX-01/02, REQ-ACC-01; #1, #2, #4, #63).
- `docs/future/MaxThink-Stellar.md` - requirement trace for personas and metrics (REQ-UX-01/02, REQ-ACC-01, REQ-PERF-01; #1, #2, #4, #6, #16, #63).
- `docs/future/stellar-tui-vision.md` - shared CLI/TUI parity vision and accessibility baselines.
- `docs/future/stellar/definition-of-done.md` - Definition of Done guardrails for unit, snapshot, accessibility, security, observability.

## Checklist
- [x] KEYMAP - Persona aligned keymap and command routes confirmed (REQ-UX-01, REQ-ACC-01; #1, #2, #6).
- [x] FLEXGRID - FlexGrid baseline and fallback layouts approved (REQ-UX-02; #4, #63).
- [x] MICROCOPY - Golden Path and Progressive Disclosure copy agreed (REQ-UX-01, REQ-ACC-01; #1, #45).

## Outputs
- Persona-aware keymap matrix and command routing notes for `stellar_core::command_router` with CLI parity.
- FlexGrid layout specification (>=120 column baseline, 80-119 column fallback) tracing REQ-UX-02 (#4, #63) and accessibility hooks from REQ-ACC-01.
- Golden Path microcopy set and Progressive Disclosure triggers for Insight Canvas and CLI mirrors.

---

## 1. Keymap and Command Routes (REQ-UX-01, REQ-ACC-01; #1, #2, #6, #62)

### 1.1 Action Vocabulary
| Action ID | Description | Requirement Trace | CLI Alias |
| --- | --- | --- | --- |
| `core.navigate.next_pane` | Move focus to the next FlexGrid segment | REQ-UX-01 (#1), REQ-ACC-01 (#6) | `codex stellar focus --next`
| `core.navigate.prev_pane` | Return focus to the previous segment | REQ-UX-01 (#2) | `codex stellar focus --prev`
| `core.palette.open` | Open the command palette router | REQ-UX-01 (#1), REQ-ACC-01 (#62) | `codex stellar command`
| `core.canvas.toggle` | Show or hide the Insight Canvas | REQ-UX-01 (#1), REQ-OBS-01 (#45) | `codex stellar canvas toggle`
| `core.overlay.telemetry` | Enable the Telemetry Overlay preview | REQ-OBS-01 (#45) | `codex stellar overlay telemetry`
| `core.runbook.invoke` | Launch the linked runbook | REQ-OPS-01 (#23) | `codex stellar runbook <id>`
| `core.input.undo` | Undo the last Insight Canvas action | REQ-UX-01 (#1), REQ-PERF-01 (#16) | `codex stellar insight undo`
| `core.input.redo` | Redo the last action | REQ-UX-01 (#1) | `codex stellar insight redo`
| `core.input.field_lock` | Lock the current field against auto updates | REQ-UX-01 (#1), REQ-DATA-01 (#67) | `codex stellar field lock`
| `core.input.confidence` | Show Confidence Bar and reason codes | REQ-DATA-01 (#67), REQ-UX-01 (#1) | `codex stellar insight confidence`
| `core.input.submit` | Confirm the current insight or command | REQ-UX-01 (#1) | `codex stellar insight submit`
| `core.accessibility.toggle` | Toggle high contrast or screen reader mode | REQ-ACC-01 (#6, #62, #87) | `codex stellar accessibility toggle`

### 1.2 Global Keymap (Baseline)
| Action ID | Default Binding | Assistive Alternate | Rationale |
| --- | --- | --- | --- |
| `core.navigate.next_pane` | `Tab` | `Ctrl+Right` | Linear navigation keeps screen reader order intact (REQ-ACC-01 #6).
| `core.navigate.prev_pane` | `Shift+Tab` | `Ctrl+Left` | Symmetric control supports braille display reversal.
| `core.palette.open` | `Ctrl+K` | `F1` | Matches existing CLI/TUI command palette patterns.
| `core.canvas.toggle` | `Ctrl+I` | `Ctrl+Alt+I` | Mnemonic for Insight Canvas with sticky modifier fallback (#1).
| `core.overlay.telemetry` | `Ctrl+O` | `F6` | Quick path to Observability Mesh overlays (#45).
| `core.runbook.invoke` | `Ctrl+R` | `F9` | Mnemonic `R` for runbook (REQ-OPS-01 #23).
| `core.input.undo` | `Ctrl+Z` | `Alt+Backspace` | Standard editing convention.
| `core.input.redo` | `Ctrl+Shift+Z` | `Ctrl+Y` | Standard redo pairing.
| `core.input.field_lock` | `Ctrl+L` | `Alt+L` | Direct mnemonic for Lock (#67).
| `core.input.confidence` | `i` (in-canvas) | `Shift+Enter` | Lightweight Progressive Disclosure (REQ-DATA-01 #67).
| `core.input.submit` | `Ctrl+Enter` | `Enter` with confirmation toggle | Prevents accidental submits.
| `core.accessibility.toggle` | `Ctrl+Alt+A` | `F10` | Keeps accessibility toggle away from regular typing.

### 1.3 Persona Overlays
| Persona | Primary Actions | Overlay Binding | Notes |
| --- | --- | --- | --- |
| Operator | `core.overlay.telemetry`, `core.runbook.invoke`, `core.input.confidence` | `Ctrl+O`, `Ctrl+R`, `i` | Aligns with Incident Timeline workflows (REQ-OBS-01 #45).
| SRE | `core.overlay.telemetry`, `core.palette.open` filtered for chaos tools | `Ctrl+Shift+T` | Command palette filtered by tag `role:sre` (REQ-REL-01 #14, #35).
| SecOps | `core.palette.open` scoped to security actions, `core.input.field_lock` | `Ctrl+Shift+K`, `Ctrl+L` | Requires guardrail confirmation (REQ-SEC-01 #9, #11, M3 follow-up).
| Platform Engineer | `core.runbook.invoke` -> pipeline scripts, `core.input.submit` -> approvals | `Alt+Enter` (submit override) | Role DSL enables override, keeps governance trace (#79).
| Assistive Tech Bridge | All actions exposed via alternates plus sequential hints | `F` keys, onscreen hint labels | Screen reader can recite bindings on `Shift+?` (REQ-ACC-01 #6).

### 1.4 Routing Pathways
- Command Router (`stellar_core::command_router`) normalizes terminal events to action IDs and adds persona metadata (`role`, `capabilities`, `assistive=true`).
- Overlay rules apply in priority order `assistive > role > global` (REQ-ACC-01 #6, RFC 0002 section 4.1).
- CLI/TUI parity: every routed action is emitted on `stellar_kernel::bridge::events` and serialized as JSON `{action, payload, source}` for CLI parity (`REQ-UX-01`, `stellar-tui-vision.md`). CLI requests land via the `codex stellar <action>` subcommand which mirrors the same payload format.
- Input Guard will call `guard::check(action, persona)` before execution, reusing security policy hooks planned for M3 (REQ-UX-01, #56).

## 2. FlexGrid Layout Specification (REQ-UX-02; #4, #63)

### 2.1 Baseline Layout (Width >= 120 columns)
```
+------------------------------+----------------------+
| Insight Canvas (70%)         | Telemetry Overlay    |
| - Prompt Field               | - Latency Heatmap    |
| - Suggestions Stack          | - Cache Hit Panel    |
| - Confidence Bar             | - Runbook Shortcuts  |
+--------------+---------------+----------------------+
| Command Log  | Golden Path Footer (two actions max) |
| (CLI parity) | - Submit - Open Overlay               |
+--------------+--------------------------------------+
```
- Grid tracks: `col0=7fr`, `col1=3fr`, `row0=minmax(12, auto)`, `row1=3`.
- Telemetry overlay collapses back into Insight Canvas when disabled, freeing `col1` for the runbook drawer (#45).

### 2.2 Narrow Layout (Width 80-119 columns)
```
+----------------------------------------------+
| Insight Canvas (stacked)                     |
| - Prompt Field                               |
| - Suggestions (accordion)                    |
| - Confidence Bar (inline)                    |
+----------------------------------------------+
| Tab Strip: [Telemetry] [Command Log] [Runbook]|
+----------------------------------------------+
| Golden Path Footer (stacked buttons)         |
+----------------------------------------------+
```
- `tab_strip` replaces the side overlay; keyboard navigation cycles with `Ctrl+Alt+Right/Left` (#63).
- Focus order: Canvas -> Footer -> Tabs (Telemetry -> Command Log -> Runbook) (REQ-ACC-01 #6).

### 2.3 CLI/TUI Bridge Alignment
- Command Log row mirrors CLI transcript output from `codex stellar --json` to maintain parity (`stellar-tui-vision.md`).
- FlexGrid template реализован программно в `codex-rs/tui/src/stellar/view.rs` через Layout API Ratatui (REQ-UX-02 #4).

### 2.4 Accessibility Notes
- Focus outlines stay visible through layout transitions; `prefix_lines` provides narration text for screen readers (REQ-ACC-01 #6, #62).
- High contrast palettes from `codex-rs/tui/styles.md` apply to Confidence Bar and Golden Path footer to satisfy WCAG AA.

## 3. Golden Path Microcopy and Progressive Disclosure (REQ-UX-01, REQ-ACC-01, REQ-OBS-01; #1, #45)

### 3.1 Golden Path Actions (two steps)
1. **Submit Insight** - Button text: "Send Insight" (Operator/SRE); CLI echo `> codex stellar insight submit`.
2. **Open Telemetry Overlay** - Secondary action: "Show Metrics (Ctrl+O)" referencing the key binding.
- Footer hint: `Need a runbook? Press Ctrl+R` (REQ-OPS-01 #23).

### 3.2 Progressive Disclosure Hints
- Press `i` or choose "Explain Confidence" to reveal a panel titled "Why this insight?" with bullet reasons from the Confidence Bar (#67).
- Screen reader summary appends `Confidence 0.82 - based on recent telemetry` (REQ-ACC-01 #6).
- Undo/Redo hint toast appears after the first edit: `Undo (Ctrl+Z) / Redo (Ctrl+Shift+Z)` wrapped with `textwrap::wrap` for compact layout support.

### 3.3 Microcopy Guardrails
- Tone: action oriented, under 45 characters per hint to maintain cognitive ease (Definition of Done accessibility clause).
- Avoid jargon; reuse nouns from `MaxThink-Stellar.md` (Insight, Telemetry, Runbook).
- Provide the same strings via CLI help: `codex stellar help insight` prints mirrored copy (REQ-UX-01).

### 3.4 Validation Hooks
- Snapshot baselines: `insight_canvas_default.snap` (wide) and `insight_canvas_compact.snap` (narrow) capture footer and copy variations.
- Accessibility smoke: `tests/accessibility/stellar_core_smoke.rs` asserts hint presence (REQ-ACC-01).
- APDEX instrumentation: timers around `core.input.submit` capture latency baseline (REQ-PERF-01 #16).

## 4. Traceability Summary
| Artifact | Requirement(s) | Blueprint Ref |
| --- | --- | --- |
| Keymap matrix and command routes | REQ-UX-01, REQ-ACC-01 | #1, #2, #6, #62 |
| FlexGrid layout specification | REQ-UX-02, REQ-ACC-01 | #4, #63 |
| Golden Path microcopy | REQ-UX-01, REQ-ACC-01, REQ-OBS-01, REQ-OPS-01 | #1, #23, #45 |

## 5. Next Build Actions
- Implement Command Router and Keymap Engine per section 1, emitting bridge events for CLI parity.
- Materialize FlexGrid templates and Golden Path footer UI in `codex-tui` with the referenced snapshot tests.
- Configure telemetry capture for APDEX and latency before entering the Validate phase.
