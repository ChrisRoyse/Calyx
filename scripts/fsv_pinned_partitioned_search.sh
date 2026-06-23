#!/usr/bin/env bash
set -uo pipefail

usage() {
  cat >&2 <<'USAGE'
usage: scripts/fsv_pinned_partitioned_search.sh --root <fsv-root> --vault <vault> --bin <pinned-calyx> --out <out-dir> [options]

Options:
  --n <queries>              default: 1000
  --k <k>                    default: 10
  --n-probe <n>              default: 8
  --region-beam <n>          default: 64
  --slo-us <microseconds>    default: 25000

The script refuses to rely on mutable target/release paths. It runs the supplied
pinned binary and writes pinned_partitioned_search_readback.json even on failure.
USAGE
}

root=""
vault=""
bin_path=""
out_dir=""
n=1000
k=10
n_probe=8
region_beam=64
slo_us=25000

while [ "$#" -gt 0 ]; do
  case "$1" in
    --root) root="${2:-}"; shift 2 ;;
    --vault) vault="${2:-}"; shift 2 ;;
    --bin) bin_path="${2:-}"; shift 2 ;;
    --out) out_dir="${2:-}"; shift 2 ;;
    --n) n="${2:-}"; shift 2 ;;
    --k) k="${2:-}"; shift 2 ;;
    --n-probe) n_probe="${2:-}"; shift 2 ;;
    --region-beam) region_beam="${2:-}"; shift 2 ;;
    --slo-us) slo_us="${2:-}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown arg: $1" >&2; usage; exit 2 ;;
  esac
done

if [ -z "$root" ] || [ -z "$vault" ] || [ -z "$bin_path" ] || [ -z "$out_dir" ]; then
  usage
  exit 2
fi

