#!/usr/bin/env bash
set -euo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$DIR"
if [[ ! -f target/release/runpost_app ]]; then
  bash scripts/build.sh
fi
export RUNPOST_WORKSPACE="${RUNPOST_WORKSPACE:-$DIR/workspace}"
if [[ "$(uname -s)" == Darwin && -d target/release/bundle/macos/RunPost.app ]]; then
  exec open -a "$DIR/target/release/bundle/macos/RunPost.app" --args "$@"
fi
exec "$DIR/target/release/runpost_app" "$@"
