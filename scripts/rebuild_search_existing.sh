#!/usr/bin/env bash
set -uo pipefail
source /home/croyse/calyx_env.sh
ROOT="${1:?vault root required}"
echo "=== rebuild cli ==="
cargo build -q -p calyx-cli 2>&1 | grep -E "error" | head -20
BIN=/home/croyse/calyx/repo/target/debug/calyx
echo "=== search after prefetch fix (beam 64 probe 8) ==="
"$BIN" bench partitioned-search --vault "$ROOT" --n 1000 --k 10 --n-probe 8 --region-beam 64 --seed 42 | grep -E "p50|p99|p999|self_recall"
echo "=== DONE ==="
