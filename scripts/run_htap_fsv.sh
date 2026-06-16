#!/usr/bin/env bash
set -uo pipefail
source /home/croyse/calyx_env.sh
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
VAULT="/home/croyse/calyx/data/fsv-issue587-htap-${STAMP}/vault"
OUT="/home/croyse/calyx/data/fsv-issue587-htap-${STAMP}/fsv"
mkdir -p "$VAULT" "$OUT"
echo "VAULT=$VAULT"
echo "=== build calyx-cli ==="
cargo build -q -p calyx-cli 2>&1 | tail -5
echo "=== run calyx htap-validate ==="
./target/debug/calyx htap-validate --vault "$VAULT" --out "$OUT" --rows 256 --dim 4 --value-column 1
echo "=== INDEPENDENT READBACK: artifacts on disk ==="
ls -la "$OUT"
echo "--- htap-report.json ---"; cat "$OUT/htap-report.json"
echo "--- htap-blake3.json ---"; cat "$OUT/htap-blake3.json"
echo "--- materialized Arrow column manifest (column path SoT) ---"
cat "$OUT/col-main/slot-column-manifest.json" 2>/dev/null | head -20
echo "FSV_DIR=/home/croyse/calyx/data/fsv-issue587-htap-${STAMP}"
echo "=== DONE run_htap_fsv ==="
