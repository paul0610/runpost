#!/usr/bin/env bash
set -euo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$DIR"
command -v cargo >/dev/null 2>&1 || { echo '[runpost] Falta Rust: https://rustup.rs' >&2; exit 1; }
command -v npm >/dev/null 2>&1 || { echo '[runpost] Falta Node.js LTS' >&2; exit 1; }
export RUNPOST_WORKSPACE="${RUNPOST_WORKSPACE:-$DIR/workspace}"
pushd ui >/dev/null
npm ci --ignore-scripts
popd >/dev/null
pushd apps/desktop >/dev/null
npm ci --ignore-scripts
npm run tauri -- dev -- -- --dev "$@"
