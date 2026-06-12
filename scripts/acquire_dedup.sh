#!/usr/bin/env bash
# PH69 T06 / issue #605 - acquire QQP + PAWS dedup corpora, checksum-verified,
# and emit the deterministic FSV pair subset used by dedup_qqp_paws_fsv.rs.
# Fail-closed: any mismatch exits 1 with an exact CALYX_* code on stderr.
set -euo pipefail

DATASET_ROOT="${CALYX_DATASET_ROOT:-/zfs/archive/calyx/datasets}"
QQP_DIR="$DATASET_ROOT/quora_qp"
PAWS_DIR="$DATASET_ROOT/paws"
VENV_DIR="$DATASET_ROOT/.dataset_tools_venv"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

QQP_URL="https://qim.fs.quoracdn.net/quora_duplicate_questions.tsv"
QQP_EXPECTED_BYTES=58176133
QQP_EXPECTED_ROWS=404290

PAWS_REVISION="161ece9501cf0a11f3e48bd356eaa82de46d6a09"
PAWS_BASE="https://huggingface.co/datasets/google-research-datasets/paws/resolve/$PAWS_REVISION/labeled_final"
PAWS_TRAIN_ROWS=49401
PAWS_DEV_ROWS=8000
PAWS_TEST_ROWS=8000
# 24.0.0 is the first pin verified to ship a cp314 wheel for aiwonder's Python 3.14.
PYARROW_PIN="${CALYX_PYARROW_PIN:-pyarrow==24.0.0}"

fail() {
  echo "$1: $2" >&2
  exit 1
}

download() {
  local url="$1" dest="$2"
  if [[ -s "$dest" ]]; then
    return 0
  fi
  curl -fsSL --retry 3 --retry-delay 5 "$url" -o "$dest.tmp" \
    || fail CALYX_DATASET_DOWNLOAD_FAILED "$url"
  mv "$dest.tmp" "$dest"
}

mkdir -p "$QQP_DIR" "$PAWS_DIR"

# --- QQP raw TSV ---
QQP_RAW="$QQP_DIR/quora_duplicate_questions.tsv"
download "$QQP_URL" "$QQP_RAW"
actual_bytes=$(stat -c%s "$QQP_RAW")
if [[ "$actual_bytes" != "$QQP_EXPECTED_BYTES" ]]; then
  fail CALYX_DATASET_BYTES_MISMATCH "quora_qp bytes $actual_bytes != expected $QQP_EXPECTED_BYTES"
fi

# --- PAWS labeled_final parquet (pinned revision) ---
for split in train validation test; do
  download "$PAWS_BASE/$split-00000-of-00001.parquet" "$PAWS_DIR/$split.parquet"
done

# --- venv with pinned pyarrow for parquet -> tsv ---
if [[ ! -x "$VENV_DIR/bin/python3" ]]; then
  python3 -m venv "$VENV_DIR" || fail CALYX_DATASET_VENV_FAILED "python3 -m venv $VENV_DIR"
fi
if ! "$VENV_DIR/bin/python3" -c 'import pyarrow' 2>/dev/null; then
  "$VENV_DIR/bin/pip" install --quiet "$PYARROW_PIN" \
    || fail CALYX_DATASET_VENV_FAILED "pip install $PYARROW_PIN"
fi

"$VENV_DIR/bin/python3" - "$QQP_DIR" "$PAWS_DIR" "$DATASET_ROOT" <<'PY'
import csv
import hashlib
import json
import pathlib
import sys

import pyarrow.parquet as pq

qqp_dir = pathlib.Path(sys.argv[1])
paws_dir = pathlib.Path(sys.argv[2])
root = pathlib.Path(sys.argv[3])

QQP_EXPECTED_ROWS = 404290
PAWS_EXPECTED = {"train": 49401, "validation": 8000, "test": 8000}
QQP_PER_BUCKET = 256
PAWS_PER_LABEL = 200
MAX_TEXT_CHARS = 1000

