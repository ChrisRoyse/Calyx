#!/usr/bin/env bash
# PH70 T05 FSV: prove a FORM-ONLY panel (text-embedding lenses on a SWE-bench
# problem's surface text) is INSUFFICIENT to predict the binary oracle
# test_pass_fail (did a real model's patch resolve the instance) -> I(panel;oracle)
# < H(Y) -> the sufficiency-refusal gate fires. Builds the panel corpus from a real
# SWE-bench experiments run (grounded pass/fail outcomes) + TEI embeddings, then
# runs `calyx oracle sufficiency-validate` (real calyx-assay MI estimators).
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GPU_PY="${CALYX_GPU_PYTHON:-/home/croyse/calyx/.venv-gpu/bin/python}"
STAMP="${CALYX_FSV_STAMP:-$(date -u +%Y%m%dT%H%M%SZ)}"
EVIDENCE_ROOT="${CALYX_FSV_ROOT:-/home/croyse/calyx/data/fsv-issue563-oracle-$STAMP}"
METRICS_DIR="${CALYX_ORACLE_METRICS_DIR:-/zfs/hot/calyx/metrics}"
CORPUS_DIR="$EVIDENCE_ROOT/corpus"
CF_ROOT="$EVIDENCE_ROOT/oracle_cf"
ORACLE_MODEL="${CALYX_ORACLE_MODEL:-20240402_sweagent_gpt4}"
export HF_HOME="${HF_HOME:-/home/croyse/.cache/huggingface}"

if [[ ! -x "$GPU_PY" ]]; then
  echo "CALYX_ORACLE_GPU_TOOLCHAIN_MISSING: $GPU_PY not executable" >&2
  exit 2
fi
mkdir -p "$CORPUS_DIR" "$METRICS_DIR"

# Step 1: build the form-only panel corpus from a real SWE-bench experiments run.
"$GPU_PY" "$REPO_ROOT/scripts/build_oracle_corpus.py" "$ORACLE_MODEL" "$CORPUS_DIR" | tee "$EVIDENCE_ROOT/corpus_build.log"

# Step 2: oracle sufficiency validation (real calyx-assay MI; gate per honesty_gate).
cd "$REPO_ROOT"
cargo run -p calyx-cli -- oracle sufficiency-validate \
  --corpus-dir "$CORPUS_DIR" \
  --metrics-dir "$METRICS_DIR" \
  --cf-root "$CF_ROOT" \
  --domain swebench_lite | tee "$EVIDENCE_ROOT/oracle_sufficiency_validate.log"

# Step 3: independent readback of the source-of-truth metric files.
echo "=== READBACK ==="
echo "oracle_i_panel.txt=$(cat "$METRICS_DIR/oracle_i_panel.txt")"
echo "oracle_entropy.txt=$(cat "$METRICS_DIR/oracle_entropy.txt")"
echo "oracle_deficit.txt=$(cat "$METRICS_DIR/oracle_deficit.txt")"
echo "oracle_refused.txt=$(cat "$METRICS_DIR/oracle_refused.txt")"
"$GPU_PY" -c "i=float(open('$METRICS_DIR/oracle_i_panel.txt').read()); h=float(open('$METRICS_DIR/oracle_entropy.txt').read()); print('PASS' if i<h else 'FAIL', 'I=%.4f < H=%.4f deficit=%.4f refusal_fires=%s'%(i,h,h-i,i<h))"

printf 'ORACLE_FSV_ROOT=%s\n' "$EVIDENCE_ROOT"
printf 'ORACLE_METRICS_DIR=%s\n' "$METRICS_DIR"
printf 'ORACLE_CF_ROOT=%s\n' "$CF_ROOT"
