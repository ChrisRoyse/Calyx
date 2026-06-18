#!/usr/bin/env python3
"""Acquire the real audio/video mini corpus for Calyx media FSV (#756)."""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import math
import os
import re
import shutil
import struct
import subprocess
import sys
import urllib.parse
import urllib.request
import wave
import zipfile
from pathlib import Path

DATASET_NAME = "media_fsv_mini"
DEFAULT_DATASET_ROOT = "/home/croyse/calyx/data/datasets"
USER_AGENT = "Calyx-Dev-FSV/1.0 (https://github.com/ChrisRoyse/Calyx-Dev)"
RAVDESS_API = "https://zenodo.org/api/records/1188976"
RAVDESS_PAGE = "https://zenodo.org/records/1188976"
RAVDESS_FILE = "Audio_Speech_Actors_01-24.zip"
COMMONS_API = "https://commons.wikimedia.org/w/api.php"
COMMONS_TITLES = [
    "File:2005 TG45.ogv",
    "File:2012 DA14.ogv",
    "File:2013 Daily Arctic Sea Ice from AMSR2 May - September 2013 01.webm",
]
RAVDESS_LABELS = {
    1: "neutral",
    2: "calm",
    3: "happy",
    4: "sad",
    5: "angry",
    6: "fearful",
    7: "disgust",
    8: "surprised",
}


def fail(code: str, message: str) -> None:
    print(f"{code}: {message}", file=sys.stderr)
    raise SystemExit(1)


def utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat()


def request_json(url: str) -> dict:
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(request, timeout=60) as response:
        return json.loads(response.read().decode("utf-8"))


def download(url: str, dest: Path, *, bytes_expected=None, md5=None, sha1=None) -> None:
    if dest.is_file() and file_matches(dest, bytes_expected, md5, sha1):
        print(f"[cached] {dest}")
        return
    dest.parent.mkdir(parents=True, exist_ok=True)
    tmp = dest.with_suffix(dest.suffix + ".tmp")
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    try:
        with urllib.request.urlopen(request, timeout=120) as response, tmp.open("wb") as out:
            shutil.copyfileobj(response, out, length=1 << 20)
    except OSError as error:
        tmp.unlink(missing_ok=True)
        fail("CALYX_MEDIA_FSV_DOWNLOAD_FAILED", f"{url}: {error}")
    if not file_matches(tmp, bytes_expected, md5, sha1):
        tmp.unlink(missing_ok=True)
        fail("CALYX_MEDIA_FSV_CHECKSUM_MISMATCH", str(dest))
    tmp.replace(dest)
    print(f"[fetched] {dest}")


def file_matches(path: Path, bytes_expected, md5, sha1) -> bool:
    if bytes_expected is not None and path.stat().st_size != int(bytes_expected):
        return False
    if md5 and digest_file(path, "md5") != md5:
        return False
    if sha1 and digest_file(path, "sha1") != sha1:
        return False
    return True


def digest_file(path: Path, algo: str) -> str:
    digest = hashlib.new(algo)
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def ravdess_file_info() -> dict:
    record = request_json(RAVDESS_API)
    for file_info in record["files"]:
        if file_info["key"] == RAVDESS_FILE:
            checksum = file_info["checksum"]
            if not checksum.startswith("md5:"):
                fail("CALYX_MEDIA_FSV_MANIFEST_INVALID", f"unexpected checksum {checksum}")
            return {
                "title": record["metadata"]["title"],
                "doi": record["metadata"]["doi"],
                "license": record["metadata"]["license"]["id"],
                "size": file_info["size"],
                "md5": checksum.split(":", 1)[1],
                "url": file_info["links"]["self"],
            }
    fail("CALYX_MEDIA_FSV_MANIFEST_INVALID", f"{RAVDESS_FILE} missing from {RAVDESS_API}")


