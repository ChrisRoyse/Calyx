#!/usr/bin/env bash
# Search-latency sweep on an existing partitioned vault (build already done).
set -uo pipefail
source /home/croyse/calyx_env.sh
ROOT="${1:?vault root required}"
BIN=/home/croyse/calyx/repo/target/debug/calyx
for BEAM in 64 32 16; do
  for PROBE in 4 2 1; do
    echo "--- region-beam=$BEAM n-probe=$PROBE ---"
    "$BIN" bench partitioned-search --vault "$ROOT" --n 500 --k 10 --n-probe "$PROBE" --region-beam "$BEAM" --seed 42 \
      | grep -E "p50|p99|self_recall"
  done
done
echo "=== DONE sweep ==="
