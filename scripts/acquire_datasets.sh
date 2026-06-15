#!/usr/bin/env bash
# PH69 T01 / issue #551 - dataset acquisition orchestrator.
#
# Sources $CALYX_HOME/.env for HF_HUB_TOKEN, provisions the shared dataset
# tools venv (pinned pyarrow), runs every registered per-modality acquire
# script in sequence, then byte-verifies the whole catalog with
# verify_dataset.sh ALL. Fail-closed (A16): the first failure aborts with a
# non-zero exit; nothing is skipped silently.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CALYX_HOME="${CALYX_HOME:-/home/croyse/calyx}"
DATASET_ROOT="${CALYX_DATASET_ROOT:-/zfs/archive/calyx/datasets}"
VENV_DIR="$DATASET_ROOT/.dataset_tools_venv"
# 24.0.0 is the first pin verified to ship a cp314 wheel for aiwonder's Python 3.14.
PYARROW_PIN="${CALYX_PYARROW_PIN:-pyarrow==24.0.0}"

fail() {
  echo "$1: $2" >&2
  exit 1
}

# --- secrets: HF_HUB_TOKEN must exist (gated HF datasets need it) -----------
ENV_FILE="$CALYX_HOME/.env"
if [[ ! -f "$ENV_FILE" ]]; then
  fail CALYX_DATASET_ENV_MISSING "$ENV_FILE not found - provision CALYX_HOME (PH00)"
fi
set -a
# shellcheck disable=SC1090
source "$ENV_FILE"
set +a
if [[ -z "${HF_HUB_TOKEN:-${HF_TOKEN:-}}" ]]; then
  fail CALYX_DATASET_ENV_MISSING "HF_HUB_TOKEN/HF_TOKEN not set in $ENV_FILE"
fi
export HF_HUB_TOKEN="${HF_HUB_TOKEN:-$HF_TOKEN}"

# --- shared tools venv (pyarrow for parquet hashing/row counts) -------------
if [[ ! -d "$DATASET_ROOT" ]]; then
  fail CALYX_DATASET_NOT_FOUND "dataset root missing: $DATASET_ROOT (PH00 ZFS provisioning)"
fi
if [[ ! -x "$VENV_DIR/bin/python3" ]]; then
  python3 -m venv "$VENV_DIR" || fail CALYX_DATASET_VENV_FAILED "python3 -m venv $VENV_DIR"
fi
if ! "$VENV_DIR/bin/python3" -c 'import pyarrow' 2>/dev/null; then
  "$VENV_DIR/bin/pip" install --quiet "$PYARROW_PIN" \
    || fail CALYX_DATASET_VENV_FAILED "pip install $PYARROW_PIN"
fi
export CALYX_DATASET_PYTHON="$VENV_DIR/bin/python3"

# --- per-modality acquire scripts -------------------------------------------
# Registered scripts run in order; each is fail-closed itself. The pending list
# is printed explicitly so a partial catalog is never mistaken for a full one
# (no silent caps). Move a script from PENDING to REGISTERED as its card lands.
REGISTERED=(
  acquire_dedup.sh           # PH69 T06 - QQP / PAWS (#556, landed via #605)
  acquire_retrieval.sh       # PH69 T02 - BEIR scifact / MS MARCO / NQ / TREC-COVID (#552)
  acquire_classification.sh  # PH69 T03 - AG News / IMDB / SST-2 / banking77 / DBpedia-14 (#553)
  acquire_code_oracle.sh     # PH69 T04 - SWE-bench Lite / HumanEval / MBPP-sanitized (#554)
  acquire_graph_kernel.sh    # PH69 T05 - WordNet / ConceptNet / Cora / ogbn-arxiv (#555; wiktionary deferred loudly)
  acquire_audio.sh           # PH69 T07 - VoxCeleb1/2 / LibriSpeech / RAVDESS / IEMOCAP (#557)
  acquire_image.sh           # PH69 T07 - ImageNet-1k-val (gated, #683) / CIFAR-100 / COCO (#557)
  acquire_temporal_adversarial.sh  # PH69 T08 - NAB temporal / prompt-injection / personas + coverage gate (#558)
)
PENDING=()

for script in "${REGISTERED[@]}"; do
  path="$repo_root/scripts/$script"
  if [[ ! -f "$path" ]]; then
    fail CALYX_DATASET_NOT_FOUND "registered acquire script missing: $path"
  fi
  echo "=== acquire: $script ==="
  bash "$path"
done

echo "=== pending modality cards (not yet acquired - catalog is PARTIAL) ==="
for entry in "${PENDING[@]}"; do
  echo "  PENDING: $entry"
done

# --- the truth gate: byte-verify every catalog row + dir coverage -----------
echo "=== verify: ALL ==="
bash "$repo_root/scripts/verify_dataset.sh" ALL
echo "acquire_datasets: OK"
