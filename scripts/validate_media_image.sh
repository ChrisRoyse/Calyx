#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DATASET_ROOT="${CALYX_DATASET_ROOT:-/zfs/archive/calyx/datasets}"
PYTHON="${CALYX_DATASET_PYTHON:-$DATASET_ROOT/.dataset_tools_venv/bin/python3}"
STAMP="${CALYX_FSV_STAMP:-$(date -u +%Y%m%dT%H%M%SZ)}"
EVIDENCE_ROOT="${CALYX_FSV_ROOT:-/home/croyse/calyx/data/fsv-issue607-media-image-$STAMP}"
SAMPLES="$EVIDENCE_ROOT/media_image_samples.jsonl"
SOURCE_READBACK="$EVIDENCE_ROOT/media_image_source_readback.json"
FSV_BOUNDED="$REPO_ROOT/scripts/fsv_bounded.py"
METRICS_DIR="$EVIDENCE_ROOT/metrics"
VAULT_DIR="$EVIDENCE_ROOT/vault"

if [[ ! -x "$PYTHON" ]]; then
  echo "CALYX_DATASET_TOOLCHAIN_MISSING: $PYTHON not executable" >&2
  exit 2
fi

mkdir -p "$EVIDENCE_ROOT" "$METRICS_DIR"

"$PYTHON" - <<'PY'
import importlib.util, subprocess, sys
missing = [name for name in ("pyarrow", "PIL") if importlib.util.find_spec(name) is None]
if "pyarrow" in missing:
    raise SystemExit("CALYX_DATASET_TOOLCHAIN_MISSING: pyarrow missing from dataset venv")
if "PIL" in missing:
    subprocess.check_call([sys.executable, "-m", "pip", "install", "--disable-pip-version-check", "Pillow"])
PY

"$PYTHON" - "$DATASET_ROOT" "$SAMPLES" "$SOURCE_READBACK" <<'PY'
import hashlib, io, json, math, os, re, sys, zipfile
from collections import defaultdict
from pathlib import Path

import pyarrow.parquet as pq
from PIL import Image

root = Path(sys.argv[1])
sample_path = Path(sys.argv[2])
readback_path = Path(sys.argv[3])

CIFAR_PER_COARSE = int(os.environ.get("CALYX_MEDIA_CIFAR_PER_COARSE", "5"))
IMAGENET_LIMIT = int(os.environ.get("CALYX_MEDIA_IMAGENET_LIMIT", "80"))
COCO_LIMIT = int(os.environ.get("CALYX_MEDIA_COCO_LIMIT", "120"))

def image_features(data: bytes) -> list[float]:
    image = Image.open(io.BytesIO(data)).convert("RGB").resize((16, 16))
    raw = image.tobytes()
    pixels = [(raw[i], raw[i + 1], raw[i + 2]) for i in range(0, len(raw), 3)]
    feats = []
    for channel in range(3):
        vals = [pixel[channel] / 255.0 for pixel in pixels]
        mean = sum(vals) / len(vals)
        var = sum((value - mean) ** 2 for value in vals) / len(vals)
        feats.extend([mean, math.sqrt(var)])
    for gy in range(4):
        for gx in range(4):
            vals = []
            for y in range(gy * 4, (gy + 1) * 4):
                for x in range(gx * 4, (gx + 1) * 4):
                    r, g, b = pixels[y * 16 + x]
                    vals.append((r + g + b) / (3.0 * 255.0))
            feats.append(sum(vals) / len(vals))
    horiz = []
    vert = []
    for y in range(16):
        for x in range(15):
            a = sum(pixels[y * 16 + x]) / 3.0
            b = sum(pixels[y * 16 + x + 1]) / 3.0
            horiz.append(abs(a - b) / 255.0)
    for y in range(15):
        for x in range(16):
            a = sum(pixels[y * 16 + x]) / 3.0
            b = sum(pixels[(y + 1) * 16 + x]) / 3.0
            vert.append(abs(a - b) / 255.0)
    feats.extend([sum(horiz) / len(horiz), sum(vert) / len(vert)])
    return [round(value, 6) for value in feats]

def emit(out, row):
    out.write(json.dumps(row, sort_keys=True, separators=(",", ":")) + "\n")

def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()

def load_cifar(out):
    path = root / "cifar100" / "test.parquet"
    table = pq.ParquetFile(path).read()
    counts = defaultdict(int)
    rows = 0
    for idx, row in enumerate(table.to_pylist()):
        label = int(row["coarse_label"])
        if counts[label] >= CIFAR_PER_COARSE:
            continue
        data = row["img"]["bytes"]
        emit(out, {
            "sample_id": f"cifar100:test:{idx}",
            "dataset": "cifar100",
            "image_features": image_features(data),
            "class_label": label,
            "source_sha256": sha256(data),
        })
        counts[label] += 1
        rows += 1
        if len(counts) == 20 and all(value >= CIFAR_PER_COARSE for value in counts.values()):
            break
    return {"path": str(path), "rows": rows, "coarse_labels": len(counts)}

