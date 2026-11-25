#!/usr/bin/env bash
# Builds the RTW artifact for the simulation package.
#
# Requires the rtwKit toolchain: `wit-bindgen` for the guest and `tools/componentize`
# (or a cargo component build) for the final `backend.wasm`. Until `wit/engine.wit`
# and the exact exported entry points are pinned (see src/wasm.rs), this script only
# assembles the artifact root from the manifests so the layout can be reviewed.
set -euo pipefail

root="$(cd "$(dirname "$0")" && pwd)"
artifact="$root/build/rtw"

rm -rf "$artifact"
mkdir -p "$artifact"
cp "$root/rtw/rtw.toml" "$artifact/rtw.toml"
cp "$root/rtw/manifest.toml" "$artifact/manifest.toml"

if [[ ! -f "$artifact/backend.wasm" ]]; then
  echo "backend.wasm is not built yet: see packages/sim/src/wasm.rs for the blockers" >&2
  exit 1
fi
