#!/usr/bin/env bash
set -euo pipefail

SOAK_INCOMPLETE=CALYX_FSV_ANNEAL_SOAK_INCOMPLETE
J_NOT_GROWING=CALYX_FSV_ANNEAL_J_NOT_GROWING
P99_REGRESSION=CALYX_FSV_ANNEAL_P99_REGRESSION
RECALL_REGRESSION=CALYX_FSV_ANNEAL_RECALL_REGRESSION
GOODHART_FAILED=CALYX_FSV_ANNEAL_GOODHART_FAILED

METRICS_DIR=${CALYX_METRICS_DIR:-/zfs/hot/calyx/metrics}
DATASET_ROOT=${CALYX_DATASET_ROOT:-/zfs/archive/calyx/datasets}
VAULT=${CALYX_PH70_ANNEAL_VAULT:-/zfs/hot/calyx/vaults/calyx_ph70_validation}
QUERIES=${CALYX_ANNEAL_QUERIES:-1000000}
SAMPLE_INTERVAL=${CALYX_ANNEAL_SAMPLE_INTERVAL:-10000}
DOC_LIMIT=${CALYX_PH70_DOC_LIMIT:-50000}
MIN_DOCS=${CALYX_PH70_MIN_DOCS:-50000}
EXPECTED_SAMPLES=${CALYX_EXPECTED_SAMPLES:-100}

usage() {
  cat <<'EOF'
usage: scripts/validate_anneal_j.sh [--self-test]

Runs PH70 T06 Anneal J validation on aiwonder:
  1. exports >=50K AG News docs from the verified PH69 parquet dataset
  2. ingests them into an Aster vault through calyx anneal soak
  3. reads metric bytes from /zfs/hot/calyx/metrics
  4. fails closed on J, p99, recall, Goodhart, or missing-soak evidence
EOF
}

python_bin() {
  local candidate="$DATASET_ROOT/.dataset_tools_venv/bin/python"
  if [[ -x "$candidate" ]]; then
    printf '%s\n' "$candidate"
  else
    printf '%s\n' python3
  fi
}

