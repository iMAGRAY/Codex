#!/usr/bin/env bash
set -euo pipefail

dir="$(cd "$(dirname "$0")/.." && pwd)"
patch_dir="$dir/overlays"

if ! command -v git >/dev/null; then
  echo "git is required" >&2
  exit 1
fi

if ! command -v quilt >/dev/null 2>/dev/null; then
  while read -r patch; do
    [ -z "$patch" ] && continue
    patch_path="$patch_dir/$patch"
    if [ ! -f "$patch_path" ]; then
      echo "Missing patch $patch_path" >&2
      exit 1
    fi
    echo "Applying $patch"
    git apply --3way "$patch_path"
  done <"$patch_dir/series"
else
  pushd "$patch_dir" >/dev/null
  quilt push -a
  popd >/dev/null
fi
