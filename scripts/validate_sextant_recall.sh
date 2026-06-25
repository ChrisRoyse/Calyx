#!/usr/bin/env bash
set -euo pipefail

CALYX_HOME="${CALYX_HOME:-/home/croyse/calyx}"
REPO="${CALYX_REPO:-$CALYX_HOME/repo}"
FSV_BOUNDED="$REPO/scripts/fsv_bounded.py"
DATASET="${CALYX_QRELS_ROOT:-$CALYX_HOME/data/datasets/beir-scifact/scifact}"
CORPUS_JSONL="$DATASET/corpus.jsonl"
QUERIES_JSONL="$DATASET/queries.jsonl"
QRELS_TSV="$DATASET/qrels/test.tsv"
METRICS_DIR="${CALYX_METRICS_DIR:-/zfs/hot/calyx/metrics}"
VAULT="${CALYX_SEXTANT_VAULT:-/zfs/hot/calyx/vaults/ph70_sextant_recall}"
QUERY_LIMIT="${CALYX_SEXTANT_QUERY_LIMIT:-50}"
MIN_DELTA="${CALYX_SEXTANT_MIN_DELTA:-0.15}"
SELF_TEST=0

usage() {
  cat <<'USAGE'
usage: scripts/validate_sextant_recall.sh [--self-test]
       [--dataset <dir>] [--metrics-dir <dir>] [--vault <dir>]
       [--query-limit <n>] [--min-delta <float>]
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --self-test)
      SELF_TEST=1
      shift
      ;;
    --dataset)
      DATASET="$2"
      CORPUS_JSONL="$DATASET/corpus.jsonl"
      QUERIES_JSONL="$DATASET/queries.jsonl"
      QRELS_TSV="$DATASET/qrels/test.tsv"
      shift 2
      ;;
    --metrics-dir)
      METRICS_DIR="$2"
      shift 2
      ;;
    --vault)
      VAULT="$2"
      shift 2
      ;;
    --query-limit)
      QUERY_LIMIT="$2"
      shift 2
      ;;
    --min-delta)
      MIN_DELTA="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown arg: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

cd "$REPO"

if [[ "$SELF_TEST" == 1 ]]; then
  cargo test -p calyx-cli sextant_recall_validation -- --nocapture
  exit 0
fi

for path in "$CORPUS_JSONL" "$QUERIES_JSONL" "$QRELS_TSV"; do
  if [[ ! -f "$path" ]]; then
    echo "CALYX_FSV_SEXTANT_DATASET_MISSING: $path" >&2
    exit 1
  fi
done

safe_reset_vault() {
  case "$VAULT" in
    /zfs/hot/calyx/vaults/*|/tmp/calyx-*)
      rm -rf -- "$VAULT"
      ;;
    *)
      echo "refusing to reset vault outside Calyx hot/tmp roots: $VAULT" >&2
      exit 2
      ;;
  esac
}

mkdir -p "$METRICS_DIR"
echo "BEFORE_METRIC_STATE"
for file in \
  "$METRICS_DIR/sextant_single_recall.txt" \
  "$METRICS_DIR/sextant_multi_recall.txt" \
  "$METRICS_DIR/sextant_recall_delta.txt" \
  "$METRICS_DIR/sextant_recall_summary.json"; do
  if [[ -e "$file" ]]; then
    ls -l "$file"
  else
    echo "ABSENT $file"
  fi
done

rm -f -- \
  "$METRICS_DIR/sextant_single_recall.txt" \
  "$METRICS_DIR/sextant_multi_recall.txt" \
  "$METRICS_DIR/sextant_recall_delta.txt" \
  "$METRICS_DIR/sextant_recall_summary.json" \
  "$METRICS_DIR/sextant_online_cf_readback.txt"
safe_reset_vault

cargo build -p calyx-cli
BIN="$REPO/target/debug/calyx"

python3 "$FSV_BOUNDED" capture \
  --stdout "$METRICS_DIR/sextant_recall_command_output.json" \
  --stderr "$METRICS_DIR/sextant_recall_command_output.stderr" \
  -- "$BIN" sextant recall-validate \
  --corpus-jsonl "$CORPUS_JSONL" \
  --queries-jsonl "$QUERIES_JSONL" \
  --qrels "$QRELS_TSV" \
  --metrics-dir "$METRICS_DIR" \
  --vault "$VAULT" \
  --query-limit "$QUERY_LIMIT" \
  --min-delta "$MIN_DELTA"

python3 "$FSV_BOUNDED" capture \
  --stdout "$METRICS_DIR/sextant_online_cf_readback.txt" \
  --stderr "$METRICS_DIR/sextant_online_cf_readback.stderr" \
  -- "$BIN" readback --cf online --vault "$VAULT"

echo "AFTER_METRIC_BYTES"
cat "$METRICS_DIR/sextant_single_recall.txt"
cat "$METRICS_DIR/sextant_multi_recall.txt"
cat "$METRICS_DIR/sextant_recall_delta.txt"

python3 - "$METRICS_DIR" "$MIN_DELTA" <<'PY'
import json
import math
import pathlib
import sys

metrics = pathlib.Path(sys.argv[1])
min_delta = float(sys.argv[2])
single = float((metrics / "sextant_single_recall.txt").read_text().strip())
multi = float((metrics / "sextant_multi_recall.txt").read_text().strip())
delta = float((metrics / "sextant_recall_delta.txt").read_text().strip())
summary = json.loads((metrics / "sextant_recall_summary.json").read_text())
command = json.loads((metrics / "sextant_recall_command_output.json").read_text())
cf_text = (metrics / "sextant_online_cf_readback.txt").read_text()
key_hex = command["metric_cf_key_hex"]
if not all(math.isfinite(x) for x in (single, multi, delta)):
    raise SystemExit("CALYX_FSV_SEXTANT_NONFINITE_METRIC")
if abs((multi - single) - delta) > 1e-6:
    raise SystemExit(f"CALYX_FSV_SEXTANT_DELTA_MISMATCH: {multi-single} != {delta}")
if delta < min_delta:
    raise SystemExit(f"CALYX_FSV_SEXTANT_RECALL_BELOW_THRESHOLD: delta={delta:.6f}")
report = command["report"]
if not report["provenance_ok"] or report["multi_hits_examined"] <= 0:
    raise SystemExit("CALYX_FSV_LEDGER_REF_MISSING")
if key_hex not in cf_text:
    raise SystemExit("CALYX_FSV_SEXTANT_METRIC_CF_MISSING")
print(f"PASS sextant_recall single={single:.6f} multi={multi:.6f} delta={delta:.6f}")
print(f"PASS metric_cf_key_hex={key_hex}")
PY
