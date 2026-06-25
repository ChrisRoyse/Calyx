#!/usr/bin/env bash
# PH70 T02 FSV: build a real AG News multi-lens corpus via TEI and prove, with
# the real calyx-assay estimators, that each real lens carries bits about a
# grounded anchor, a planted-redundant lens is rejected, the panel MI is
# reported with a CI, and per-stratum bits are present.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DATASET_ROOT="${CALYX_DATASET_ROOT:-/home/croyse/calyx/datasets/datasets}"
PYTHON="${CALYX_DATASET_PYTHON:-/zfs/archive/calyx/datasets/.dataset_tools_venv/bin/python3}"
STAMP="${CALYX_FSV_STAMP:-$(date -u +%Y%m%dT%H%M%SZ)}"
EVIDENCE_ROOT="${CALYX_FSV_ROOT:-/home/croyse/calyx/data/fsv-issue560-assay-$STAMP}"
CORPUS_DIR="$EVIDENCE_ROOT/corpus"
METRICS_DIR="${CALYX_ASSAY_METRICS_DIR:-/zfs/hot/calyx/metrics}"
CF_ROOT="$EVIDENCE_ROOT/assay_cf"
SOURCE_READBACK="$EVIDENCE_ROOT/assay_source_readback.json"
FSV_BOUNDED="$REPO_ROOT/scripts/fsv_bounded.py"
TEI="${CALYX_TEI_ENDPOINT:-http://127.0.0.1:8088/embed}"
N="${CALYX_ASSAY_N:-600}"
TARGET_CLASS="${CALYX_ASSAY_TARGET_CLASS:-2}" # AG News class 2 = Business

if [[ ! -x "$PYTHON" ]]; then
  echo "CALYX_DATASET_TOOLCHAIN_MISSING: $PYTHON not executable" >&2
  exit 2
fi

mkdir -p "$EVIDENCE_ROOT" "$CORPUS_DIR" "$METRICS_DIR"

"$PYTHON" - "$DATASET_ROOT" "$CORPUS_DIR" "$SOURCE_READBACK" "$TEI" "$N" "$TARGET_CLASS" <<'PY'
import hashlib, json, math, sys, urllib.request
from pathlib import Path

import pyarrow.parquet as pq

root = Path(sys.argv[1])
out_dir = Path(sys.argv[2])
readback_path = Path(sys.argv[3])
tei = sys.argv[4]
n_target = int(sys.argv[5])
target_class = int(sys.argv[6])

SEED = 42
BATCH = 32
TOKEN_DIM = 256
# The planted-redundant lens is an EXACT duplicate of the gte lens (cosine = 1.0,
# the canonical maximally-redundant case). Because the input is byte-identical,
# the deterministic logistic-probe estimator yields identical bits, so the
# engine's stable greedy admission keeps gte_cls (corpus index 0) and rejects the
# duplicate (index 1) for redundancy (corr = 1.0 > 0.6). This avoids relying on a
# fragile bits tie-break between two near-equally-informative lenses.

parquet_path = root / "ag_news" / "train.parquet"
if not parquet_path.is_file():
    print(f"CALYX_FSV_ASSAY_CORPUS_NOT_FOUND: {parquet_path}", file=sys.stderr)
    sys.exit(2)

def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()

# Deterministic shuffled sample of N rows (seed 42).
table = pq.read_table(parquet_path, columns=["text", "label"])
texts = table.column("text").to_pylist()
labels = table.column("label").to_pylist()
order = list(range(len(texts)))

def key(i):
    return hashlib.blake2b(f"{SEED}:{i}".encode(), digest_size=8).digest()

order.sort(key=key)
order = order[:n_target]
texts = [texts[i] for i in order]
labels = [int(labels[i]) for i in order]

label_counts = {}
for lab in labels:
    label_counts[str(lab)] = label_counts.get(str(lab), 0) + 1
positives = label_counts.get(str(target_class), 0)
if positives < 80:
    # Pick the most-represented class instead.
    target_class = int(max(label_counts.items(), key=lambda kv: kv[1])[0])
    positives = label_counts[str(target_class)]
print(f"target_class={target_class} positives={positives} counts={label_counts}", file=sys.stderr)

