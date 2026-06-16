#!/usr/bin/env bash
# #550 build-rate measurement: time the CURRENT (in-memory) bench-vault builder at
# small scales to extrapolate the 1e8 build cost and RAM ceiling. Honest data for
# deciding the streaming-builder rework. dim=512 slots=1 per the card geometry.
set -uo pipefail
source /home/croyse/calyx_env.sh
BIN=/home/croyse/calyx/repo/target/release/calyx
OUT=/tmp/ph68-rate
rm -rf "$OUT"; mkdir -p "$OUT"
for N in 100000 300000 1000000 3000000; do
  V="$OUT/v_${N}"
  /usr/bin/time -v "$BIN" build-bench-vault --vault "$V" --n-cx "$N" --dim 512 --slots 1 --seed 42 \
    > "$OUT/build_${N}.json" 2> "$OUT/time_${N}.txt"
  RC=$?
  SEC=$(grep -oP 'Elapsed.*?:\s*\K[0-9:.]+' "$OUT/time_${N}.txt" | tail -1)
  RSS=$(grep -oP 'Maximum resident set size \(kbytes\):\s*\K[0-9]+' "$OUT/time_${N}.txt")
  GRAPH=$(ls -l "$V/idx/slot_00.ann/graph.cda" 2>/dev/null | awk '{print $5}')
  echo "N=$N rc=$RC elapsed=$SEC max_rss_kb=$RSS graph_bytes=$GRAPH"
done
echo "=== DONE measure_ph68_build_rate ==="
