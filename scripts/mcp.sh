#!/usr/bin/env bash
set -euo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$DIR"
if [[ ! -f target/release/runpost_mcp ]]; then
  if [[ ! -f ui/dist/index.html ]]; then
    pushd ui >/dev/null
    npm ci --ignore-scripts >&2
    npm run build >&2
    popd >/dev/null
  fi
  cargo build --release --locked --quiet -p runpost-cli >&2
fi
export RUNPOST_WORKSPACE="${RUNPOST_WORKSPACE:-$DIR/workspace}"
exec "$DIR/target/release/runpost_mcp" mcp "$@"
