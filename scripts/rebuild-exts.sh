#!/usr/bin/env bash
# rebuild-exts.sh — rebuild every dynamically-loaded `hnl` extension.
#
# `hnl` is a dumb host: each subcommand (`test`, `sdkgen`, `fmt`, `lint`,
# `typecheck`, ...) is a cdylib (`libhnl_ext_<name>.so`) built from a
# sibling `ext-*` repo and loaded from $CARGO_TARGET_DIR/debug at runtime.
# When a shared dependency (kit-core, lib-*, kernel) changes, an
# un-rebuilt .so silently runs STALE code — this rebuilds all of them.
#
# Usage:  ./scripts/rebuild-exts.sh [ext-name...]   # no args = all ext-*
# Env:    CARGO_LOCK_HELD=1 is set automatically (flock wrapper).
set -euo pipefail

WS="$(cd "$(dirname "$0")/../.." && pwd)"
CARGO_LOCK_HELD="${CARGO_LOCK_HELD:-1}"
export CARGO_LOCK_HELD CARGO_PROFILE_DEV_DEBUG=0

if [ $# -gt 0 ]; then
  dirs=("$@")
else
  dirs=("$WS"/ext-*)
fi

for d in "${dirs[@]}"; do
  name="$(basename "$d")"
  [ -f "$d/Cargo.toml" ] || { echo "== $name: no Cargo.toml, skipped"; continue; }
  echo "== $name"
  (cd "$d" && cargo build 2>&1 | tail -2)
done

echo "== built:"
ls -1 "${CARGO_TARGET_DIR:-$HOME/.cargo/shared-target}"/debug/libhnl_ext_*.so 2>/dev/null | while read -r so; do
  printf '   %-50s %s\n' "$(basename "$so")" "$(date -r "$so" '+%H:%M')"
done
