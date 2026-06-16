#!/usr/bin/env bash
# Re-measure the partitioned vault in RELEASE (the debug numbers were misleading).
set -uo pipefail
source /home/croyse/calyx_env.sh
N="${1:?n_cx}"; R="${2:?regions}"; DIM="${3:-512}"
echo "=== building calyx-cli --release ==="
cargo build -q --release -p calyx-cli 2>&1 | grep -E "error" | head
BIN=/home/croyse/calyx/repo/target/release/calyx
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
ROOT="/zfs/hot/calyx/ph68-rel-${N}-${STAMP}"
mkdir -p "$ROOT"
echo "=== BUILD (release) n=$N regions=$R dim=$DIM ==="
/usr/bin/time -v "$BIN" build-partitioned-vault --vault "$ROOT" --n-cx "$N" --dim "$DIM" --regions "$R" --seed 42 \
  > "$ROOT/build.json" 2> "$ROOT/build.time"
grep -E "build_seconds" "$ROOT/build.json"; grep -E "Elapsed|Maximum resident" "$ROOT/build.time"
echo "=== SEARCH (release) 1000 q, k=10, n-probe=8, beam=64 ==="
"$BIN" bench partitioned-search --vault "$ROOT" --n 1000 --k 10 --n-probe 8 --region-beam 64 --seed 42 | grep -E "p50|p99|p999|self_recall"
echo "ROOT=$ROOT"
echo "=== DONE ==="