def load_imagenet(out):
    rows = 0
    labels = set()
    files = sorted((root / "imagenet_subset").glob("validation-*.parquet"))
    for file in files:
        for idx, row in enumerate(pq.ParquetFile(file).read().to_pylist()):
            data = row["image"]["bytes"]
            label = int(row["label"])
            emit(out, {
                "sample_id": f"imagenet_subset:{file.name}:{idx}",
                "dataset": "imagenet_subset",
                "image_features": image_features(data),
                "class_label": 1000 + label,
                "source_sha256": sha256(data),
            })
            labels.add(label)
            rows += 1
            if rows >= IMAGENET_LIMIT:
                return {"files_seen": files.index(file) + 1, "rows": rows, "labels": len(labels)}
    return {"files_seen": len(files), "rows": rows, "labels": len(labels)}

def norm_tokens(text: str) -> set[str]:
    return set(re.sub(r"[^a-z0-9]+", " ", text.lower()).split())

def coco_vectors(out):
    ann_zip = root / "coco" / "annotations_trainval2017.zip"
    img_zip = root / "coco" / "val2017.zip"
    with zipfile.ZipFile(ann_zip) as anns:
        instances = json.load(anns.open("annotations/instances_val2017.json"))
        captions = json.load(anns.open("annotations/captions_val2017.json"))
    categories = sorted(instances["categories"], key=lambda row: row["id"])
    cat_ids = [row["id"] for row in categories]
    cat_index = {cat_id: idx for idx, cat_id in enumerate(cat_ids)}
    cat_tokens = {row["id"]: norm_tokens(row["name"]) for row in categories}
    cats_by_image = defaultdict(set)
    for ann in instances["annotations"]:
        cats_by_image[int(ann["image_id"])].add(int(ann["category_id"]))
    captions_by_image = defaultdict(list)
    for ann in captions["annotations"]:
        captions_by_image[int(ann["image_id"])].append(ann["caption"])
    image_name = {int(row["id"]): row["file_name"] for row in instances["images"]}
    rows = 0
    with zipfile.ZipFile(img_zip) as images:
        for image_id in sorted(cats_by_image):
            image_cats = cats_by_image[image_id]
            for caption in captions_by_image.get(image_id, []):
                tokens = norm_tokens(caption)
                caption_cats = {
                    cat_id for cat_id, parts in cat_tokens.items()
                    if parts and parts.issubset(tokens)
                }
                overlap = sorted(image_cats & caption_cats)
                if not overlap:
                    continue
                focus = overlap[0]
                image_vec = [0.0] * len(cat_ids)
                caption_vec = [0.0] * len(cat_ids)
                for cat_id in image_cats:
                    image_vec[cat_index[cat_id]] = 0.2
                image_vec[cat_index[focus]] = 1.0
                for cat_id in caption_cats:
                    caption_vec[cat_index[cat_id]] = 0.2
                caption_vec[cat_index[focus]] = 1.0
                name = image_name[image_id]
                image_bytes = images.read(f"val2017/{name}")
                emit(out, {
                    "sample_id": f"coco:{image_id}:{rows}",
                    "dataset": "coco",
                    "image_features": image_vec,
                    "caption_features": caption_vec,
                    "source_sha256": sha256(image_bytes),
                })
                rows += 1
                if rows >= COCO_LIMIT:
                    return {
                        "annotation_zip": str(ann_zip),
                        "image_zip": str(img_zip),
                        "rows": rows,
                        "categories": len(cat_ids),
                    }
    return {"annotation_zip": str(ann_zip), "image_zip": str(img_zip), "rows": rows, "categories": len(cat_ids)}

sample_path.parent.mkdir(parents=True, exist_ok=True)
with sample_path.open("w", encoding="utf-8") as out:
    summary = {
        "cifar100": load_cifar(out),
        "imagenet_subset": load_imagenet(out),
        "coco": coco_vectors(out),
    }
summary["sample_jsonl"] = str(sample_path)
summary["sample_rows"] = sum(1 for _ in sample_path.open("r", encoding="utf-8"))
summary["sample_sha256"] = sha256(sample_path.read_bytes())
readback_path.write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY

"$PYTHON" "$FSV_BOUNDED" summarize "$SOURCE_READBACK" \
  --field sample_rows=.sample_rows \
  --field sample_sha256=.sample_sha256

cd "$REPO_ROOT"
"$PYTHON" "$FSV_BOUNDED" capture \
  --stdout "$EVIDENCE_ROOT/media_image_validate.log" \
  --stderr "$EVIDENCE_ROOT/media_image_validate.stderr" \
  -- cargo run -p calyx-cli -- media image-validate \
  --samples "$SAMPLES" \
  --metrics-dir "$METRICS_DIR" \
  --vault "$VAULT_DIR" \
  --min-image-bits "${CALYX_MEDIA_MIN_IMAGE_BITS:-0.05}" \
  --min-cross-modal-bits "${CALYX_MEDIA_MIN_CROSS_MODAL_BITS:-0.05}" \
  --k "${CALYX_MEDIA_K:-3}"

printf 'MEDIA_IMAGE_FSV_ROOT=%s\n' "$EVIDENCE_ROOT"
