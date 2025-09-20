#!/usr/bin/env bash
set -euo pipefail

if ! command -v apply_patch >/dev/null 2>&1; then
  echo "apply_patch binary not found in PATH" >&2
  exit 1
fi

temp_dir=$(mktemp -d)
trap 'rm -rf "$temp_dir"' EXIT

cp "$(dirname "$0")/sample_patch.patch" "$temp_dir/patch.txt"

pushd "$temp_dir" >/dev/null

printf '\n== Dry-run preview ==\n'
apply_patch --dry-run --yes "$(cat patch.txt)"

printf '\n== Applying patch with undo history ==\n'
apply_patch --yes "$(cat patch.txt)"
ls -l demo.txt

printf '\n== Undoing last patch ==\n'
apply_patch --undo-last

if [[ -f demo.txt ]]; then
  echo "Expected demo.txt to be removed by undo." >&2
  exit 1
fi

printf '\nAll apply_patch demos succeeded in %s\n' "$temp_dir"