def commons_info(title: str) -> dict:
    params = urllib.parse.urlencode(
        {
            "action": "query",
            "format": "json",
            "prop": "imageinfo",
            "iiprop": "url|size|sha1|mime|extmetadata",
            "titles": title,
        }
    )
    data = request_json(f"{COMMONS_API}?{params}")
    page = next(iter(data["query"]["pages"].values()))
    if page.get("missing") is not None or "imageinfo" not in page:
        fail("CALYX_MEDIA_FSV_MANIFEST_INVALID", f"Commons title not found: {title}")
    info = page["imageinfo"][0]
    meta = info.get("extmetadata", {})
    return {
        "title": page["title"],
        "page_url": f"https://commons.wikimedia.org/wiki/{urllib.parse.quote(page['title'])}",
        "url": info["url"],
        "bytes": info["size"],
        "sha1": info["sha1"],
        "mime": info["mime"],
        "license_short_name": meta.get("LicenseShortName", {}).get("value", ""),
        "license_url": meta.get("LicenseUrl", {}).get("value", ""),
        "usage_terms": strip_html(meta.get("UsageTerms", {}).get("value", "")),
    }


def strip_html(value: str) -> str:
    return re.sub(r"<[^>]+>", "", value).strip()


def extract_ravdess_actor01(archive: Path, audio_dir: Path) -> list[Path]:
    audio_dir.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(archive) as zf:
        bad = zf.testzip()
        if bad is not None:
            fail("CALYX_MEDIA_FSV_CHECKSUM_MISMATCH", f"{archive}: corrupt member {bad}")
        members = sorted(
            (
                info
                for info in zf.infolist()
                if not info.is_dir()
                and info.filename.startswith("Actor_01/")
                and info.filename.endswith(".wav")
            ),
            key=lambda info: info.filename,
        )
        if len(members) != 60:
            fail("CALYX_MEDIA_FSV_ROWCOUNT_MISMATCH", f"Actor_01 wav count {len(members)} != 60")
        extracted = []
        for member in members:
            name = Path(member.filename).name
            if "/" in name or "\\" in name:
                fail("CALYX_MEDIA_FSV_MANIFEST_INVALID", f"unsafe zip member {member.filename}")
            target = audio_dir / name
            with zf.open(member) as src, target.open("wb") as out:
                shutil.copyfileobj(src, out, length=1 << 20)
            extracted.append(target)
    return extracted


def ravdess_label(path: Path) -> tuple[int, str]:
    parts = path.stem.split("-")
    if len(parts) != 7 or parts[0:2] != ["03", "01"] or parts[6] != "01":
        fail("CALYX_MEDIA_FSV_MANIFEST_INVALID", f"unexpected RAVDESS filename: {path.name}")
    emotion = int(parts[2])
    if emotion not in RAVDESS_LABELS:
        fail("CALYX_MEDIA_FSV_LABEL_INVALID", f"{path.name}: emotion {emotion}")
    return emotion, RAVDESS_LABELS[emotion]