safe_reset_vault() {
  if [[ "${CALYX_PH70_RESET_VAULT:-1}" != "1" ]]; then
    return
  fi
  case "$VAULT" in
    /zfs/hot/calyx/vaults/*|/tmp/calyx-ph70-*)
      rm -rf -- "$VAULT"
      ;;
    *)
      echo "refusing to reset unsafe vault path: $VAULT" >&2
      exit 2
      ;;
  esac
}

reset_metric_outputs() {
  rm -f -- \
    "$METRICS_DIR/anneal_j_series.jsonl" \
    "$METRICS_DIR/anneal_j_summary.json" \
    "$METRICS_DIR/anneal_p99_delta.txt" \
    "$METRICS_DIR/anneal_goodhart.txt" \
    "$METRICS_DIR/anneal_j_grafana.png"
}

prepare_corpus() {
  local py corpus manifest train
  py=$(python_bin)
  corpus="$METRICS_DIR/ag_news_ph70_50k.jsonl"
  manifest="$DATASET_ROOT/ag_news/manifest.json"
  train="$DATASET_ROOT/ag_news/train.parquet"
  mkdir -p "$METRICS_DIR"
  "$py" - "$manifest" "$train" "$corpus" "$DOC_LIMIT" <<'PY'
import json, pathlib, sys

manifest_path = pathlib.Path(sys.argv[1])
train_path = pathlib.Path(sys.argv[2])
out_path = pathlib.Path(sys.argv[3])
limit = int(sys.argv[4])

if not manifest_path.exists():
    raise SystemExit(f"missing AG News manifest: {manifest_path}")
if not train_path.exists():
    raise SystemExit(f"missing AG News train parquet: {train_path}")

try:
    import pyarrow.parquet as pq
except Exception as exc:
    raise SystemExit(f"pyarrow required to export AG News parquet: {exc}")

manifest = json.loads(manifest_path.read_text())
table = pq.read_table(train_path, columns=["text", "label"])
rows = min(limit, table.num_rows)
if rows < 50000:
    raise SystemExit(f"AG News export rows {rows} below PH70 minimum 50000")
texts = table.column("text")
labels = table.column("label")
with out_path.open("w", encoding="utf-8") as handle:
    for idx in range(rows):
        text = texts[idx].as_py()
        label = labels[idx].as_py()
        handle.write(json.dumps({
            "row": idx,
            "label": str(label),
            "text": text,
            "source": f"ag_news://train.parquet#row={idx}",
        }, ensure_ascii=True) + "\n")
profile = {
    "source": "PH69 ag_news train.parquet",
    "manifest": manifest,
    "export_rows": rows,
    "corpus_jsonl": str(out_path),
}
out_path.with_name("anneal_corpus_profile.json").write_text(
    json.dumps(profile, indent=2, sort_keys=True) + "\n",
    encoding="utf-8",
)
print(out_path)
PY
}

write_reflex_request() {
  local target="$METRICS_DIR/anneal_reflex_request.json"
  cat >"$target" <<EOF
{
  "synapse_tool": "reflex_register",
  "condition": "metric file contains soak_status=complete",
  "source_of_truth": "$METRICS_DIR/anneal_j_series.jsonl",
  "note": "Agent registers this request through Synapse MCP before launching the soak."
}
EOF
}

run_soak() {
  local corpus
  corpus="$METRICS_DIR/ag_news_ph70_50k.jsonl"
  cargo run -p calyx-cli -- anneal soak \
    --queries "$QUERIES" \
    --vault "$VAULT" \
    --corpus-jsonl "$corpus" \
    --metrics-dir "$METRICS_DIR" \
    --sample-interval "$SAMPLE_INTERVAL" \
    --min-docs "$MIN_DOCS"
}

validate_metric_dir() {
  local py
  py=$(python_bin)
  CALYX_EXPECTED_SAMPLES="$EXPECTED_SAMPLES" "$py" - "$METRICS_DIR" <<'PY'
import json, os, pathlib, struct, sys, zlib

SOAK_INCOMPLETE = "CALYX_FSV_ANNEAL_SOAK_INCOMPLETE"
J_NOT_GROWING = "CALYX_FSV_ANNEAL_J_NOT_GROWING"
P99_REGRESSION = "CALYX_FSV_ANNEAL_P99_REGRESSION"
RECALL_REGRESSION = "CALYX_FSV_ANNEAL_RECALL_REGRESSION"
GOODHART_FAILED = "CALYX_FSV_ANNEAL_GOODHART_FAILED"

root = pathlib.Path(sys.argv[1])
series_path = root / "anneal_j_series.jsonl"
goodhart_path = root / "anneal_goodhart.txt"
summary_path = root / "anneal_j_summary.json"
p99_path = root / "anneal_p99_delta.txt"
png_path = root / "anneal_j_grafana.png"
expected = int(os.environ.get("CALYX_EXPECTED_SAMPLES", "100"))

def fail(code, detail):
    print(f"{code}: {detail}", file=sys.stderr)
    raise SystemExit(1)

if not series_path.exists():
    fail(SOAK_INCOMPLETE, f"missing {series_path}")
lines = [line for line in series_path.read_text().splitlines() if line.strip()]
if len(lines) != expected:
    fail(SOAK_INCOMPLETE, f"expected {expected} samples, found {len(lines)}")
rows = [json.loads(line) for line in lines]
first, last = rows[0], rows[-1]
if last.get("soak_status") != "complete":
    fail(SOAK_INCOMPLETE, "last sample did not mark soak_status=complete")
if float(last["j"]) <= float(first["j"]):
    fail(J_NOT_GROWING, f"first={first['j']} last={last['j']}")
if float(last["p99"]) > float(first["p99"]) * 0.80:
    fail(P99_REGRESSION, f"first={first['p99']} last={last['p99']}")
if float(last["recall"]) + 1e-12 < float(first["recall"]):
    fail(RECALL_REGRESSION, f"first={first['recall']} last={last['recall']}")
if not goodhart_path.exists():
    fail(GOODHART_FAILED, f"missing {goodhart_path}")
goodhart = json.loads(goodhart_path.read_text())
if not goodhart.get("goodhart_pass"):
    fail(GOODHART_FAILED, json.dumps(goodhart, sort_keys=True))

summary = {
    "source_of_truth": str(root),
    "sample_count": len(rows),
    "j_first": first["j"],
    "j_last": last["j"],
    "j_growing": last["j"] > first["j"],
    "p99_first": first["p99"],
    "p99_last": last["p99"],
    "p99_pass": last["p99"] <= first["p99"] * 0.80,
    "recall_first": first["recall"],
    "recall_last": last["recall"],
    "recall_pass": last["recall"] >= first["recall"],
    "goodhart_pass": True,
}
summary_path.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n")
p99_path.write_text(
    "\n".join([
        f"p99_first_ns={first['p99']}",
        f"p99_last_ns={last['p99']}",
        f"p99_required_max_ns={float(first['p99']) * 0.80:.0f}",
        f"p99_pass={str(summary['p99_pass']).lower()}",
        "",
    ])
)

def png_chunk(tag, data):
    return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data) & 0xffffffff)

def write_png(path, values, width=960, height=540):
    pixels = bytearray([250, 250, 250] * width * height)
    def set_px(x, y, rgb):
        if 0 <= x < width and 0 <= y < height:
            offset = (y * width + x) * 3
            pixels[offset:offset+3] = bytes(rgb)
    for x in range(72, width - 48):
        set_px(x, height - 72, (50, 50, 50))
    for y in range(48, height - 72):
        set_px(72, y, (50, 50, 50))
    lo, hi = min(values), max(values)
    span = hi - lo or 1.0
    pts = []
    for idx, value in enumerate(values):
        x = 72 + int(idx * (width - 140) / max(1, len(values) - 1))
        y = height - 72 - int((value - lo) * (height - 140) / span)
        pts.append((x, y))
    for (x0, y0), (x1, y1) in zip(pts, pts[1:]):
        steps = max(abs(x1 - x0), abs(y1 - y0), 1)
        for step in range(steps + 1):
            x = x0 + (x1 - x0) * step // steps
            y = y0 + (y1 - y0) * step // steps
            for dx in (-1, 0, 1):
                for dy in (-1, 0, 1):
                    set_px(x + dx, y + dy, (31, 119, 180))
    raw = b"".join(b"\x00" + pixels[y * width * 3:(y + 1) * width * 3] for y in range(height))
    png = b"\x89PNG\r\n\x1a\n"
    png += png_chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
    png += png_chunk(b"IDAT", zlib.compress(raw, 9))
    png += png_chunk(b"IEND", b"")
    path.write_bytes(png)

write_png(png_path, [float(row["j"]) for row in rows])
print(json.dumps(summary, sort_keys=True))
PY
}

self_test() {
  local tmp
  tmp=$(mktemp -d)
  METRICS_DIR="$tmp/pass" EXPECTED_SAMPLES=10 make_case pass
  METRICS_DIR="$tmp/pass" EXPECTED_SAMPLES=10 validate_metric_dir
  echo "edge_ok happy_path"
  METRICS_DIR="$tmp/j" EXPECTED_SAMPLES=10 make_case j_flat
  expect_fail "$J_NOT_GROWING" "$tmp/j" 10
  METRICS_DIR="$tmp/p99" EXPECTED_SAMPLES=10 make_case p99_bad
  expect_fail "$P99_REGRESSION" "$tmp/p99" 10
  METRICS_DIR="$tmp/recall" EXPECTED_SAMPLES=10 make_case recall_bad
  expect_fail "$RECALL_REGRESSION" "$tmp/recall" 10
  METRICS_DIR="$tmp/goodhart" EXPECTED_SAMPLES=10 make_case goodhart_bad
  expect_fail "$GOODHART_FAILED" "$tmp/goodhart" 10
  mkdir -p "$tmp/missing"
  expect_fail "$SOAK_INCOMPLETE" "$tmp/missing" 10
  rm -rf "$tmp"
}

make_case() {
  local mode=${1:?}
  mkdir -p "$METRICS_DIR"
  python3 - "$METRICS_DIR" "$mode" <<'PY'
import json, pathlib, sys
root = pathlib.Path(sys.argv[1])
mode = sys.argv[2]
rows = []
for i in range(10):
    frac = (i + 1) / 10
    j = 0.50 + 0.30 * frac
    p99 = 100 - 30 * frac
    recall = 0.95 + 0.005 * frac
    if mode == "j_flat":
        j = 0.5
    if mode == "p99_bad":
        p99 = 95
    if mode == "recall_bad" and i == 9:
        recall = 0.949
    rows.append({
        "step": i + 1,
        "query_count": (i + 1) * 100,
        "j": j,
        "delta_j": 0 if i == 0 else 0.03,
        "p99": p99,
        "recall": recall,
        "soak_status": "complete" if i == 9 else "running",
    })
(root / "anneal_j_series.jsonl").write_text("".join(json.dumps(row) + "\n" for row in rows))
(root / "anneal_goodhart.txt").write_text(json.dumps({"goodhart_pass": mode != "goodhart_bad"}))
PY
}

expect_fail() {
  local code=${1:?} dir=${2:?} samples=${3:?} output rc
  set +e
  output=$(METRICS_DIR="$dir" EXPECTED_SAMPLES="$samples" validate_metric_dir 2>&1)
  rc=$?
  set -e
  if [[ "$rc" -eq 0 || "$output" != *"$code"* ]]; then
    echo "expected $code failure, got rc=$rc output=$output" >&2
    exit 1
  fi
  echo "edge_ok $code"
}

main() {
  case "${1:-}" in
    -h|--help)
      usage
      ;;
    --self-test)
      self_test
      ;;
    "")
      mkdir -p "$METRICS_DIR"
      safe_reset_vault
      reset_metric_outputs
      write_reflex_request
      prepare_corpus
      run_soak
      validate_metric_dir
      ;;
    *)
      usage >&2
      exit 2
      ;;
  esac
}

main "$@"
