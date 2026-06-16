#!/usr/bin/env bash
# Rebuild calyx-cli --release and run the partitioned search bench (with true
# ground-truth recall) against an EXISTING vault ROOT. No vault rebuild.
set -uo pipefail
source /home/croyse/calyx_env.sh
ROOT="${1:?vault root}"
GT="${2:-100}"
echo "=== building calyx-cli --release ==="
cargo build -q --release -p calyx-cli 2>&1 | grep -E "error" | head
BIN=/home/croyse/calyx/repo/target/release/calyx
echo "=== SEARCH (release) 1000 q, k=10, n-probe=8, beam=64, ground-truth=$GT ==="
/usr/bin/time -v "$BIN" bench partitioned-search --vault "$ROOT" --n 1000 --k 10 --n-probe 8 --region-beam 64 --ground-truth "$GT" --seed 42 \
  2> "$ROOT/search.time" | tee "$ROOT/search.json"
echo "--- search wall/RSS ---"
grep -E "Elapsed|Maximum resident" "$ROOT/search.time"
echo "ROOT=$ROOT"
echo "=== DONE ==="
