#!/usr/bin/env bash
# Re-measure the partitioned vault in RELEASE with PARALLEL region builds (#706).
# Also dumps region-size distribution to diagnose assignment skew.
set -uo pipefail
source /home/croyse/calyx_env.sh
N="${1:?n_cx}"; R="${2:?regions}"; DIM="${3:-512}"
echo "=== building calyx-cli --release ==="
cargo build -q --release -p calyx-cli 2>&1 | grep -E "error" | head
BIN=/home/croyse/calyx/repo/target/release/calyx
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
ROOT="/zfs/hot/calyx/ph68-rel-${N}-${STAMP}"
mkdir -p "$ROOT"
echo "=== BUILD (release, parallel regions) n=$N regions=$R dim=$DIM ==="
date -u +"build_start=%Y-%m-%dT%H:%M:%SZ"
/usr/bin/time -v "$BIN" build-partitioned-vault --vault "$ROOT" --n-cx "$N" --dim "$DIM" --regions "$R" --seed 42 \
  > "$ROOT/build.json" 2> "$ROOT/build.time"
date -u +"build_end=%Y-%m-%dT%H:%M:%SZ"
grep -E "build_seconds" "$ROOT/build.json"; grep -E "Elapsed|Maximum resident" "$ROOT/build.time"
echo "=== REGION SIZE DISTRIBUTION ==="
python3 - "$ROOT/partitioned-manifest.json" <<'PY'
import json,sys
m=json.load(open(sys.argv[1]))
c=sorted(r["count"] for r in m["regions"])
n=len(c); tot=sum(c)
def pct(p): return c[min(n-1,int(p*n))]
print(f"regions={n} total_cx={tot} min={c[0]} p50={pct(0.5)} p90={pct(0.9)} p99={pct(0.99)} max={c[-1]} mean={tot//n}")
PY
echo "=== SEARCH (release) 1000 q, k=10, n-probe=8, beam=64, ground-truth=100 ==="
"$BIN" bench partitioned-search --vault "$ROOT" --n 1000 --k 10 --n-probe 8 --region-beam 64 --ground-truth 100 --seed 42 \
  | grep -E "p50|p99|p999|self_recall|ground_truth"
echo "ROOT=$ROOT"
echo "=== DONE ==="
