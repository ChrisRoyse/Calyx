#!/usr/bin/env bash
# PH69 T08 / issue #558 - the DATA BUILD_DONE coverage gate (PRD 28 sections
# 3 + 7): assert the canonical dataset catalog covers every required
# (modality x grounded-outcome-type) cell with at least one registered
# dataset, so every lens family and intelligence metric has a real grounded
# test. Rows only enter MANIFEST.md via verify_dataset.sh register (which
# recomputes from bytes and self-verifies), so row presence == verified.
#
#   check_manifest_coverage.sh              gate the real catalog
#   check_manifest_coverage.sh --self-test  hermetic synthetic-MANIFEST battery
#
# Fail-closed: any uncovered cell -> CALYX_DATASET_COVERAGE_MISSING on
# stderr, one line per missing cell, exit 1.
set -euo pipefail

SCRIPT_PATH="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")"
DATASET_ROOT="${CALYX_DATASET_ROOT:-/zfs/archive/calyx/datasets}"

fail() {
  echo "$1: $2" >&2
  exit 1
}

# cell|candidate datasets (PRD 28 section 3 rows 1-12; a cell is covered when
# ANY candidate has a MANIFEST row).
CELLS=(
  "text-semantic/qrels|beir msmarco natural_questions trec_covid"
  "text/class-label|ag_news imdb sst2 banking77 dbpedia_14"
  "code/test-pass-fail|swebench_lite humaneval mbpp"
  "graph/community|cora wordnet conceptnet ogbn"
  "text/duplicate-label|quora_qp paws"
  "audio-speaker/identity|voxceleb1 voxceleb2 librispeech"
  "audio/emotion-label|ravdess iemocap"
  "image/class-caption|cifar100 coco imagenet_subset"
  "temporal/recurrence|temporal_logs"
  "adversarial-text/injection-benign|prompt_injection"
  "civic/tie-formation|synthetic_personas"
  "text/distribution-shift|drift_pair"
)

gate() {
  local manifest="$DATASET_ROOT/MANIFEST.md"
  if [[ ! -f "$manifest" ]]; then
    fail CALYX_DATASET_MANIFEST_INVALID "catalog missing: $manifest"
  fi
  # Registered dataset names: first cell of each table body row.
  local names
  names=" $(awk -F'|' '/^\|/ {gsub(/ /, "", $2); if ($2 != "name" && $2 !~ /^-+$/) print $2}' "$manifest" | tr '\n' ' ') "
  local entry cell candidates candidate hit missing=0 covered
  for entry in "${CELLS[@]}"; do
    IFS='|' read -r cell candidates <<<"$entry"
    hit=""
    for candidate in $candidates; do
      if [[ "$names" == *" $candidate "* ]]; then
        hit="$candidate"
        break
      fi
    done
    if [[ -n "$hit" ]]; then
      printf '  [covered] %-36s <- %s\n' "$cell" "$hit"
    else
      echo "CALYX_DATASET_COVERAGE_MISSING: $cell (need one of: $candidates)" >&2
      missing=$((missing + 1))
    fi
  done
  if (( missing > 0 )); then
    echo "coverage gate: $missing of ${#CELLS[@]} required cells UNCOVERED" >&2
    exit 1
  fi
  echo "coverage gate: OK (${#CELLS[@]}/${#CELLS[@]} modality x outcome cells covered)"
}

