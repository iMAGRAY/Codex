# Enhancex Layer

This branch extends `openai/codex` with begin_patch integration and auxiliary tooling.

Structure:

- `overlays/` — declarative patches applied on top of upstream main.
- `scripts/` — automation (sync-with-upstream, apply-overlays, smoke tests).
- `docs/` — operations manuals and regression policies.
- `submodules/Apply_Patch` — external toolchain (added later via `git submodule add`).

All custom code must live under `enhancex/` to keep merges from upstream predictable.
- `modes/begin_patch/` — begin_patch toolchain (git submodule).

