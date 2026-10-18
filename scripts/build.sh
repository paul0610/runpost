#!/usr/bin/env bash
set -euo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$DIR"
command -v cargo >/dev/null 2>&1 || { echo '[runpost] Falta Rust: https://rustup.rs' >&2; exit 1; }
command -v npm >/dev/null 2>&1 || { echo '[runpost] Falta Node.js LTS' >&2; exit 1; }
pushd ui >/dev/null
npm ci --ignore-scripts
npm run build
popd >/dev/null
cargo build --release --locked -p runpost-cli
pushd apps/desktop >/dev/null
npm ci --ignore-scripts
# SPEC-DEVIATION: CLI Tauri local fijado en package-lock.json; mismo comando Rust que cargo tauri, sin instalación global.
npm run tauri -- build --no-bundle --ci -- --locked
popd >/dev/null
echo '[runpost] OK: target/release/runpost_mcp y target/release/runpost_app'
