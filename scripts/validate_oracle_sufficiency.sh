#!/usr/bin/env bash
# PH70 T05 FSV: prove a FORM-ONLY panel (text-embedding lenses on a SWE-bench
# problem's surface text) is INSUFFICIENT to predict the binary oracle
# test_pass_fail (did a real model's patch resolve the instance) -> I(panel;oracle)
# < H(Y) -> the sufficiency-refusal gate fires. Builds the panel corpus from a real
# SWE-bench experiments run (grounded pass/fail outcomes) + TEI embeddings, then
# runs `calyx oracle sufficiency-validate` (real calyx-assay MI estimators).
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ -f "$REPO_ROOT/env.sh" ]]; then
  # aiwonder runtime contract: ORT_DYLIB_PATH, CUDA libs, and runpath flags.
  # shellcheck source=/dev/null
  . "$REPO_ROOT/env.sh"
fi
GPU_PY="${CALYX_GPU_PYTHON:-/home/croyse/calyx/.venv-gpu/bin/python}"
STAMP="${CALYX_FSV_STAMP:-$(date -u +%Y%m%dT%H%M%SZ)}"
EVIDENCE_ROOT="${CALYX_FSV_ROOT:-/home/croyse/calyx/data/fsv-issue563-oracle-$STAMP}"
METRICS_DIR="${CALYX_ORACLE_METRICS_DIR:-/zfs/hot/calyx/metrics}"
SOURCE_DIR="$EVIDENCE_ROOT/source_rows"
CORPUS_DIR="$EVIDENCE_ROOT/corpus"
CF_ROOT="$EVIDENCE_ROOT/oracle_cf"
FSV_BOUNDED="$REPO_ROOT/scripts/fsv_bounded.py"
ORACLE_MODEL="${CALYX_ORACLE_MODEL:-20240402_sweagent_gpt4}"
A38_READBACK="${CALYX_A38_GATE_READBACK:-/home/croyse/calyx/fsv/issue832-bgem3-colbert-roster-20260621T141356Z/final_readback.json}"
REGISTRY_JSON="${CALYX_LENS_REGISTRY:-/home/croyse/calyx/lenses/registry.json}"
A38_COST_JSON="${CALYX_A38_COST_JSON:-/home/croyse/calyx/fsv/issue832-bgem3-colbert-roster-20260621T141356Z/code_language_corpus/cost.json}"
BATCH_SIZE="${CALYX_ORACLE_BATCH_SIZE:-16}"
export HF_HOME="${HF_HOME:-/home/croyse/.cache/huggingface}"

if [[ ! -x "$GPU_PY" ]]; then
  echo "CALYX_ORACLE_GPU_TOOLCHAIN_MISSING: $GPU_PY not executable" >&2
  exit 2
fi
if [[ ! -f "$A38_COST_JSON" ]]; then
  echo "CALYX_ORACLE_A38_COST_JSON_NOT_FOUND: $A38_COST_JSON" >&2
  exit 2
fi
mkdir -p "$SOURCE_DIR" "$METRICS_DIR"

# Step 1: select the accepted >=10-lens A38 roster from live registry readback.
MANIFEST_LIST="$EVIDENCE_ROOT/oracle_manifests.txt"
"$GPU_PY" - "$REGISTRY_JSON" "$A38_READBACK" "$MANIFEST_LIST" <<'PY'
import json
import pathlib
import sys

registry_path = pathlib.Path(sys.argv[1])
readback_path = pathlib.Path(sys.argv[2])
out_path = pathlib.Path(sys.argv[3])

registry = json.loads(registry_path.read_text())
if isinstance(registry, dict) and isinstance(registry.get("lenses"), list):
    rows = registry["lenses"]
elif isinstance(registry, dict) and isinstance(registry.get("rows"), list):
    rows = registry["rows"]
elif isinstance(registry, list):
    rows = registry
else:
    rows = []
    if isinstance(registry, dict):
        for value in registry.values():
            if isinstance(value, list):
                rows.extend(item for item in value if isinstance(item, dict))

by_name = {}
for row in rows:
    name = row.get("name") or row.get("id") or row.get("lens") or row.get("lens_name")
    if name:
        by_name[name] = row

readback = json.loads(readback_path.read_text())
names = [item.get("lens") or item.get("name") for item in readback.get("lens_bests", [])]
names = [name for name in names if name]
if len(names) < 10:
    raise SystemExit(f"CALYX_ORACLE_A38_ROSTER_TOO_SMALL: lens_bests={len(names)}")

