#!/usr/bin/env bash
set -euo pipefail

OUT_DIR="${1:-/zfs/archive/calyx/datasets/drift_pair}"
URL="${CALYX_AG_NEWS_URL:-https://raw.githubusercontent.com/mhjabreel/CharCnn_Keras/master/data/ag_news_csv/test.csv}"
RAW="$OUT_DIR/ag_news_test.csv"
TMP="$RAW.tmp"

mkdir -p "$OUT_DIR"

if [[ ! -s "$RAW" ]]; then
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL --retry 3 "$URL" -o "$TMP"
  else
    python3 - "$URL" "$TMP" <<'PY'
import sys
import urllib.request
urllib.request.urlretrieve(sys.argv[1], sys.argv[2])
PY
  fi
  mv "$TMP" "$RAW"
fi

python3 - "$RAW" "$OUT_DIR" "$URL" <<'PY'
import csv
import hashlib
import json
import pathlib
import re
import sys

raw = pathlib.Path(sys.argv[1])
out = pathlib.Path(sys.argv[2])
url = sys.argv[3]

N_PER_SPLIT = 64
CLASS_NAME = {"1": "world", "2": "sports", "3": "business", "4": "sci_tech"}
KEYWORDS = {
    "world": {
        "war", "iraq", "china", "government", "president", "minister", "election",
        "security", "military", "peace", "israel", "russia", "officials",
    },
    "sports": {
        "game", "team", "season", "win", "coach", "league", "cup", "match",
        "players", "baseball", "football", "olympic", "champion",
    },
    "business": {
        "market", "stocks", "shares", "company", "profit", "oil", "economy",
        "trade", "bank", "prices", "sales", "investor", "business",
    },
    "scitech": {
        "software", "internet", "computer", "technology", "science", "nasa",
        "microsoft", "chip", "web", "phone", "space", "research", "digital",
    },
}

def sha256_file(path):
    h = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()

def tokenize(text):
    return re.findall(r"[a-z0-9]+", text.lower())

def features(text):
    toks = tokenize(text)
    token_count = max(len(toks), 1)
    mean_len = sum(len(tok) for tok in toks) / token_count
    counts = []
    token_set = toks
    for name in ("world", "sports", "business", "scitech"):
        counts.append(sum(1 for tok in token_set if tok in KEYWORDS[name]))
    return [
        token_count / 100.0,
        mean_len / 10.0,
        counts[0] / 10.0,
        counts[1] / 10.0,
        counts[2] / 10.0,
        counts[3] / 10.0,
    ]

def read_rows():
    buckets = {key: [] for key in CLASS_NAME}
    with raw.open("r", newline="", encoding="utf-8") as fh:
        for idx, row in enumerate(csv.reader(fh)):
            if len(row) < 3:
                continue
            label, title, desc = row[0], row[1], row[2]
            if label in buckets:
                text = f"{title} {desc}"
                buckets[label].append({
                    "source_id": idx,
                    "class": CLASS_NAME[label],
                    "title": title,
                    "text": text,
                    "features": features(text),
                    "text_sha256": hashlib.sha256(text.encode("utf-8")).hexdigest(),
                })
    return buckets

def take_class(rows, label, offset):
    start = offset
    end = offset + N_PER_SPLIT
    if len(rows[label]) < end:
        raise SystemExit(f"CALYX_DATASET_ROWCOUNT_MISMATCH {label} has {len(rows[label])}, need {end}")
    return rows[label][start:end]

def write_tsv(path, rows):
    with path.open("w", encoding="utf-8", newline="") as fh:
        fh.write("id\tsource_class\ttoken_count\tmean_token_len\tworld_hits\tsports_hits\tbusiness_hits\tscitech_hits\ttext_sha256\n")
        for idx, row in enumerate(rows):
            nums = "\t".join(f"{value:.8f}" for value in row["features"])
            fh.write(f"{idx}\t{row['class']}\t{nums}\t{row['text_sha256']}\n")

def write_source(path, split, rows):
    with path.open("w", encoding="utf-8") as fh:
        for row in rows:
            fh.write(json.dumps({
                "split": split,
                "source_id": row["source_id"],
                "source_class": row["class"],
                "title": row["title"],
                "text_sha256": row["text_sha256"],
            }, sort_keys=True) + "\n")

rows = read_rows()
month_a = take_class(rows, "1", 0)
month_a_control = take_class(rows, "1", N_PER_SPLIT)
month_b = take_class(rows, "4", 0)

write_tsv(out / "month_a.tsv", month_a)
write_tsv(out / "month_a_control.tsv", month_a_control)
write_tsv(out / "month_b.tsv", month_b)
write_source(out / "source_rows.jsonl", "month_a", month_a)
write_source(out / "source_rows.jsonl.tmp", "month_a_control", month_a_control)
with (out / "source_rows.jsonl").open("a", encoding="utf-8") as dest, (out / "source_rows.jsonl.tmp").open("r", encoding="utf-8") as src:
    dest.write(src.read())
(out / "source_rows.jsonl.tmp").unlink()
with (out / "source_rows.jsonl").open("a", encoding="utf-8") as dest:
    for row in month_b:
        dest.write(json.dumps({
            "split": "month_b",
            "source_id": row["source_id"],
            "source_class": row["class"],
            "title": row["title"],
            "text_sha256": row["text_sha256"],
        }, sort_keys=True) + "\n")

manifest = {
    "dataset": "drift_pair",
    "source": url,
    "raw_sha256": sha256_file(raw),
    "split_criteria": {
        "month_a": "AG News test rows, class world, first 64 rows",
        "month_a_control": "AG News test rows, class world, next 64 rows",
        "month_b": "AG News test rows, class sci_tech, first 64 rows",
    },
    "feature_columns": [
        "token_count", "mean_token_len", "world_hits", "sports_hits",
        "business_hits", "scitech_hits",
    ],
    "row_counts": {
        "month_a": len(month_a),
        "month_a_control": len(month_a_control),
        "month_b": len(month_b),
    },
}
(out / "manifest.json").write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8")
(out / "README.txt").write_text(
    "PH70 issue #609 drift pair. Month labels are deterministic split labels over real AG News text rows; features are text-derived lexical counts, not class one-hot labels.\n",
    encoding="utf-8",
)
print(json.dumps(manifest, sort_keys=True))
PY