def fail(code, message):
    print(f"{code}: {message}", file=sys.stderr)
    raise SystemExit(1)

def sha256_file(path):
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()

def sanitize(text):
    return " ".join(text.split())

def text_sha(text):
    return hashlib.sha256(text.encode("utf-8")).hexdigest()

# --- QQP parse + label partition check ---
qqp_raw = qqp_dir / "quora_duplicate_questions.tsv"
qqp_rows = []
with qqp_raw.open("r", encoding="utf-8", newline="") as handle:
    reader = csv.DictReader(handle, delimiter="\t", quoting=csv.QUOTE_MINIMAL)
    for row in reader:
        label = row.get("is_duplicate")
        q1 = row.get("question1") or ""
        q2 = row.get("question2") or ""
        if label not in ("0", "1"):
            fail("CALYX_DATASET_LABEL_INVALID", f"qqp row {len(qqp_rows)} label {label!r}")
        qqp_rows.append((row["id"], sanitize(q1), sanitize(q2), int(label)))
if len(qqp_rows) != QQP_EXPECTED_ROWS:
    fail("CALYX_DATASET_ROWCOUNT_MISMATCH", f"qqp rows {len(qqp_rows)} != {QQP_EXPECTED_ROWS}")
qqp_dup = sum(1 for r in qqp_rows if r[3] == 1)
if qqp_dup == 0 or qqp_dup == len(qqp_rows):
    fail("CALYX_DATASET_LABEL_PARTITION_MISSING", f"qqp dup_count {qqp_dup}")

# --- PAWS parquet -> tsv ---
paws_meta = {}
for split, expected in PAWS_EXPECTED.items():
    table = pq.read_table(paws_dir / f"{split}.parquet")
    rows = table.to_pylist()
    if len(rows) != expected:
        fail("CALYX_DATASET_ROWCOUNT_MISMATCH", f"paws {split} rows {len(rows)} != {expected}")
    dup = sum(1 for r in rows if int(r["label"]) == 1)
    if dup == 0 or dup == len(rows):
        fail("CALYX_DATASET_LABEL_PARTITION_MISSING", f"paws {split} dup_count {dup}")
    out = paws_dir / f"{split}.tsv"
    with out.open("w", encoding="utf-8", newline="") as handle:
        handle.write("id\tsentence1\tsentence2\tlabel\n")
        for r in rows:
            s1, s2 = sanitize(r["sentence1"]), sanitize(r["sentence2"])
            handle.write(f"{r['id']}\t{s1}\t{s2}\t{int(r['label'])}\n")
    paws_meta[split] = {"rows": len(rows), "dup_count": dup, "tsv_sha256": sha256_file(out)}

# --- deterministic FSV pair subset (file order, first-N per bucket) ---
def qqp_buckets():
    buckets = {("calib", 1): [], ("calib", 0): [], ("eval", 1): [], ("eval", 0): []}
    for pair_id, q1, q2, label in qqp_rows:
        if not q1 or not q2 or len(q1) > MAX_TEXT_CHARS or len(q2) > MAX_TEXT_CHARS:
            continue
        for split in ("calib", "eval"):
            bucket = buckets[(split, label)]
            if len(bucket) < QQP_PER_BUCKET:
                bucket.append((split, pair_id, q1, q2, label))
                break
    for key, bucket in buckets.items():
        if len(bucket) != QQP_PER_BUCKET:
            fail("CALYX_DATASET_SUBSET_SHORT", f"qqp bucket {key} has {len(bucket)}")
    return buckets

def paws_bucket():
    rows = []
    counts = {0: 0, 1: 0}
    test_tsv = paws_dir / "test.tsv"
    with test_tsv.open("r", encoding="utf-8") as handle:
        next(handle)
        for line in handle:
            pair_id, s1, s2, label = line.rstrip("\n").split("\t")
            label = int(label)
            if counts[label] >= PAWS_PER_LABEL or len(s1) > MAX_TEXT_CHARS or len(s2) > MAX_TEXT_CHARS:
                continue
            counts[label] += 1
            rows.append(("paws", pair_id, s1, s2, label))
    if counts[0] != PAWS_PER_LABEL or counts[1] != PAWS_PER_LABEL:
        fail("CALYX_DATASET_SUBSET_SHORT", f"paws counts {counts}")
    return rows