def embed(batch):
    payload = json.dumps({"inputs": batch, "normalize": True}).encode()
    req = urllib.request.Request(tei, data=payload, headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=120) as resp:
        return json.loads(resp.read())

embeddings = []
for start in range(0, len(texts), BATCH):
    chunk = texts[start:start + BATCH]
    vecs = embed(chunk)
    embeddings.extend(vecs)
if len(embeddings) != len(texts):
    print("CALYX_FSV_ASSAY_INVALID_CORPUS: embedding count mismatch", file=sys.stderr)
    sys.exit(2)

def l2(vec):
    norm = math.sqrt(sum(v * v for v in vec))
    return [v / norm for v in vec] if norm > 0 else vec

def token_hash(text):
    vec = [0.0] * TOKEN_DIM
    for tok in text.replace("\n", " ").split():
        tok = tok.lower()
        if len(tok) < 2:
            continue
        idx = int.from_bytes(hashlib.blake2b(tok.encode(), digest_size=2).digest(), "big") % TOKEN_DIM
        vec[idx] += 1.0
    return l2(vec)

rows = []
for i, (text, lab, emb) in enumerate(zip(texts, labels, embeddings)):
    redundant = list(emb)  # exact duplicate -> corr 1.0, identical bits
    rows.append({
        "id": f"agnews-{order[i]}",
        "split": "train",
        "label": lab,
        "lenses": {
            "gte_cls": [round(v, 6) for v in emb],
            "gte_redundant": [round(v, 6) for v in redundant],
            "token_hash": [round(v, 6) for v in token_hash(text)],
        },
    })

with (out_dir / "vectors.jsonl").open("w", encoding="utf-8") as handle:
    for row in rows:
        handle.write(json.dumps(row, separators=(",", ":")) + "\n")

manifest = {
    "dataset": "ag_news",
    "embedding_model_id": "tei:gte",
    "n_samples": len(rows),
    "label_counts": label_counts,
    "lenses": [
        {"name": "gte_cls", "redundant": False},
        {"name": "gte_redundant", "redundant": True},
        {"name": "token_hash", "redundant": False},
    ],
    "target_class": target_class,
}
(out_dir / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")

readback = {
    "dataset": "ag_news",
    "source_path": str(parquet_path),
    "source_sha256": sha256(parquet_path),
    "n_samples": len(rows),
    "target_class": target_class,
    "label_counts": label_counts,
    "tei_endpoint": tei,
    "embedding_dim": len(embeddings[0]),
}
readback_path.write_text(json.dumps(readback, indent=2) + "\n", encoding="utf-8")
# Emit the resolved target class for the shell to consume.
(out_dir / ".target_class").write_text(str(target_class), encoding="utf-8")
PY

TARGET_CLASS="$(cat "$CORPUS_DIR/.target_class")"
"$PYTHON" "$FSV_BOUNDED" summarize "$SOURCE_READBACK" \
  --field dataset=.dataset \
  --field n_samples=.n_samples \
  --field target_class=.target_class \
  --field embedding_dim=.embedding_dim

cd "$REPO_ROOT"
"$PYTHON" "$FSV_BOUNDED" capture \
  --stdout "$EVIDENCE_ROOT/assay_bits_validate.log" \
  --stderr "$EVIDENCE_ROOT/assay_bits_validate.stderr" \
  -- cargo run -p calyx-cli -- assay bits-validate \
  --corpus-dir "$CORPUS_DIR" \
  --metrics-dir "$METRICS_DIR" \
  --cf-root "$CF_ROOT" \
  --target-class "$TARGET_CLASS" \
  --domain "ag_news"

printf 'ASSAY_FSV_ROOT=%s\n' "$EVIDENCE_ROOT"
printf 'ASSAY_CORPUS_DIR=%s\n' "$CORPUS_DIR"
printf 'ASSAY_METRICS_DIR=%s\n' "$METRICS_DIR"
printf 'ASSAY_CF_ROOT=%s\n' "$CF_ROOT"
printf 'ASSAY_SOURCE_READBACK=%s\n' "$SOURCE_READBACK"
printf 'ASSAY_TARGET_CLASS=%s\n' "$TARGET_CLASS"