def audio_features(path: Path) -> list[float]:
    try:
        with wave.open(str(path), "rb") as wav:
            channels = wav.getnchannels()
            width = wav.getsampwidth()
            rate = wav.getframerate()
            frames = wav.readframes(wav.getnframes())
    except (wave.Error, OSError) as error:
        fail("CALYX_MEDIA_FSV_DECODE_FAILED", f"{path}: {error}")
    if width == 1:
        vals = [(byte - 128) / 128.0 for byte in frames]
    elif width == 2:
        vals = [v / 32768.0 for v in struct.unpack("<" + "h" * (len(frames) // 2), frames)]
    elif width == 4:
        vals = [v / 2147483648.0 for v in struct.unpack("<" + "i" * (len(frames) // 4), frames)]
    else:
        fail("CALYX_MEDIA_FSV_DECODE_FAILED", f"{path}: unsupported sample width {width}")
    if channels > 1:
        vals = [sum(vals[i : i + channels]) / channels for i in range(0, len(vals), channels)]
    n = max(len(vals), 1)
    mean = sum(vals) / n
    centered = [value - mean for value in vals]
    rms = math.sqrt(sum(value * value for value in centered) / n)
    mean_abs = sum(abs(value) for value in centered) / n
    peak = max(abs(value) for value in centered) if centered else 0.0
    zcr = sum((a >= 0.0) != (b >= 0.0) for a, b in zip(centered, centered[1:])) / max(n - 1, 1)
    feats = [n / max(rate, 1) / 10.0, rate / 48000.0, rms, mean_abs, peak, zcr]
    for idx in range(8):
        start = idx * n // 8
        end = max((idx + 1) * n // 8, start + 1)
        chunk = centered[start:end]
        feats.append(math.sqrt(sum(value * value for value in chunk) / len(chunk)))
    return [round(value, 6) for value in feats]


def ffprobe(path: Path, *, count_frames: bool) -> dict:
    command = ["ffprobe", "-v", "error"]
    if count_frames:
        command.append("-count_frames")
    command += ["-show_streams", "-show_format", "-of", "json", str(path)]
    try:
        result = subprocess.run(command, check=False, capture_output=True, text=True)
    except FileNotFoundError:
        fail("CALYX_MEDIA_FSV_TOOL_MISSING", "ffprobe is required")
    if result.returncode != 0:
        fail("CALYX_MEDIA_FSV_DECODE_FAILED", f"{path}: {result.stderr.strip()}")
    try:
        return json.loads(result.stdout)
    except json.JSONDecodeError as error:
        fail("CALYX_MEDIA_FSV_DECODE_FAILED", f"{path}: invalid ffprobe JSON: {error}")


def media_stream(probe: dict, codec_type: str, path: Path) -> dict:
    for stream in probe.get("streams", []):
        if stream.get("codec_type") == codec_type:
            return stream
    fail("CALYX_MEDIA_FSV_DECODE_FAILED", f"{path}: no {codec_type} stream")


def audio_metadata(path: Path, rel: str) -> dict:
    probe = ffprobe(path, count_frames=False)
    stream = media_stream(probe, "audio", path)
    duration = stream.get("duration") or probe.get("format", {}).get("duration")
    return {
        "path": rel,
        "sha256": digest_file(path, "sha256"),
        "bytes": path.stat().st_size,
        "duration_seconds": round(float(duration), 6),
        "sample_rate_hz": int(stream["sample_rate"]),
        "channels": int(stream["channels"]),
        "codec": stream.get("codec_name"),
        "container": probe.get("format", {}).get("format_name"),
    }


def video_metadata(path: Path, rel: str, source: dict) -> dict:
    probe = ffprobe(path, count_frames=True)
    stream = media_stream(probe, "video", path)
    frame_count = stream.get("nb_read_frames") or stream.get("nb_frames")
    if frame_count in (None, "N/A"):
        fail("CALYX_MEDIA_FSV_DECODE_FAILED", f"{path}: frame count unavailable")
    fps = parse_fps(stream.get("avg_frame_rate") or stream.get("r_frame_rate") or "0/1")
    return {
        "path": rel,
        "source_title": source["title"],
        "source_url": source["url"],
        "page_url": source["page_url"],
        "license": source["license_short_name"],
        "license_url": source["license_url"],
        "sha256": digest_file(path, "sha256"),
        "bytes": path.stat().st_size,
        "frame_count": int(frame_count),
        "fps": fps,
        "width": int(stream["width"]),
        "height": int(stream["height"]),
        "codec": stream.get("codec_name"),
        "container": probe.get("format", {}).get("format_name"),
        "mime": source["mime"],
    }


def parse_fps(value: str) -> float:
    if "/" not in value:
        return round(float(value), 6)
    numerator, denominator = value.split("/", 1)
    denominator_float = float(denominator)
    return 0.0 if denominator_float == 0.0 else round(float(numerator) / denominator_float, 6)


def rel(path: Path, root: Path) -> str:
    return path.relative_to(root).as_posix()


def write_jsonl(path: Path, rows: list[dict]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8") as handle:
        for row in rows:
            handle.write(json.dumps(row, sort_keys=True, separators=(",", ":")) + "\n")


def acquire(args: argparse.Namespace) -> None:
    dataset_root = Path(args.dataset_root)
    dataset_dir = dataset_root / args.dataset_name
    if args.force:
        shutil.rmtree(dataset_dir, ignore_errors=True)
    if dataset_dir.exists():
        validate_dataset(dataset_dir)
        register_dataset(dataset_root, args.dataset_name)
        return
    downloads = dataset_root / ".downloads" / args.dataset_name
    audio_dir = dataset_dir / "audio" / "ravdess_actor_01"
    video_dir = dataset_dir / "video" / "wikimedia_nasa"
    metadata_dir = dataset_dir / "metadata"
    ravdess = ravdess_file_info()
    archive = downloads / RAVDESS_FILE
    download(ravdess["url"], archive, bytes_expected=ravdess["size"], md5=ravdess["md5"])
    audio_paths = extract_ravdess_actor01(archive, audio_dir)
    video_sources = []
    for source in [commons_info(title) for title in COMMONS_TITLES]:
        ext = Path(urllib.parse.urlparse(source["url"]).path).suffix.lower()
        if ext not in {".ogv", ".webm"}:
            fail("CALYX_MEDIA_FSV_UNSUPPORTED_MEDIA_EXTENSION", f"{source['title']}: {ext}")
        target = video_dir / Path(urllib.parse.unquote(urllib.parse.urlparse(source["url"]).path)).name
        download(source["url"], target, bytes_expected=source["bytes"], sha1=source["sha1"])
        video_sources.append((target, source))
    audio_rows = []
    sample_rows = []
    for path in sorted(audio_paths):
        metadata = audio_metadata(path, rel(path, dataset_dir))
        emotion_id, emotion_name = ravdess_label(path)
        audio_rows.append({**metadata, "emotion_id": emotion_id, "emotion_label": emotion_name})
        sample_rows.append(
            {
                "sample_id": f"ravdess-actor01:{path.stem}",
                "dataset": "ravdess_actor_01",
                "audio_features": audio_features(path),
                "emotion_label": emotion_id,
                "source_sha256": metadata["sha256"],
            }
        )
    video_rows = [video_metadata(path, rel(path, dataset_dir), source) for path, source in video_sources]
    write_jsonl(metadata_dir / "audio_metadata.jsonl", audio_rows)
    write_jsonl(metadata_dir / "audio_samples.jsonl", sample_rows)
    write_jsonl(metadata_dir / "video_metadata.jsonl", video_rows)
    manifest = acquisition_manifest(dataset_dir, ravdess, audio_rows, video_rows)
    (metadata_dir / "acquisition_manifest.json").write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    validate_dataset(dataset_dir)
    register_dataset(dataset_root, args.dataset_name)


def acquisition_manifest(dataset_dir: Path, ravdess: dict, audio_rows: list[dict], video_rows: list[dict]) -> dict:
    media_files = audio_rows + video_rows
    metadata_files = []
    for path in sorted((dataset_dir / "metadata").glob("*.jsonl")):
        metadata_files.append(
            {"path": rel(path, dataset_dir), "sha256": digest_file(path, "sha256"), "bytes": path.stat().st_size}
        )
    return {
        "schema_version": 1,
        "name": DATASET_NAME,
        "downloaded_at_utc": utc_now(),
        "stable_subset_selection_rule": (
            "RAVDESS: all 60 WAV files under Actor_01 from Audio_Speech_Actors_01-24.zip, "
            "sorted by filename. Video: fixed Commons titles in COMMONS_TITLES order."
        ),
        "audio": {
            "source": RAVDESS_PAGE,
            "api": RAVDESS_API,
            "title": ravdess["title"],
            "doi": ravdess["doi"],
            "archive": RAVDESS_FILE,
            "archive_bytes": ravdess["size"],
            "archive_md5": ravdess["md5"],
            "license_id": ravdess["license"],
            "license_text": "Creative Commons Attribution-NonCommercial-ShareAlike 4.0 International",
            "license_url": "https://creativecommons.org/licenses/by-nc-sa/4.0/",
            "file_count": len(audio_rows),
        },
        "video": {
            "source": "https://commons.wikimedia.org/wiki/Category:Videos_from_NASA",
            "api": COMMONS_API,
            "titles": COMMONS_TITLES,
            "file_count": len(video_rows),
            "licenses": sorted({row["license"] for row in video_rows}),
        },
        "totals": {
            "media_file_count": len(media_files),
            "media_bytes": sum(row["bytes"] for row in media_files),
        },
        "media_files": media_files,
        "metadata_files": metadata_files,
    }


def validate_dataset(dataset_dir: Path) -> None:
    manifest_path = dataset_dir / "metadata" / "acquisition_manifest.json"
    if not manifest_path.is_file():
        fail("CALYX_MEDIA_FSV_MANIFEST_INVALID", f"missing {manifest_path}")
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    audio_files = sorted((dataset_dir / "audio").rglob("*"))
    video_files = sorted((dataset_dir / "video").rglob("*"))
    media_paths = [path for path in audio_files + video_files if path.is_file()]
    if not media_paths:
        fail("CALYX_MEDIA_FSV_NOT_FOUND", f"no media files under {dataset_dir}")
    for path in media_paths:
        suffix = path.suffix.lower()
        if path.is_relative_to(dataset_dir / "audio") and suffix != ".wav":
            fail("CALYX_MEDIA_FSV_UNSUPPORTED_MEDIA_EXTENSION", str(path))
        if path.is_relative_to(dataset_dir / "video") and suffix not in {".ogv", ".webm"}:
            fail("CALYX_MEDIA_FSV_UNSUPPORTED_MEDIA_EXTENSION", str(path))
    expected = sorted(row["path"] for row in manifest["media_files"])
    actual = sorted(rel(path, dataset_dir) for path in media_paths)
    if actual != expected:
        fail("CALYX_MEDIA_FSV_MANIFEST_MISMATCH", f"actual media paths differ from manifest")
    hashes = set()
    for path in media_paths:
        sha = digest_file(path, "sha256")
        if sha in hashes:
            fail("CALYX_MEDIA_FSV_DUPLICATE_CONTENT", str(path))
        hashes.add(sha)
        row = next(row for row in manifest["media_files"] if row["path"] == rel(path, dataset_dir))
        if sha != row["sha256"]:
            fail("CALYX_MEDIA_FSV_CHECKSUM_MISMATCH", str(path))
        if path.suffix.lower() == ".wav":
            audio_metadata(path, rel(path, dataset_dir))
        else:
            video_metadata(path, rel(path, dataset_dir), {"title": row.get("source_title", ""), "url": row.get("source_url", ""), "page_url": row.get("page_url", ""), "license_short_name": row.get("license", ""), "license_url": row.get("license_url", ""), "mime": row.get("mime", "")})
    print(
        json.dumps(
            {
                "status": "ok",
                "dataset_dir": str(dataset_dir),
                "media_files": len(media_paths),
                "media_bytes": sum(path.stat().st_size for path in media_paths),
            },
            sort_keys=True,
        )
    )


def register_dataset(dataset_root: Path, dataset_name: str) -> None:
    repo_root = Path(__file__).resolve().parents[1]
    verify = repo_root / "scripts" / "verify_dataset.sh"
    command = [
        "bash",
        str(verify),
        "register",
        dataset_name,
        "--source",
        f"{RAVDESS_PAGE} + Commons NASA fixed titles",
        "--revision",
        "RAVDESS Zenodo record 1188976 actor01 + Commons title list 2026-06-18",
        "--license",
        "RAVDESS CC-BY-NC-SA-4.0; Commons Public domain/CC0",
        "--tests",
        "real audio/video media FSV corpus (#756): ffprobe metadata + Calyx media emotion validate",
        "--rows-from",
        "metadata/audio_metadata.jsonl,metadata/video_metadata.jsonl",
    ]
    env = os.environ.copy()
    env["CALYX_DATASET_ROOT"] = str(dataset_root)
    result = subprocess.run(command, check=False, text=True, env=env)
    if result.returncode != 0:
        fail("CALYX_MEDIA_FSV_REGISTER_FAILED", f"verify_dataset.sh exited {result.returncode}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=["acquire", "validate"])
    parser.add_argument("--dataset-root", default=os.environ.get("CALYX_DATASET_ROOT", DEFAULT_DATASET_ROOT))
    parser.add_argument("--dataset-name", default=DATASET_NAME)
    parser.add_argument("--force", action="store_true")
    args = parser.parse_args()
    if args.mode == "acquire":
        acquire(args)
    else:
        validate_dataset(Path(args.dataset_root) / args.dataset_name)


if __name__ == "__main__":
    main()
