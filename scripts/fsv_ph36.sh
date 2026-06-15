#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

source "$HOME/.cargo/env"
cd "$repo_root"

if [[ -f "$repo_root/env.sh" ]]; then
  source "$repo_root/env.sh"
fi

root="${CALYX_FSV_ROOT:-/home/croyse/calyx/data/fsv-ph36-integration-$(date -u +%Y%m%dT%H%M%SZ)}"
export CALYX_FSV_ROOT="$root"
mkdir -p "$root"

log="$root/ph36-fsv.log"
xxd_log="$log.xxd"

cargo test -p calyx-cli --test ph36_fsv_integration ph36_fsv_integration_aiwonder -- --ignored --nocapture 2>&1 |
  tee "$log"
xxd "$log" > "$xxd_log"

grep -F "PH36 FSV PASS: tamper detected at seq=11" "$log"
grep -F "reproduce max_drift=" "$log"
grep -F "CALYX_LEDGER_CHAIN_BROKEN at seq=11" "$log"

echo "PH36_FSV_LOG=$log"
echo "PH36_FSV_LOG_XXD=$xxd_log"
