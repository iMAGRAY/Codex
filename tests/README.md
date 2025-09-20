# apply_patch demo checks

This folder contains a tiny smoke script for the enhanced `apply_patch` tool.

## Scripts

- `run_apply_patch_demo.sh` — spins up a temp directory, exercises:
  1. `--dry-run` preview (non-destructive summary)
  2. `--yes` non-interactive apply
  3. `--undo-last` rollback from the generated history file

## Usage

```bash
./tests/run_apply_patch_demo.sh
```

The script prints each phase and fails if undo leaves the sample file behind.
