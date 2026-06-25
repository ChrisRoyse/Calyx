#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DATASET_ROOT="${CALYX_DATASET_ROOT:-/zfs/archive/calyx/datasets}"
PYTHON="${CALYX_DATASET_PYTHON:-$DATASET_ROOT/.dataset_tools_venv/bin/python3}"
STAMP="${CALYX_FSV_STAMP:-$(date -u +%Y%m%dT%H%M%SZ)}"
EVIDENCE_ROOT="${CALYX_FSV_ROOT:-/home/croyse/calyx/data/fsv-issue606-media-emotion-$STAMP}"
SAMPLES="$EVIDENCE_ROOT/media_emotion_samples.jsonl"
SOURCE_READBACK="$EVIDENCE_ROOT/media_emotion_source_readback.json"
FSV_BOUNDED="$REPO_ROOT/scripts/fsv_bounded.py"
METRICS_DIR="$EVIDENCE_ROOT/metrics"
VAULT_DIR="$EVIDENCE_ROOT/vault"

if [[ ! -x "$PYTHON" ]]; then
  echo "CALYX_DATASET_TOOLCHAIN_MISSING: $PYTHON not executable" >&2
  exit 2
fi

mkdir -p "$EVIDENCE_ROOT" "$METRICS_DIR"

"$PYTHON" - <<'PY'
import importlib.util
if importlib.util.find_spec("pyarrow") is None:
    raise SystemExit("CALYX_DATASET_TOOLCHAIN_MISSING: pyarrow missing from dataset venv")
PY

"$PYTHON" - "$DATASET_ROOT" "$SAMPLES" "$SOURCE_READBACK" <<'PY'
import hashlib, io, json, math, os, struct, sys, wave
from collections import defaultdict
from pathlib import Path

import pyarrow.parquet as pq

root = Path(sys.argv[1])
sample_path = Path(sys.argv[2])
readback_path = Path(sys.argv[3])

RAVDESS_PER_LABEL = int(os.environ.get("CALYX_MEDIA_RAVDESS_PER_LABEL", "10"))
IEMOCAP_PER_EMOTION = int(os.environ.get("CALYX_MEDIA_IEMOCAP_PER_EMOTION", "8"))
IEMOCAP_LABELS = [
    "angry", "disgust", "excited", "fear", "frustrated",
    "happy", "neutral", "other", "sad", "surprise",
]
IEMOCAP_INDEX = {name: idx for idx, name in enumerate(IEMOCAP_LABELS)}

def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()

def pcm_samples(data: bytes) -> tuple[list[float], int]:
    with wave.open(io.BytesIO(data), "rb") as wav:
        channels = wav.getnchannels()
        width = wav.getsampwidth()
        rate = wav.getframerate()
        frames = wav.readframes(wav.getnframes())
    if width == 1:
        vals = [(byte - 128) / 128.0 for byte in frames]
    elif width == 2:
        count = len(frames) // 2
        vals = [value / 32768.0 for value in struct.unpack("<" + "h" * count, frames)]
    elif width == 4:
        count = len(frames) // 4
        vals = [value / 2147483648.0 for value in struct.unpack("<" + "i" * count, frames)]
    else:
        raise ValueError(f"unsupported wav sample width {width}")
    if channels > 1:
        vals = [sum(vals[i:i + channels]) / channels for i in range(0, len(vals), channels)]
    return vals, rate

def audio_features(data: bytes) -> list[float]:
    samples, rate = pcm_samples(data)
    n = max(len(samples), 1)
    mean = sum(samples) / n
    centered = [value - mean for value in samples]
    rms = math.sqrt(sum(value * value for value in centered) / n)
    mean_abs = sum(abs(value) for value in centered) / n
    peak = max(abs(value) for value in centered) if centered else 0.0
    zcr = sum(
        1 for left, right in zip(centered, centered[1:])
        if (left >= 0.0) != (right >= 0.0)
    ) / max(n - 1, 1)
    duration = n / max(rate, 1)
    feats = [duration / 10.0, rate / 48000.0, rms, mean_abs, peak, zcr]
    for idx in range(8):
        start = idx * n // 8
        end = max((idx + 1) * n // 8, start + 1)
        chunk = centered[start:end]
        feats.append(math.sqrt(sum(value * value for value in chunk) / len(chunk)))
    return [round(value, 6) for value in feats]

def emit(out, row):
    out.write(json.dumps(row, sort_keys=True, separators=(",", ":")) + "\n")

def load_ravdess(out):
    path = root / "ravdess" / "ravdess-train.parquet"
    table = pq.ParquetFile(path).read()
    counts = defaultdict(int)
    rows = 0
    for idx, row in enumerate(table.to_pylist()):
        label = int(row["labels"])
        if counts[label] >= RAVDESS_PER_LABEL:
            continue
        data = row["audio"]["bytes"]
        emit(out, {
            "sample_id": f"ravdess:{idx}",
            "dataset": "ravdess",
            "audio_features": audio_features(data),
            "emotion_label": label,
            "source_sha256": sha256(data),
        })
        counts[label] += 1
        rows += 1
        if len(counts) == 8 and all(value >= RAVDESS_PER_LABEL for value in counts.values()):
            break
    return {"path": str(path), "rows": rows, "labels": len(counts)}

def load_iemocap(out):
    rows = 0
    counts = defaultdict(int)
    files = sorted((root / "iemocap").glob("train-*.parquet"))
    for file in files:
        for idx, row in enumerate(pq.ParquetFile(file).read().to_pylist()):
            emotion = row["major_emotion"]
            if emotion not in IEMOCAP_INDEX:
                continue
            if counts[emotion] >= IEMOCAP_PER_EMOTION:
                continue
            data = row["audio"]["bytes"]
            emit(out, {
                "sample_id": f"iemocap:{file.name}:{idx}",
                "dataset": "iemocap",
                "audio_features": audio_features(data),
                "emotion_label": 100 + IEMOCAP_INDEX[emotion],
                "source_sha256": sha256(data),
            })
            counts[emotion] += 1
            rows += 1
            if len(counts) == len(IEMOCAP_LABELS) and all(
                counts[name] >= IEMOCAP_PER_EMOTION for name in IEMOCAP_LABELS
            ):
                return {"files_seen": files.index(file) + 1, "rows": rows, "labels": len(counts)}
    return {"files_seen": len(files), "rows": rows, "labels": len(counts)}

sample_path.parent.mkdir(parents=True, exist_ok=True)
with sample_path.open("w", encoding="utf-8") as out:
    summary = {
        "ravdess": load_ravdess(out),
        "iemocap": load_iemocap(out),
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
  --stdout "$EVIDENCE_ROOT/media_emotion_validate.log" \
  --stderr "$EVIDENCE_ROOT/media_emotion_validate.stderr" \
  -- cargo run -p calyx-cli -- media emotion-validate \
  --samples "$SAMPLES" \
  --metrics-dir "$METRICS_DIR" \
  --vault "$VAULT_DIR" \
  --min-bits "${CALYX_MEDIA_EMOTION_MIN_BITS:-0.05}" \
  --k "${CALYX_MEDIA_EMOTION_K:-3}"

printf 'MEDIA_EMOTION_FSV_ROOT=%s\n' "$EVIDENCE_ROOT"