write_readback() {
  local exit_code="$1"
  local phase="$2"
  local error_code="${3:-}"
  local error_message="${4:-}"
  READBACK_EXIT_CODE="$exit_code" \
  READBACK_PHASE="$phase" \
  READBACK_ERROR_CODE="$error_code" \
  READBACK_ERROR_MESSAGE="$error_message" \
  READBACK_ROOT="$root" \
  READBACK_VAULT="$vault" \
  READBACK_BIN="$bin_path" \
  READBACK_OUT="$out_dir" \
  READBACK_N="$n" \
  READBACK_K="$k" \
  READBACK_N_PROBE="$n_probe" \
  READBACK_REGION_BEAM="$region_beam" \
  READBACK_SLO_US="$slo_us" \
  python3 - <<'PY'
import hashlib
import json
import os
import pathlib
import time

root = pathlib.Path(os.environ["READBACK_ROOT"])
vault = pathlib.Path(os.environ["READBACK_VAULT"])
bin_path = pathlib.Path(os.environ["READBACK_BIN"])
out_dir = pathlib.Path(os.environ["READBACK_OUT"])
exit_code = int(os.environ["READBACK_EXIT_CODE"])
phase = os.environ["READBACK_PHASE"]
error_code = os.environ["READBACK_ERROR_CODE"] or None
error_message = os.environ["READBACK_ERROR_MESSAGE"] or None

def meta(path):
    path = pathlib.Path(path)
    if not path.exists():
        return {"path": str(path), "exists": False, "bytes": 0, "sha256": None}
    data = path.read_bytes()
    return {
        "path": str(path),
        "exists": True,
        "bytes": len(data),
        "sha256": hashlib.sha256(data).hexdigest(),
    }

def load_json(path):
    path = pathlib.Path(path)
    if not path.exists():
        return None
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except Exception as error:
        return {"_parse_error": str(error)}

def pct(values, percentile):
    if not values:
        return None
    idx = min(len(values) - 1, round((len(values) - 1) * percentile / 100))
    return values[idx]

manifest_path = vault / "partitioned-manifest.json"
manifest = load_json(manifest_path) or {}
regions = manifest.get("regions") if isinstance(manifest, dict) else None
counts = sorted(
    int(region.get("count", 0))
    for region in regions or []
    if isinstance(region, dict)
)
search = load_json(out_dir / "search_stdout.json") or {}
status = load_json(out_dir / "anneal_partitioned_search/bench/bw_postcutoff_status.json") or {}
latency = search.get("latency_us") if isinstance(search, dict) else None
recall = None
if isinstance(search, dict):
    recall = search.get("ground_truth_recall_at_k")
    if recall is None:
        recall = search.get("self_recall_at_k")

readback = {
    "format": "calyx-fsv-pinned-partitioned-search-readback-v1",
    "utc_time": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
    "phase": phase,
    "exit_code": exit_code,
    "error_code": error_code,
    "error_message": error_message,
    "root": str(root),
    "vault": str(vault),
    "out": str(out_dir),
    "parameters": {
        "n": int(os.environ["READBACK_N"]),
        "k": int(os.environ["READBACK_K"]),
        "n_probe": int(os.environ["READBACK_N_PROBE"]),
        "region_beam": int(os.environ["READBACK_REGION_BEAM"]),
        "slo_us": int(os.environ["READBACK_SLO_US"]),
    },
    "pinned_binary": meta(bin_path),
    "pinned_binary_manifest": meta(bin_path.with_name(bin_path.name + ".pin.json")),
    "manifest": meta(manifest_path),
    "graph_files": len(list((vault / "idx").glob("**/graph.cda"))) if (vault / "idx").exists() else 0,
    "root_graph_exists": (vault / "idx/slot_00.ann/graph.cda").exists(),
    "n_cx": manifest.get("n_cx") if isinstance(manifest, dict) else None,
    "dim": manifest.get("dim") if isinstance(manifest, dict) else None,
    "n_regions": manifest.get("n_regions") if isinstance(manifest, dict) else None,
    "stored_region_members": manifest.get("stored_region_members") if isinstance(manifest, dict) else None,
    "final_assignment_probe": manifest.get("final_assignment_probe") if isinstance(manifest, dict) else None,
    "final_assignment_cap": manifest.get("final_assignment_cap") if isinstance(manifest, dict) else None,
    "m_max": manifest.get("m_max") if isinstance(manifest, dict) else None,
    "ef_construction": manifest.get("ef_construction") if isinstance(manifest, dict) else None,
    "graph_build_backend": manifest.get("graph_build_backend") if isinstance(manifest, dict) else None,
    "region_count_p50": pct(counts, 50),
    "region_count_p90": pct(counts, 90),
    "region_count_p95": pct(counts, 95),
    "region_count_p99": pct(counts, 99),
    "search_stdout": meta(out_dir / "search_stdout.json"),
    "search_stderr": meta(out_dir / "search_stderr.log"),
    "search": {
        "queries": search.get("queries") if isinstance(search, dict) else None,
        "k": search.get("k") if isinstance(search, dict) else None,
        "n_probe": search.get("n_probe") if isinstance(search, dict) else None,
        "region_beam": search.get("region_beam") if isinstance(search, dict) else None,
        "latency_us": latency,
        "self_recall_at_k": search.get("self_recall_at_k") if isinstance(search, dict) else None,
        "ground_truth_queries": search.get("ground_truth_queries") if isinstance(search, dict) else None,
        "ground_truth_recall_at_k": search.get("ground_truth_recall_at_k") if isinstance(search, dict) else None,
        "grounded_phase_exit_eligible": search.get("grounded_phase_exit_eligible") if isinstance(search, dict) else None,
        "metric_class": search.get("metric_class") if isinstance(search, dict) else None,
    },
    "tuner_status": {
        "artifact": meta(out_dir / "anneal_partitioned_search/bench/bw_postcutoff_status.json"),
        "mode": status.get("mode") if isinstance(status, dict) else None,
        "observations": status.get("observations") if isinstance(status, dict) else None,
        "posting_cutoff_semantic": status.get("posting_cutoff_semantic") if isinstance(status, dict) else None,
        "recall_observation_mode": status.get("recall_observation_mode") if isinstance(status, dict) else None,
        "ledger_event_count": len(status.get("ledger_entries", [])) if isinstance(status, dict) else 0,
        "aggregate_recall_at_k": status.get("aggregate_recall_at_k") if isinstance(status, dict) else None,
    },
}
p99 = latency.get("p99") if isinstance(latency, dict) else None
readback["p99_under_slo"] = p99 is not None and p99 < int(os.environ["READBACK_SLO_US"])
readback["recall_at_k"] = recall
readback["recall_ge_085"] = recall is not None and recall >= 0.85
out_dir.mkdir(parents=True, exist_ok=True)
path = out_dir / "pinned_partitioned_search_readback.json"
path.write_text(json.dumps(readback, indent=2, sort_keys=True), encoding="utf-8")
summary = {
    "readback": meta(path),
    "phase": phase,
    "exit_code": exit_code,
    "error_code": error_code,
    "p99_under_slo": readback["p99_under_slo"],
    "recall_ge_085": readback["recall_ge_085"],
}
print(json.dumps(summary, indent=2, sort_keys=True))
PY
}

mkdir -p "$out_dir"
case "$root" in /home/croyse/calyx/fsv/*) ;;
  *) write_readback 2 precondition_failed CALYX_FSV_ROOT_INVALID "root must be under /home/croyse/calyx/fsv"; exit 2 ;;
esac
if [ ! -x "$bin_path" ]; then
  write_readback 74 precondition_failed CALYX_FSV_PINNED_BINARY_MISSING "$bin_path"
  exit 74
fi
if [ ! -f "$vault/partitioned-manifest.json" ]; then
  write_readback 75 precondition_failed CALYX_FSV_PARTITIONED_MANIFEST_MISSING "$vault/partitioned-manifest.json"
  exit 75
fi

sha256sum "$bin_path" "$vault/partitioned-manifest.json" > "$out_dir/pre_search_hashes.txt"
"$bin_path" bench partitioned-search \
  --vault "$vault" \
  --n "$n" \
  --k "$k" \
  --n-probe "$n_probe" \
  --region-beam "$region_beam" \
  --anneal-vault "$out_dir/anneal_partitioned_search" \
  --tuner-slo-us "$slo_us" \
  > "$out_dir/search_stdout.json" 2> "$out_dir/search_stderr.log"
search_code=$?

if [ "$search_code" -eq 0 ]; then
  write_readback 0 complete "" ""
else
  write_readback "$search_code" search_failed CALYX_FSV_PINNED_SEARCH_FAILED "partitioned-search exited $search_code"
fi
exit "$search_code"
