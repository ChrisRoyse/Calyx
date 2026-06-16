#!/usr/bin/env bash
# #550 intermediate-scale validation: build a partitioned vault + search it,
# capturing wall time, max RSS (memory-bounded proof), p99 latency, self-recall.
# Arg 1: n_cx, Arg 2: n_regions, Arg 3: dim (default 512).
set -uo pipefail
source /home/croyse/calyx_env.sh
N="${1:?n_cx required}"
R="${2:?regions required}"
DIM="${3:-512}"
BIN=/home/croyse/calyx/repo/target/debug/calyx
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
ROOT="/zfs/hot/calyx/ph68-part-${N}-${STAMP}"
mkdir -p "$ROOT"
echo "=== BUILD n_cx=$N regions=$R dim=$DIM root=$ROOT ==="
/usr/bin/time -v "$BIN" build-partitioned-vault --vault "$ROOT" --n-cx "$N" --dim "$DIM" --regions "$R" --seed 42 \
  > "$ROOT/build.json" 2> "$ROOT/build.time"
echo "--- build.json ---"; cat "$ROOT/build.json"
echo "--- build wall/RSS ---"; grep -E "Elapsed|Maximum resident" "$ROOT/build.time"
echo "=== disk layout ==="
ls -lh "$ROOT/idx/slot_00.ann/graph.cda" "$ROOT/idx/slot_00.sparse/centroids.spn" 2>&1 | head
echo "region graphs: $(ls "$ROOT"/idx/region_*.ann/graph.cda 2>/dev/null | wc -l)"
echo "=== SEARCH 1000 queries k=10 n-probe=8 ==="
/usr/bin/time -v "$BIN" bench partitioned-search --vault "$ROOT" --n 1000 --k 10 --n-probe 8 --region-beam 64 --seed 42 \
  > "$ROOT/search.json" 2> "$ROOT/search.time"
echo "--- search.json ---"; cat "$ROOT/search.json"
echo "--- search RSS ---"; grep -E "Maximum resident" "$ROOT/search.time"
echo "ROOT=$ROOT"
echo "=== DONE run_part_validate ==="