manifests = []
for name in names:
    row = by_name.get(name)
    if not row:
        raise SystemExit(f"CALYX_ORACLE_A38_LENS_NOT_IN_REGISTRY: {name}")
    manifest = row.get("manifest") or row.get("manifest_path")
    if not manifest:
        raise SystemExit(f"CALYX_ORACLE_A38_MANIFEST_MISSING: {name}")
    path = pathlib.Path(manifest)
    if not path.is_file():
        raise SystemExit(f"CALYX_ORACLE_A38_MANIFEST_NOT_FOUND: {path}")
    manifests.append(str(path))

out_path.write_text("\n".join(manifests) + "\n")
print(f"selected_lens_count={len(manifests)}")
print("selected_lenses=" + ",".join(names))
PY
mapfile -t MANIFESTS < "$MANIFEST_LIST"
MANIFEST_ARGS=()
for manifest in "${MANIFESTS[@]}"; do
  MANIFEST_ARGS+=(--manifest "$manifest")
done

# Step 2: build grounded SWE-bench oracle source rows from a real experiments run.
"$GPU_PY" "$FSV_BOUNDED" capture \
  --stdout "$EVIDENCE_ROOT/source_rows_build.log" \
  --stderr "$EVIDENCE_ROOT/source_rows_build.stderr" \
  -- "$GPU_PY" "$REPO_ROOT/scripts/build_oracle_corpus.py" "$ORACLE_MODEL" "$SOURCE_DIR"
ROWS_JSONL="$SOURCE_DIR/rows.jsonl"

# Step 3: measure the rows through the accepted A38 lenses and persist vectors.
cd "$REPO_ROOT"
"$GPU_PY" "$FSV_BOUNDED" capture \
  --stdout "$EVIDENCE_ROOT/assay_corpus_build.log" \
  --stderr "$EVIDENCE_ROOT/assay_corpus_build.stderr" \
  -- cargo run -p calyx-cli -- assay corpus-build \
  --rows-jsonl "$ROWS_JSONL" \
  --out-dir "$CORPUS_DIR" \
  --dataset "swebench_lite_oracle_${ORACLE_MODEL}" \
  --target-class 1 \
  --batch-size "$BATCH_SIZE" \
  --cost-override-json "$A38_COST_JSON" \
  --embedding-model-id "a38:issue832-accepted-10" \
  "${MANIFEST_ARGS[@]}"

# Step 4: oracle sufficiency validation (real calyx-assay MI; gate per honesty_gate).
"$GPU_PY" "$FSV_BOUNDED" capture \
  --stdout "$EVIDENCE_ROOT/oracle_sufficiency_validate.log" \
  --stderr "$EVIDENCE_ROOT/oracle_sufficiency_validate.stderr" \
  -- cargo run -p calyx-cli -- oracle sufficiency-validate \
  --corpus-dir "$CORPUS_DIR" \
  --metrics-dir "$METRICS_DIR" \
  --cf-root "$CF_ROOT" \
  --domain swebench_lite

# Step 5: independent readback of the source-of-truth metric files.
echo "=== READBACK ==="
"$GPU_PY" - "$METRICS_DIR" <<'PY'
import json
import pathlib
import sys

metrics = pathlib.Path(sys.argv[1])
report = json.loads((metrics / "oracle_sufficiency.json").read_text())
print(f"lens_count={len(report['lenses'])}")
print(f"i_panel_oracle={report['i_panel_oracle']:.6f}")
print(f"h_y={report['h_y']:.6f}")
print(f"deficit={report['deficit']:.6f}")
print(f"refused={report['refused']} sufficient={report['sufficient']}")
print(f"rows_persisted={report['rows_persisted']} rows_readback={report['rows_readback']}")
print("PASS" if report["i_panel_oracle"] < report["h_y"] and report["refused"] else "FAIL")
PY

printf 'ORACLE_FSV_ROOT=%s\n' "$EVIDENCE_ROOT"
printf 'ORACLE_SOURCE_DIR=%s\n' "$SOURCE_DIR"
printf 'ORACLE_METRICS_DIR=%s\n' "$METRICS_DIR"
printf 'ORACLE_CF_ROOT=%s\n' "$CF_ROOT"
