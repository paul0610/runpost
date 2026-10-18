#!/usr/bin/env bash
set -euo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$DIR"
if [[ ! -f target/release/runpost_mcp ]]; then
  bash scripts/build.sh
fi
export RUNPOST_WORKSPACE="${RUNPOST_WORKSPACE:-$DIR/workspace}"
exec "$DIR/target/release/runpost_mcp" serve --browser --open --port 7878 "$@"