fsv_path = root / "dedup_fsv_pairs.tsv"
with fsv_path.open("w", encoding="utf-8", newline="") as handle:
    handle.write("source\tsplit\tpair_id\tlabel\ttext_a_sha256\ttext_b_sha256\ttext_a\ttext_b\n")
    for (split, _), bucket in sorted(qqp_buckets().items(), key=lambda kv: (kv[0][0], -kv[0][1])):
        for _, pair_id, q1, q2, label in bucket:
            handle.write(
                f"qqp\t{split}\t{pair_id}\t{label}\t{text_sha(q1)}\t{text_sha(q2)}\t{q1}\t{q2}\n"
            )
    for source, pair_id, s1, s2, label in paws_bucket():
        handle.write(
            f"{source}\tadversarial\t{pair_id}\t{label}\t{text_sha(s1)}\t{text_sha(s2)}\t{s1}\t{s2}\n"
        )

# --- manifests ---
qqp_manifest = {
    "dataset": "quora_qp",
    "source": "https://qim.fs.quoracdn.net/quora_duplicate_questions.tsv",
    "raw_sha256": sha256_file(qqp_raw),
    "raw_bytes": qqp_raw.stat().st_size,
    "rows": len(qqp_rows),
    "dup_count": qqp_dup,
    "license": "Quora custom / non-commercial research",
    "tests": "TCT cosine-Gtau dedup correctness (PH70 issue #605)",
}
paws_manifest = {
    "dataset": "paws",
    "source": "huggingface:google-research-datasets/paws labeled_final",
    "revision": "161ece9501cf0a11f3e48bd356eaa82de46d6a09",
    "parquet_sha256": {
        split: sha256_file(paws_dir / f"{split}.parquet")
        for split in ("train", "validation", "test")
    },
    "splits": paws_meta,
    "license": "Provided 'AS IS' by Google (PAWS release); free for any purpose",
    "tests": "conflicting-anchor never-merge on adversarial high-overlap pairs (PH70 issue #605)",
}
fsv_sha = sha256_file(fsv_path)
summary = {
    "fsv_pairs": str(fsv_path),
    "fsv_pairs_sha256": fsv_sha,
    "qqp": qqp_manifest,
    "paws": paws_manifest,
}
(root / "dedup_fsv_pairs.manifest.json").write_text(
    json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8"
)
print(json.dumps(summary, sort_keys=True))
PY

# --- canonical registration: manifest.json + MANIFEST.md row + verify -------
# verify_dataset.sh register is the single writer of catalog rows (PH69 T01);
# it recomputes per-file sha256/bytes/rows from the bytes on disk and then
# byte-verifies its own output. PAWS counts rows from the pinned parquet
# splits only - the derived *.tsv files hold the same records.
export CALYX_DATASET_PYTHON="$VENV_DIR/bin/python3"
bash "$SCRIPT_DIR/verify_dataset.sh" register quora_qp \
  --source "$QQP_URL" \
  --revision "2017-03-06" \
  --license "Quora custom / non-commercial research" \
  --tests "TCT cosine-Gtau dedup correctness (PH70 issue #605)"
bash "$SCRIPT_DIR/verify_dataset.sh" register paws \
  --source "huggingface:google-research-datasets/paws labeled_final" \
  --revision "$PAWS_REVISION" \
  --license "Provided 'AS IS' by Google (PAWS release); free for any purpose" \
  --tests "conflicting-anchor never-merge on adversarial high-overlap pairs (PH70 issue #605)" \
  --rows-from "*.parquet"

echo "acquire_dedup: OK"