# --- self-test: hermetic synthetic MANIFESTs ---------------------------------
self_test() {
  local tmp_root
  tmp_root="$(mktemp -d)"
  trap "rm -rf '$tmp_root'" EXIT
  local pass=0
  step() { pass=$((pass + 1)); echo "[SELF-TEST $pass] $1"; }

  row() { printf '| %s | src | rev | sha | 1 | 1 | lic | tests |\n' "$1"; }
  header() {
    printf '# synthetic catalog\n\n'
    printf '| name | source | revision | sha256 | rows | bytes | license | tests |\n'
    printf '|---|---|---|---|---|---|---|---|\n'
  }
  expect_gate() {
    # expect_gate <root> <want_exit> <desc> [required-stderr-grep]
    local root="$1" want="$2" desc="$3" want_err="${4:-}"
    local got=0
    CALYX_DATASET_ROOT="$root" bash "$SCRIPT_PATH" \
      >"$tmp_root/out.log" 2>"$tmp_root/err.log" || got=$?
    if [[ "$got" != "$want" ]]; then
      echo "SELF-TEST FAILED: $desc - exit $got != expected $want" >&2
      cat "$tmp_root/out.log" "$tmp_root/err.log" >&2
      exit 1
    fi
    if [[ -n "$want_err" ]] && ! grep -qF "$want_err" "$tmp_root/err.log"; then
      echo "SELF-TEST FAILED: $desc - stderr missing $want_err" >&2
      cat "$tmp_root/err.log" >&2
      exit 1
    fi
    echo "    exit $got as expected"
  }

  step "full synthetic MANIFEST (one row per cell) -> exit 0"
  mkdir -p "$tmp_root/full"
  { header
    row beir; row ag_news; row swebench_lite; row cora; row quora_qp
    row voxceleb1; row ravdess; row cifar100; row temporal_logs
    row prompt_injection; row synthetic_personas; row drift_pair
  } > "$tmp_root/full/MANIFEST.md"
  expect_gate "$tmp_root/full" 0 "full coverage"

  step "property: ANY candidate covers its cell (alternate candidates) -> exit 0"
  mkdir -p "$tmp_root/alt"
  { header
    row trec_covid; row dbpedia_14; row mbpp; row ogbn; row paws
    row librispeech; row iemocap; row coco; row temporal_logs
    row prompt_injection; row synthetic_personas; row drift_pair
  } > "$tmp_root/alt/MANIFEST.md"
  expect_gate "$tmp_root/alt" 0 "alternate candidates"

  step "edge 1: remove the audio-speaker/identity row -> exit 1 naming the cell"
  mkdir -p "$tmp_root/noident"
  grep -vE '^\| (voxceleb1|voxceleb2|librispeech) ' "$tmp_root/full/MANIFEST.md" \
    > "$tmp_root/noident/MANIFEST.md"
  expect_gate "$tmp_root/noident" 1 "missing identity cell" \
    "CALYX_DATASET_COVERAGE_MISSING: audio-speaker/identity"
  if grep -vq 'audio-speaker/identity' <(grep CALYX_DATASET_COVERAGE_MISSING "$tmp_root/err.log"); then
    echo "SELF-TEST FAILED: unexpected extra missing cells" >&2
    cat "$tmp_root/err.log" >&2
    exit 1
  fi

  step "edge 2: empty MANIFEST (header only) -> exit 1 listing all 12 cells"
  mkdir -p "$tmp_root/empty"
  header > "$tmp_root/empty/MANIFEST.md"
  expect_gate "$tmp_root/empty" 1 "empty catalog" "12 of 12 required cells UNCOVERED"
  local listed
  listed="$(grep -c '^CALYX_DATASET_COVERAGE_MISSING:' "$tmp_root/err.log")"
  [[ "$listed" == "12" ]] \
    || { echo "SELF-TEST FAILED: $listed missing-cell lines != 12" >&2; exit 1; }

  step "edge 3: MANIFEST file absent -> CALYX_DATASET_MANIFEST_INVALID"
  mkdir -p "$tmp_root/nodir"
  expect_gate "$tmp_root/nodir" 1 "absent catalog" "CALYX_DATASET_MANIFEST_INVALID"

  step "edge 4: name must match the whole cell (voxceleb1_mini does not cover identity)"
  mkdir -p "$tmp_root/prefix"
  { grep -vE '^\| (voxceleb1|voxceleb2|librispeech) ' "$tmp_root/full/MANIFEST.md"
    row voxceleb1_mini_issue608
  } > "$tmp_root/prefix/MANIFEST.md"
  expect_gate "$tmp_root/prefix" 1 "prefix name must not satisfy cell" \
    "CALYX_DATASET_COVERAGE_MISSING: audio-speaker/identity"

  echo "[SELF-TEST] all $pass steps passed"
}

case "${1:-gate}" in
  gate) gate ;;
  --self-test) self_test ;;
  *) fail CALYX_DATASET_MANIFEST_INVALID "unknown mode ${1:-}" ;;
esac
