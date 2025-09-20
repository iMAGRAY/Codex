## Getting started

### CLI usage

| Command            | Purpose                            | Example                         |
| ------------------ | ---------------------------------- | ------------------------------- |
| `codex`            | Interactive TUI                    | `codex`                         |
| `codex "..."`      | Initial prompt for interactive TUI | `codex "fix lint errors"`       |
| `codex exec "..."` | Non-interactive "automation mode"  | `codex exec "explain utils.ts"` |
| `codex doc search` | Semantic lookup in docs            | `codex doc search "how to login"` |

Key flags: `--model/-m`, `--ask-for-approval/-a`.

### Resuming interactive sessions

- Run `codex resume` to display the session picker UI
- Resume most recent: `codex resume --last`
- Resume by id: `codex resume <SESSION_ID>` (You can get session ids from /status or `~/.codex/sessions/`)

Examples:

```shell
# Open a picker of recent sessions
codex resume

# Resume the most recent session
codex resume --last

# Resume a specific session by id
codex resume 7f9f9a2e-1b3c-4c7a-9b0e-123456789abc
```

### Running with a prompt as input

You can also run Codex CLI with a prompt as input:

```shell
codex "explain this codebase to me"
```

```shell
codex --full-auto "create the fanciest todo-list app"
```

That's it - Codex will scaffold a file, run it inside a sandbox, install any
missing dependencies, and show you the live result. Approve the changes and
they'll be committed to your working directory.

### Example prompts

Below are a few bite-size examples you can copy-paste. Replace the text in quotes with your own task.

| ✨  | What you type                                                                   | What happens                                                               |
| --- | ------------------------------------------------------------------------------- | -------------------------------------------------------------------------- |
| 1   | `codex "Refactor the Dashboard component to React Hooks"`                       | Codex rewrites the class component, runs `npm test`, and shows the diff.   |
| 2   | `codex "Generate SQL migrations for adding a users table"`                      | Infers your ORM, creates migration files, and runs them in a sandboxed DB. |
| 3   | `codex "Write unit tests for utils/date.ts"`                                    | Generates tests, executes them, and iterates until they pass.              |
| 4   | `codex "Bulk-rename *.jpeg -> *.jpg with git mv"`                               | Safely renames files and updates imports/usages.                           |
| 5   | `codex "Explain what this regex does: ^(?=.*[A-Z]).{8,}$"`                      | Outputs a step-by-step human explanation.                                  |
| 6   | `codex "Carefully review this repo, and propose 3 high impact well-scoped PRs"` | Suggests impactful PRs in the current codebase.                            |
| 7   | `codex "Look for vulnerabilities and create a security review report"`          | Finds and explains security bugs.                                          |

### Memory with AGENTS.md

You can give Codex extra instructions and guidance using `AGENTS.md` files. Codex looks for `AGENTS.md` files in the following places, and merges them top-down:

1. `~/.codex/AGENTS.md` - personal global guidance
2. `AGENTS.md` at repo root - shared project notes
3. `AGENTS.md` in the current working directory - sub-folder/feature specifics

For more information on how to use AGENTS.md, see the [official AGENTS.md documentation](https://agents.md/).

### Tips & shortcuts

#### Use `@` for file search

Typing `@` triggers a fuzzy-filename search over the workspace root. Use up/down to select among the results and Tab or Enter to replace the `@` with the selected path. You can use Esc to cancel the search.

#### MCP wizard file browser *(REQ-DX-01)*

When the MCP manager or wizard is open, press `f` to launch the workspace-aware file browser. Use the arrow keys to navigate, `→` to enter directories, and `Enter` to select the highlighted path; this pre-fills the wizard's **Source path** field for automatic intake.

#### Image input

Paste images directly into the composer (Ctrl+V / Cmd+V) to attach them to your prompt. You can also attach files via the CLI using `-i/--image` (comma‑separated):

```bash
codex -i screenshot.png "Explain this error"
codex --image img1.png,img2.jpg "Summarize these diagrams"
```

#### Esc–Esc to edit a previous message

When the chat composer is empty, press Esc to prime “backtrack” mode. Press Esc again to open a transcript preview highlighting the last user message; press Esc repeatedly to step to older user messages. Press Enter to confirm and Codex will fork the conversation from that point, trim the visible transcript accordingly, and pre‑fill the composer with the selected user message so you can edit and resubmit it.

In the transcript preview, the footer shows an `Esc edit prev` hint while editing is active.

#### Ctrl+O Observability overlay

Press `Ctrl+O` to open the Stellar Observability Overlay (REQ-OBS-01/REQ-OPS-01; see `docs/future/MaxThink-Stellar.md`). The overlay highlights latency p95, audit fallback count, and cache hit percentage, and surfaces an `[ Investigate ]` hint that maps to persona-specific runbooks. For a deeper walkthrough, follow [docs/stellar-quickstart.md](stellar-quickstart.md).

#### Trusted pipeline commands

The CLI exposes a lightweight trusted release flow (REQ-OPS-01/REQ-INT-01/REQ-DX-01):

```shell
export CODEX_PIPELINE_SIGNING_KEY="<base64url-ed25519-secret>"
codex pipeline sign --name insight --version 1.4.0 --source packs/insight --signer vault:pipeline/insight
codex pipeline verify dist/insight-1.4.0.tar.gz --expect-fingerprint <fingerprint> --install
codex pipeline rollback insight 1.3.5
```

`sign` writes the bundle + manifest into `$CODEX_HOME/pipeline` and records an immutable audit event, `verify` validates the signature/diff (optionally installing the payload), and `rollback` reactivates a previously installed version. See the signed pipeline quickstart for validation steps and traceability requirements.

#### Weekly triage

Run the orchestrator triage helper to capture APDEX, latency, audit fallbacks, and review effort in one snapshot:

```shell
codex orchestrator triage --persona operator --review-hours 5.0
```

Targets can be tuned via `--apdex-target`, `--latency-target-ms`, `--audit-target`, and `--review-target-hours`. Archive the output in `docs/future/stellar/metrics-baseline.md` and update the weekly checklist when the command highlights yellow/red statuses.

#### Shell completions

Generate shell completion scripts via:

```shell
codex completion bash
codex completion zsh
codex completion fish
```

#### `--cd`/`-C` flag

Sometimes it is not convenient to `cd` to the directory you want Codex to use as the "working root" before running Codex. Fortunately, `codex` supports a `--cd` option so you can specify whatever folder you want. You can confirm that Codex is honoring `--cd` by double-checking the **workdir** it reports in the TUI at the start of a new session.

### Semantic documentation search

To enable semantic search, install the lightweight Python dependencies once:

```bash
python -m pip install -r requirements-docsearch.txt
```

Index the docs (you can rerun when documentation changes):

```bash
codex doc index --docs-root docs --recursive
```

Query the index:

```bash
codex doc search "как аутентифицироваться" --show-text
```

Use `--model-path` if EmbeddingGemma is in a custom location, and `--truncate-dim` to switch between 768/512/256/128 Matryoshka profiles.

### Semantic memory (локально в `~/.codex/memory/memory.jsonl`)

```bash
# Сохранить вывод в память с тегами
codex memory remember "apply_patch --dry-run перед любыми правками" --tag apply_patch --tag workflow

# Посмотреть записи
codex memory list --tag apply_patch

# Семантический поиск по памяти
codex memory search "как откатить патч" --show-text

# Удалить записи по тегу или ID
codex memory forget --tag workflow

# Очистить память, оставив максимум 200 записей
codex memory prune --max-records 200
```
