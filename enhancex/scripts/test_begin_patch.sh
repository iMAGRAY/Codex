#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"

pushd "$ROOT_DIR/codex-rs" >/dev/null
cargo test -p codex-apply-patch maybe_parse_begin_patch
popd >/dev/null
