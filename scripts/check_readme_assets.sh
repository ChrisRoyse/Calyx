#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

if [[ -f "$HOME/.cargo/env" ]]; then
  source "$HOME/.cargo/env"
fi

export CALYX_FSV_ROOT="${CALYX_FSV_ROOT:-$repo_root/target/fsv/readme-assets-contract}"
mkdir -p "$CALYX_FSV_ROOT"

cargo test -p calyx-cli --test readme_assets_contract -- --nocapture

readback="$CALYX_FSV_ROOT/readme-assets-contract-readback.json"
if [[ ! -s "$readback" ]]; then
  echo "ERROR: README asset contract readback was not written: $readback" >&2
  exit 1
fi

edge_audit="$CALYX_FSV_ROOT/readme-assets-edge-audit.json"
if [[ ! -s "$edge_audit" ]]; then
  echo "ERROR: README asset edge-audit readback was not written: $edge_audit" >&2
  exit 1
fi

echo "README asset contract source-of-truth readback: $readback"
echo "README asset contract edge-audit readback: $edge_audit"
