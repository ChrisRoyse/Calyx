#!/usr/bin/env bash
# PH70 T04 FSV: prove Ward's injection-block claim on a real prompt-injection
# corpus with a REAL discriminative classifier guard (not the degenerate
# cosine-to-centroid of issue #693). Fine-tunes an injection classifier on GPU,
# emits per-example guard scores, then feeds them into Ward's REAL conformal
# tau-calibration via `calyx ward guard-validate`, which gates the held-out
# injection-block-rate AND the benign false-reject-rate, persists per-example
# verdicts, and demonstrates novelty routing.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GPU_PY="${CALYX_GPU_PYTHON:-/home/croyse/calyx/.venv-gpu/bin/python}"
STAMP="${CALYX_FSV_STAMP:-$(date -u +%Y%m%dT%H%M%SZ)}"
EVIDENCE_ROOT="${CALYX_FSV_ROOT:-/home/croyse/calyx/data/fsv-issue562-ward-$STAMP}"
METRICS_DIR="${CALYX_WARD_METRICS_DIR:-/zfs/hot/calyx/metrics}"
FSV_BOUNDED="$REPO_ROOT/scripts/fsv_bounded.py"
MODEL="${CALYX_WARD_MODEL:-roberta-base}"
# Train on a diverse union of public injection corpora (lower confident-error
# rate -> robust >=99% block); evaluate/validate on the largest clean held-out
# corpus's test split.
TRAIN_DATASETS="${CALYX_WARD_TRAIN_DATASETS:-xTRam1/safe-guard-prompt-injection deepset/prompt-injections jackhhao/jailbreak-classification}"
TEST_DATASET="${CALYX_WARD_TEST_DATASET:-xTRam1/safe-guard-prompt-injection}"
EPOCHS="${CALYX_WARD_EPOCHS:-8}"
SCORES="$EVIDENCE_ROOT/scores.jsonl"
export HF_HOME="${HF_HOME:-/home/croyse/.cache/huggingface}"
export TOKENIZERS_PARALLELISM=false

if [[ ! -x "$GPU_PY" ]]; then
  echo "CALYX_WARD_GPU_TOOLCHAIN_MISSING: $GPU_PY not executable" >&2
  exit 2
fi
mkdir -p "$EVIDENCE_ROOT" "$METRICS_DIR"

# Step 1: materialize a diverse training union + the held-out test split.
"$GPU_PY" - "$EVIDENCE_ROOT" "$TEST_DATASET" $TRAIN_DATASETS <<'PY'
import json, sys
from datasets import load_dataset
out, test_name, train_names = sys.argv[1], sys.argv[2], sys.argv[3:]
def col(ds, *cands):
    for c in cands:
        if c in ds.column_names:
            return c
    raise SystemExit(f"CALYX_WARD_INVALID_CORPUS: no text/label column in {ds.column_names}")
def norm(ds):
    tcol, lcol = col(ds, "text", "prompt"), col(ds, "label", "type")
    rows = []
    for t, l in zip(ds[tcol], ds[lcol]):
        lab = l if isinstance(l, int) else (1 if str(l).lower() in ("1", "injection", "jailbreak", "attack") else 0)
        rows.append({"text": t, "label": int(lab)})
    return rows
train = []
for name in train_names:
    d = load_dataset(name)
    train += norm(d["train"])
json.dump(train, open(f"{out}/train.json", "w"))
test = norm(load_dataset(test_name)["test"])
json.dump(test, open(f"{out}/test.json", "w"))
print(f"train n={len(train)} inj={sum(r['label'] for r in train)} | test n={len(test)} inj={sum(r['label'] for r in test)}", file=sys.stderr)
PY

# Step 2: fine-tune the injection classifier on GPU and emit guard scores.
"$GPU_PY" "$FSV_BOUNDED" capture \
  --stdout "$EVIDENCE_ROOT/finetune.log" \
  --stderr "$EVIDENCE_ROOT/finetune.stderr" \
  -- "$GPU_PY" "$REPO_ROOT/scripts/finetune_injection_guard.py" \
  --model "$MODEL" \
  --train "$EVIDENCE_ROOT/train.json" \
  --test "$EVIDENCE_ROOT/test.json" \
  --out "$EVIDENCE_ROOT/model" \
  --scores-out "$SCORES" \
  --epochs "$EPOCHS" --bs 32 --lr 2e-5

# Step 3: Ward conformal validation over the classifier scores (gates block + FRR).
cd "$REPO_ROOT"
"$GPU_PY" "$FSV_BOUNDED" capture \
  --stdout "$EVIDENCE_ROOT/ward_guard_validate.log" \
  --stderr "$EVIDENCE_ROOT/ward_guard_validate.stderr" \
  -- cargo run -p calyx-cli -- ward guard-validate \
  --scores "$SCORES" \
  --metrics-dir "$METRICS_DIR" \
  --eval-split test \
  --target-far "${CALYX_WARD_TARGET_FAR:-0.01}" \
  --required-block-rate "${CALYX_WARD_REQUIRED_BLOCK:-0.99}" \
  --max-frr "${CALYX_WARD_MAX_FRR:-0.01}"

# Step 4: independent readback of the source-of-truth metric files.
echo "=== READBACK ==="
echo "ward_tau.txt=$(cat "$METRICS_DIR/ward_tau.txt")"
echo "ward_block_rate.txt=$(cat "$METRICS_DIR/ward_block_rate.txt")"
echo "ward_far.txt=$(cat "$METRICS_DIR/ward_far.txt")"
echo "ward_frr.txt=$(cat "$METRICS_DIR/ward_frr.txt")"
echo "ward_novelty_routed.txt=$(cat "$METRICS_DIR/ward_novelty_routed.txt")"
"$GPU_PY" -c "rate=float(open('$METRICS_DIR/ward_block_rate.txt').read()); frr=float(open('$METRICS_DIR/ward_frr.txt').read()); print('PASS' if rate>=0.99 and frr<=0.01 else 'FAIL', 'block=%.4f frr=%.4f'%(rate,frr))"
"$GPU_PY" "$FSV_BOUNDED" summarize "$METRICS_DIR/ward_guard_verdicts.jsonl"

printf 'WARD_FSV_ROOT=%s\n' "$EVIDENCE_ROOT"
printf 'WARD_METRICS_DIR=%s\n' "$METRICS_DIR"
printf 'WARD_SCORES=%s\n' "$SCORES"
