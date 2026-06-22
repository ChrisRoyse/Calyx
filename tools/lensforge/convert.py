#!/usr/bin/env python3
"""LensForge conversion harness for PH73.

The harness accepts a small YAML registry and emits deterministic artifacts plus
manifest.json files under CALYX_HOME/lenses by default. It prefers already
published int8 ONNX artifacts when a HuggingFace repository provides them, and
logs explicit skip records when a requested format cannot be produced locally.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any, Iterable

try:
    import yaml
except ImportError as exc:  # pragma: no cover - exercised on missing host deps.
    raise SystemExit("PyYAML is required for tools/lensforge/convert.py") from exc


VERSION = 1
STATIC_LOOKUP_MAGIC = b"CXLKUP1\0"
STATIC_LOOKUP_DTYPE = {"int8": 1, "f16": 2, "float16": 2, "f32": 3, "float32": 3}
ONNX_MODEL_CANDIDATES = (
    "onnx/model_int8.onnx",
    "model_int8.onnx",
    "onnx/model_quantized.onnx",
    "onnx/model.onnx",
    "model.onnx",
)
ONNX_FP32_MODEL_CANDIDATES = ("onnx/model.onnx", "model.onnx")
COMMON_OPTIONAL_FILES = (
    ("config", "config.json"),
    ("tokenizer", "tokenizer.json"),
    ("tokenizer_config", "tokenizer_config.json"),
    ("special_tokens_map", "special_tokens_map.json"),
    ("preprocessor", "preprocessor_config.json"),
    ("vocab", "vocab.json"),
    ("vocab", "vocab.txt"),
    ("merges", "merges.txt"),
    ("added_tokens", "added_tokens.json"),
    ("chat_template", "chat_template.json"),
)
ADAPTER_ONNX_DEFAULTS = {
    "image": {
        "onnx_repo": "onnx-community/siglip2-base-patch16-224-ONNX",
        "onnx_file": "onnx/vision_model_quantized.onnx",
        "model_name": "vision_model_quantized.onnx",
        "dim": 768,
    },
    "audio": {
        "onnx_repo": "Xenova/clap-htsat-unfused",
        "onnx_file": "onnx/audio_model_quantized.onnx",
        "model_name": "audio_model_quantized.onnx",
        "dim": 512,
    },
}
ROLE_ORDER = {
    "model": 0,
    "weights": 0,
    "embeddings": 0,
    "tokenizer": 1,
    "config": 2,
    "preprocessor": 3,
    "tokenizer_config": 4,
    "special_tokens_map": 5,
}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Build Calyx lens artifacts")
    parser.add_argument("registry", nargs="?", help="registry YAML path")
    parser.add_argument("--output-root", help="override output root")
    parser.add_argument("--log", help="JSONL log path")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv)

    if args.self_test:
        return self_test()
    if not args.registry:
        parser.error("registry YAML is required unless --self-test is set")

    registry_path = Path(args.registry)
    config = load_yaml(registry_path)
    output_root = Path(
        expand_env(args.output_root or config.get("output_root") or "${CALYX_HOME}/lenses")
    )
    log_path = Path(args.log) if args.log else output_root / "conversion-log.jsonl"
    output_root.mkdir(parents=True, exist_ok=True)
    log_path.parent.mkdir(parents=True, exist_ok=True)

    manifests: list[dict[str, Any]] = []
    skips = 0
    errors = 0
    for model in normalize_models(config):
        for target_format in model.get("formats", []):
            try:
                result = convert_entry(model, target_format, output_root, log_path)
            except Exception as exc:  # noqa: BLE001 - logged as harness error.
                errors += 1
                log_event(log_path, "error", model, target_format, {"error": str(exc)})
                continue
            if result is None:
                skips += 1
            else:
                manifests.append(result)

    summary = {
        "tool": "lensforge",
        "version": VERSION,
        "output_root": str(output_root),
        "manifests": manifests,
        "skips": skips,
        "errors": errors,
    }
    print(json.dumps(summary, sort_keys=True))
    return 1 if errors else 0


def convert_entry(
    model: dict[str, Any], target_format: str, output_root: Path, log_path: Path
) -> dict[str, Any] | None:
    modality = str(model.get("modality", "")).lower()
    if target_format in {"adapter", "multimodal-adapter", "multimodal_adapter"}:
        return convert_adapter(model, output_root, log_path)
    if target_format == "model2vec" and modality != "text":
        log_event(log_path, "skip", model, target_format, {"reason": "unsupported_format_modality"})
        return None
    if target_format == "model2vec":
        if not python_module_exists("model2vec"):
            log_event(log_path, "skip", model, target_format, {"reason": "dependency_missing"})
            return None
        return convert_model2vec(model, output_root, log_path)
    if target_format == "candle-fp16":
        return convert_candle_fp16(model, output_root, log_path)
    if target_format == "onnx-fp32":
        return convert_onnx_fp32(model, output_root, log_path)
    if target_format != "onnx-int8":
        log_event(log_path, "skip", model, target_format, {"reason": "unsupported_format"})
        return None
    return convert_onnx_int8(model, output_root, log_path)


def convert_adapter(
    model: dict[str, Any], output_root: Path, log_path: Path
) -> dict[str, Any] | None:
    name = str(model.get("name") or safe_name(str(model["hf_id"])))
    modality = str(model["modality"]).lower()
    if modality not in {"image", "audio", "protein", "dna", "molecule"}:
        log_event(
            log_path,
            "skip",
            model,
            "multimodal-adapter",
            {"reason": "unsupported_adapter_modality"},
        )
        return None
    defaults = ADAPTER_ONNX_DEFAULTS.get(modality)
    if defaults is None and "files" not in model:
        log_event(
            log_path,
            "skip",
            model,
            "multimodal-adapter",
            {"reason": "real_multimodal_adapter_not_configured"},
        )
        return None
    out_dir = output_root / safe_name(name) / "onnx-int8"
    out_dir.mkdir(parents=True, exist_ok=True)
    if "files" in model:
        artifacts = copy_local_artifacts(model, out_dir)
        model_path = role_path(artifacts, "model")
    else:
        assert defaults is not None
        onnx_repo = str(model.get("onnx_repo") or defaults["onnx_repo"])
        onnx_file = str(model.get("onnx_file") or defaults["onnx_file"])
        model_path = download_hf_file(onnx_repo, onnx_file, out_dir)
        model_dest = out_dir / str(model.get("onnx_dest") or defaults["model_name"])
        if model_path != model_dest:
            model_path.replace(model_dest)
            model_path = model_dest
        artifacts = {"model": model_path}
        for role, repo_path in COMMON_OPTIONAL_FILES:
            try:
                artifacts[role] = download_hf_file(onnx_repo, repo_path, out_dir)
            except urllib.error.HTTPError as exc:
                if exc.code != 404:
                    raise
    helper_source = Path(__file__).with_name("multimodal_onnx_embed.py")
    helper_path = out_dir / helper_source.name
    shutil.copyfile(helper_source, helper_path)
    artifacts["helper"] = helper_path
    dim = int(model.get("dim") or (defaults or {}).get("dim") or 0)
    if dim <= 0:
        raise ValueError("multimodal adapter dim is required for this modality")
    license_value = str(model.get("license") or "unknown")
    adapter = {
        "schema": "calyx-multimodal-adapter-v2",
        "name": name,
        "axis": modality,
        "model_id": str(model["hf_id"]),
        "processor_model_id": str(model.get("processor_model_id") or "."),
        "dim": dim,
        "engine": "onnx-external",
        "python": str(model.get("python") or "/var/lib/calyx/.venv-gpu/bin/python"),
        "helper": helper_path.name,
        "model_file": model_path.name,
        "provider": str(model.get("provider") or "cpu_explicit"),
        "timeout_ms": int(model.get("timeout_ms") or 120000),
    }
    if "kmer" in model:
        adapter["kmer"] = int(model["kmer"])
    adapter_path = out_dir / "adapter.json"
    write_json(adapter_path, adapter)
    artifacts["adapter"] = adapter_path
    manifest = build_manifest(
        model={**model, "dtype": model.get("dtype", "f32"), "norm": model.get("norm", "l2")},
        target_format="multimodal-adapter",
        artifacts=artifacts,
        dim=dim,
        license_value=license_value,
    )
    manifest_path = out_dir / "manifest.json"
    write_json(manifest_path, manifest)
    log_event(
        log_path,
        "manifest",
        model,
        "multimodal-adapter",
        {"manifest": str(manifest_path), "weights_sha256": manifest["weights_sha256"]},
    )
    return {"name": manifest["name"], "manifest": str(manifest_path)}


def convert_model2vec(
    model: dict[str, Any], output_root: Path, log_path: Path
) -> dict[str, Any] | None:
    import numpy as np
    from model2vec import StaticModel

    name = str(model.get("name") or safe_name(str(model["hf_id"])))
    out_dir = output_root / safe_name(name) / "model2vec"
    out_dir.mkdir(parents=True, exist_ok=True)
    info = hf_model_info(str(model["hf_id"]))
    license_value = model.get("license") or hf_license(info) or "unknown"
    static = StaticModel.from_pretrained(
        str(model["hf_id"]),
        token=os.environ.get("HF_TOKEN") or os.environ.get("HUGGINGFACE_HUB_TOKEN"),
        normalize=True,
        force_download=False,
    )
    matrix = expand_model2vec_matrix(static, np)
    dim = int(model.get("dim") or matrix.shape[1])
    if dim != int(matrix.shape[1]):
        raise ValueError(f"model2vec dim {matrix.shape[1]} != configured {dim}")
    dtype = str(model.get("dtype") or "int8").lower()
    matrix_path = out_dir / "embeddings.cslm"
    actual_dtype = write_static_lookup_matrix(matrix_path, matrix, dtype, np)
    tokenizer_path = out_dir / "tokenizer.json"
    static.tokenizer.save(str(tokenizer_path))
    artifacts = {"embeddings": matrix_path, "tokenizer": tokenizer_path}
    manifest = build_manifest(
        model={**model, "dtype": actual_dtype, "norm": model.get("norm", "l2")},
        target_format="model2vec",
        artifacts=artifacts,
        dim=dim,
        license_value=license_value,
    )
    manifest_path = out_dir / "manifest.json"
    write_json(manifest_path, manifest)
    log_event(
        log_path,
        "manifest",
        model,
        "model2vec",
        {
            "manifest": str(manifest_path),
            "weights_sha256": manifest["weights_sha256"],
            "dtype": actual_dtype,
            "dim": dim,
        },
    )
    return {"name": manifest["name"], "manifest": str(manifest_path)}


def convert_onnx_int8(
    model: dict[str, Any], output_root: Path, log_path: Path
) -> dict[str, Any] | None:
    name = str(model.get("name") or safe_name(str(model["hf_id"])))
    out_dir = output_root / safe_name(name) / "onnx-int8"
    out_dir.mkdir(parents=True, exist_ok=True)
    if "files" in model:
        artifacts = copy_local_artifacts(model, out_dir)
        license_value = model.get("license") or "unknown"
    else:
        info = hf_model_info(str(model["hf_id"]))
        license_value = model.get("license") or hf_license(info) or "unknown"
        artifacts = download_hf_onnx_artifacts(model, info, out_dir, log_path, "onnx-int8", ONNX_MODEL_CANDIDATES)
        if artifacts is None:
            return None

    config_file = role_path(artifacts, "config")
    dim = int(model.get("dim") or dim_from_config(config_file))
    manifest = build_manifest(
        model=model,
        target_format="onnx-int8",
        artifacts=artifacts,
        dim=dim,
        license_value=license_value,
    )
    manifest_path = out_dir / "manifest.json"
    write_json(manifest_path, manifest)
    log_event(
        log_path,
        "manifest",
        model,
        "onnx-int8",
        {"manifest": str(manifest_path), "weights_sha256": manifest["weights_sha256"]},
    )
    return {"name": manifest["name"], "manifest": str(manifest_path)}


def convert_onnx_fp32(
    model: dict[str, Any], output_root: Path, log_path: Path
) -> dict[str, Any] | None:
    name = str(model.get("name") or safe_name(str(model["hf_id"])))
    out_dir = output_root / safe_name(name) / "onnx-fp32"
    out_dir.mkdir(parents=True, exist_ok=True)
    if "files" in model:
        artifacts = copy_local_artifacts(model, out_dir)
        license_value = model.get("license") or "unknown"
    else:
        info = hf_model_info(str(model["hf_id"]))
        license_value = model.get("license") or hf_license(info) or "unknown"
        artifacts = download_hf_onnx_artifacts(model, info, out_dir, log_path, "onnx-fp32", ONNX_FP32_MODEL_CANDIDATES)
        if artifacts is None:
            artifacts = export_onnx_fp32(model, out_dir, log_path)
        if artifacts is None:
            return None

    config_file = role_path(artifacts, "config")
    dim = int(model.get("dim") or dim_from_config(config_file))
    manifest = build_manifest(
        model={**model, "dtype": model.get("dtype", "f32")},
        target_format="onnx",
        artifacts=artifacts,
        dim=dim,
        license_value=license_value,
    )
    manifest_path = out_dir / "manifest.json"
    write_json(manifest_path, manifest)
    log_event(
        log_path,
        "manifest",
        model,
        "onnx-fp32",
        {"manifest": str(manifest_path), "weights_sha256": manifest["weights_sha256"]},
    )
    return {"name": manifest["name"], "manifest": str(manifest_path)}


def convert_candle_fp16(
    model: dict[str, Any], output_root: Path, log_path: Path
) -> dict[str, Any] | None:
    name = str(model.get("name") or safe_name(str(model["hf_id"])))
    out_dir = output_root / safe_name(name) / "candle-fp16"
    out_dir.mkdir(parents=True, exist_ok=True)
    if "files" in model:
        artifacts = copy_local_artifacts(model, out_dir)
        license_value = model.get("license") or "unknown"
    else:
        info = hf_model_info(str(model["hf_id"]))
        license_value = model.get("license") or hf_license(info) or "unknown"
        artifacts = download_hf_candle_artifacts(model, info, out_dir, log_path)
        if artifacts is None:
            return None

    config_file = role_path(artifacts, "config")
    dim = int(model.get("dim") or dim_from_config(config_file))
    dtype = normalize_candle_dtype(str(model.get("dtype") or "f16"))
    manifest = build_manifest(
        model={**model, "dtype": dtype},
        target_format="candle-fp16",
        artifacts=artifacts,
        dim=dim,
        license_value=license_value,
    )
    manifest_path = out_dir / "manifest.json"
    write_json(manifest_path, manifest)
    log_event(
        log_path,
        "manifest",
        model,
        "candle-fp16",
        {
            "manifest": str(manifest_path),
            "weights_sha256": manifest["weights_sha256"],
            "dtype": dtype,
        },
    )
    return {"name": manifest["name"], "manifest": str(manifest_path)}


def download_hf_onnx_artifacts(
    model: dict[str, Any],
    info: dict[str, Any],
    out_dir: Path,
    log_path: Path,
    target_format: str,
    candidates: tuple[str, ...],
) -> dict[str, Path] | None:
    siblings = {entry["rfilename"] for entry in info.get("siblings", []) if "rfilename" in entry}
    model_file = next((candidate for candidate in candidates if candidate in siblings), None)
    if model_file is None:
        log_event(log_path, "skip", model, target_format, {"reason": "preconverted_onnx_missing"})
        return None
    required = [("model", model_file), ("config", "config.json")]
    if str(model.get("modality", "")).lower() in {
        "text",
        "code",
        "mixed",
        "protein",
        "dna",
        "molecule",
    }:
        required.append(("tokenizer", "tokenizer.json"))
    artifacts = {}
    for role, repo_path in required:
        if repo_path not in siblings:
            log_event(
                log_path,
                "skip",
                model,
                target_format,
                {"reason": f"required_{role}_missing", "repo_path": repo_path},
            )
            return None
        artifacts[role] = download_hf_file(str(model["hf_id"]), repo_path, out_dir)
    for role, repo_path in COMMON_OPTIONAL_FILES:
        if repo_path in siblings:
            artifacts[role] = download_hf_file(str(model["hf_id"]), repo_path, out_dir)
    return artifacts


def export_onnx_fp32(model: dict[str, Any], out_dir: Path, log_path: Path) -> dict[str, Path] | None:
    optimum = shutil.which("optimum-cli")
    if optimum is None:
        log_event(log_path, "skip", model, "onnx-fp32", {"reason": "dependency_missing", "program": "optimum-cli"})
        return None
    command = [
        optimum,
        "export",
        "onnx",
        "--model",
        str(model["hf_id"]),
        "--task",
        "feature-extraction",
        str(out_dir),
    ]
    result = subprocess.run(command, text=True, capture_output=True, check=False)
    if result.returncode != 0:
        log_event(
            log_path,
            "skip",
            model,
            "onnx-fp32",
            {
                "reason": "optimum_export_failed",
                "status": result.returncode,
                "stderr": result.stderr[-4096:],
            },
        )
        return None
    required = {"model": out_dir / "model.onnx", "config": out_dir / "config.json"}
    if str(model.get("modality", "")).lower() in {"text", "code", "mixed", "protein", "dna", "molecule"}:
        required["tokenizer"] = out_dir / "tokenizer.json"
    if any(not path.is_file() for path in required.values()):
        missing = [role for role, path in required.items() if not path.is_file()]
        log_event(log_path, "skip", model, "onnx-fp32", {"reason": "export_artifact_missing", "roles": missing})
        return None
    artifacts = dict(required)
    for role, repo_path in COMMON_OPTIONAL_FILES:
        path = out_dir / Path(repo_path).name
        if path.is_file():
            artifacts[role] = path
    return artifacts


def download_hf_candle_artifacts(
    model: dict[str, Any],
    info: dict[str, Any],
    out_dir: Path,
    log_path: Path,
) -> dict[str, Path] | None:
    siblings = {entry["rfilename"] for entry in info.get("siblings", []) if "rfilename" in entry}
    required = [("model", "model.safetensors"), ("tokenizer", "tokenizer.json"), ("config", "config.json")]
    artifacts = {}
    for role, repo_path in required:
        if repo_path not in siblings:
            log_event(
                log_path,
                "skip",
                model,
                "candle-fp16",
                {"reason": f"required_{role}_missing", "repo_path": repo_path},
            )
            return None
        artifacts[role] = download_hf_file(str(model["hf_id"]), repo_path, out_dir)
    for role, repo_path in COMMON_OPTIONAL_FILES:
        if repo_path in siblings and role not in artifacts:
            artifacts[role] = download_hf_file(str(model["hf_id"]), repo_path, out_dir)
    return artifacts


def copy_local_artifacts(model: dict[str, Any], out_dir: Path) -> dict[str, Path]:
    artifacts = {}
    for entry in model["files"]:
        role = str(entry["role"])
        source = Path(expand_env(str(entry["path"])))
        dest_name = str(entry.get("dest") or source.name)
        dest = out_dir / dest_name
        shutil.copyfile(source, dest)
        artifacts[role] = dest
    return artifacts


def expand_model2vec_matrix(static: Any, np: Any) -> Any:
    matrix = static.embedding
    mapping = getattr(static, "token_mapping", None)
    if mapping is not None:
        matrix = matrix[mapping]
    weights = getattr(static, "weights", None)
    if weights is not None:
        matrix = matrix.astype(np.float32, copy=False) * weights[:, None]
    return np.ascontiguousarray(matrix)


def write_static_lookup_matrix(path: Path, matrix: Any, dtype: str, np: Any) -> str:
    dtype_key = dtype.lower()
    if dtype_key not in STATIC_LOOKUP_DTYPE:
        raise ValueError(f"unsupported static lookup dtype {dtype}")
    rows, dim = matrix.shape
    scale = 1.0
    if dtype_key == "int8":
        body, scale = quantize_int8(matrix, np)
        actual_dtype = "int8"
    elif dtype_key in {"f16", "float16"}:
        body = matrix.astype(np.float16, copy=False)
        actual_dtype = "f16"
    else:
        body = matrix.astype(np.float32, copy=False)
        actual_dtype = "f32"
    body = np.ascontiguousarray(body)
    with path.open("wb") as handle:
        handle.write(STATIC_LOOKUP_MAGIC)
        handle.write(int(rows).to_bytes(4, "little"))
        handle.write(int(dim).to_bytes(4, "little"))
        handle.write(bytes([STATIC_LOOKUP_DTYPE[actual_dtype]]))
        handle.write(b"\0\0\0")
        handle.write(struct.pack("<f", scale))
        handle.write(body.tobytes(order="C"))
    return actual_dtype


def quantize_int8(matrix: Any, np: Any) -> tuple[Any, float]:
    peak = float(np.max(np.abs(matrix))) if matrix.size else 0.0
    scale = peak / 127.0 if peak > 0.0 else 1.0
    quantized = np.rint(matrix.astype(np.float32, copy=False) / scale)
    quantized = np.clip(quantized, -127, 127).astype(np.int8)
    return quantized, scale


def build_manifest(
    model: dict[str, Any],
    target_format: str,
    artifacts: dict[str, Path],
    dim: int,
    license_value: str,
) -> dict[str, Any]:
    ordered = ordered_artifacts(artifacts)
    file_entries = []
    for role, path in ordered:
        data = path.read_bytes()
        file_entries.append(
            {
                "role": role,
                "path": path.name,
                "sha256": plain_sha256(data),
                "bytes": len(data),
            }
        )
    model_path = model_artifact_path(artifacts)
    model_hash = plain_sha256(model_path.read_bytes())
    artifact_hash = length_delimited_sha256(path.read_bytes() for _, path in ordered)
    manifest = {
        "name": str(model.get("name") or safe_name(str(model["hf_id"]))),
        "modality": str(model["modality"]).lower(),
        "runtime": target_format,
        "target_format": target_format,
        "dim": dim,
        "dtype": "int8" if target_format == "onnx-int8" else str(model.get("dtype", "f32")),
        "weights_sha256": model_hash,
        "artifact_set_sha256": artifact_hash,
        "files": file_entries,
        "pooling": str(model.get("pooling", "mean")),
        "norm": str(model.get("norm", "l2")),
        "source_hf_id": str(model["hf_id"]),
        "license": license_value,
        "non_commercial": is_non_commercial(license_value),
        "tool": {"name": "lensforge", "version": VERSION},
        "created_by": "tools/lensforge/convert.py",
    }
    if "max_batch" in model:
        max_batch = int(model["max_batch"])
        if max_batch <= 0:
            raise ValueError("max_batch must be > 0")
        manifest["max_batch"] = max_batch
    return manifest


def load_yaml(path: Path) -> dict[str, Any]:
    with path.open("r", encoding="utf-8") as handle:
        data = yaml.safe_load(handle)
    if isinstance(data, list):
        return {"models": data}
    if not isinstance(data, dict):
        raise ValueError("registry YAML must be a list or object with models")
    return data


def normalize_models(config: dict[str, Any]) -> list[dict[str, Any]]:
    models = config.get("models")
    if not isinstance(models, list):
        raise ValueError("registry YAML requires models list")
    normalized = []
    for raw in models:
        if not isinstance(raw, dict):
            raise ValueError("each registry model must be an object")
        entry = dict(raw)
        formats = entry.get("formats")
        if isinstance(formats, str):
            formats = [formats]
        if not formats:
            raise ValueError(f"registry model {entry.get('name', entry.get('hf_id'))} has no formats")
        entry["formats"] = list(formats)
        normalized.append(entry)
    return normalized


def hf_model_info(hf_id: str) -> dict[str, Any]:
    req = urllib.request.Request(
        f"https://huggingface.co/api/models/{hf_id}",
        headers=auth_headers(),
    )
    with open_url(req, timeout=60) as response:
        return json.loads(response.read().decode("utf-8"))


def download_hf_file(hf_id: str, repo_path: str, out_dir: Path) -> Path:
    url = f"https://huggingface.co/{hf_id}/resolve/main/{repo_path}"
    dest = out_dir / Path(repo_path).name
    req = urllib.request.Request(url, headers=auth_headers())
    partial = dest.with_name(f"{dest.name}.part")
    if partial.exists():
        partial.unlink()
    with open_url(req, timeout=600) as response:
        with partial.open("wb") as handle:
            shutil.copyfileobj(response, handle)
    partial.replace(dest)
    return dest


def open_url(req: urllib.request.Request, timeout: int):
    last_error: BaseException | None = None
    for attempt in range(1, 4):
        try:
            return urllib.request.urlopen(req, timeout=timeout)
        except urllib.error.HTTPError as exc:
            if exc.code < 500:
                raise
            last_error = exc
        except (urllib.error.URLError, OSError) as exc:
            last_error = exc
        if attempt < 3:
            time.sleep(float(2**attempt))
    assert last_error is not None
    raise last_error


def auth_headers() -> dict[str, str]:
    token = os.environ.get("HF_TOKEN") or os.environ.get("HUGGINGFACE_HUB_TOKEN")
    return {"Authorization": f"Bearer {token}"} if token else {}


def hf_license(info: dict[str, Any]) -> str | None:
    card = info.get("cardData")
    if isinstance(card, dict):
        value = card.get("license")
        if isinstance(value, str):
            return value
    value = info.get("license")
    return value if isinstance(value, str) else None


def dim_from_config(path: Path) -> int:
    data = json.loads(path.read_text(encoding="utf-8"))
    for key in ("hidden_size", "d_model", "projection_dim", "encoder_embed_dim", "num_features"):
        value = data.get(key)
        if isinstance(value, int) and value > 0:
            return value
    raise ValueError(f"cannot infer dim from {path}")


def role_path(artifacts: dict[str, Path], role: str) -> Path:
    if role in artifacts:
        return artifacts[role]
    raise ValueError(f"missing artifact role {role}")


def model_artifact_path(artifacts: dict[str, Path]) -> Path:
    for role in ("model", "weights", "embeddings"):
        if role in artifacts:
            return artifacts[role]
    raise ValueError("missing artifact role model/weights/embeddings")


def normalize_candle_dtype(dtype: str) -> str:
    key = dtype.lower()
    if key in {"f16", "fp16", "float16"}:
        return "f16"
    if key in {"bf16", "bfloat16"}:
        return "bf16"
    if key in {"f32", "float32"}:
        return "f32"
    raise ValueError(f"unsupported candle dtype {dtype}")


def default_adapter(modality: str) -> str:
    defaults = {
        "image": "pixel-preprocess:v1",
        "audio": "log-mel:v1",
        "protein": "esm-repr-layer:v1",
        "dna": "kmer-bpe:v1",
        "molecule": "smiles-tokenize:v1",
    }
    return defaults.get(modality, "raw-bytes:v1")


def ordered_artifacts(artifacts: dict[str, Path]) -> list[tuple[str, Path]]:
    return sorted(artifacts.items(), key=lambda item: (ROLE_ORDER.get(item[0], 9), item[1].name))


def plain_sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def length_delimited_sha256(parts: Iterable[bytes]) -> str:
    digest = hashlib.sha256()
    for part in parts:
        digest.update(len(part).to_bytes(8, "big"))
        digest.update(part)
    return digest.hexdigest()


def is_non_commercial(license_value: str) -> bool:
    lowered = license_value.lower()
    normalized = lowered.replace("_", "-").replace(" ", "-")
    return (
        "non-commercial" in normalized
        or "noncommercial" in normalized
        or "cc-by-nc" in normalized
        or any(token == "nc" for token in re.split(r"[^a-z0-9]+", normalized))
    )


def safe_name(value: str) -> str:
    return re.sub(r"[^A-Za-z0-9_.-]+", "-", value).strip("-").lower()


def expand_env(value: str) -> str:
    return os.path.expandvars(value)


def python_module_exists(name: str) -> bool:
    result = subprocess.run(
        [sys.executable, "-c", f"import {name}"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    return result.returncode == 0


def write_json(path: Path, value: dict[str, Any]) -> None:
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def log_event(
    log_path: Path,
    event: str,
    model: dict[str, Any],
    target_format: str,
    fields: dict[str, Any],
) -> None:
    record = {
        "ts": int(time.time()),
        "event": event,
        "hf_id": model.get("hf_id"),
        "name": model.get("name"),
        "modality": model.get("modality"),
        "format": target_format,
        **fields,
    }
    with log_path.open("a", encoding="utf-8") as handle:
        handle.write(json.dumps(record, sort_keys=True) + "\n")


def self_test() -> int:
    with tempfile.TemporaryDirectory(prefix="calyx-lensforge-") as tmp:
        root = Path(tmp)
        source = root / "source"
        source.mkdir()
        (source / "model_int8.onnx").write_bytes(b"tiny-model")
        (source / "model.onnx").write_bytes(b"tiny-fp32-model")
        (source / "vision_model_quantized.onnx").write_bytes(b"tiny-vision")
        (source / "tokenizer.json").write_text('{"tiny": true}\n', encoding="utf-8")
        (source / "config.json").write_text('{"hidden_size": 3}\n', encoding="utf-8")
        (source / "preprocessor_config.json").write_text('{"size": {"height": 2, "width": 2}}\n', encoding="utf-8")
        registry = root / "registry.yaml"
        output = root / "out"
        registry.write_text(
            f"""
output_root: {yaml_single_quote(output)}
models:
  - name: tiny-fixture
    hf_id: fixture/tiny
    modality: text
    formats: [onnx-int8]
    pooling: mean
    norm: l2
    files:
      - role: model
        path: {yaml_single_quote(source / "model_int8.onnx")}
      - role: tokenizer
        path: {yaml_single_quote(source / "tokenizer.json")}
      - role: config
        path: {yaml_single_quote(source / "config.json")}
  - name: tiny-fp32-fixture
    hf_id: fixture/tiny-fp32
    modality: text
    formats: [onnx-fp32]
    pooling: mean
    norm: l2
    files:
      - role: model
        path: {yaml_single_quote(source / "model.onnx")}
      - role: tokenizer
        path: {yaml_single_quote(source / "tokenizer.json")}
      - role: config
        path: {yaml_single_quote(source / "config.json")}
  - name: bad-audio
    hf_id: fixture/audio
    modality: audio
    formats: [model2vec]
  - name: tiny-image-adapter
    hf_id: fixture/image
    modality: image
    formats: [adapter]
    dim: 768
    license: mit
    files:
      - role: model
        path: {yaml_single_quote(source / "vision_model_quantized.onnx")}
      - role: config
        path: {yaml_single_quote(source / "config.json")}
      - role: preprocessor
        path: {yaml_single_quote(source / "preprocessor_config.json")}
""",
            encoding="utf-8",
        )
        code = main([str(registry)])
        if code != 0:
            return code
        manifest = output / "tiny-fixture" / "onnx-int8" / "manifest.json"
        data = json.loads(manifest.read_text(encoding="utf-8"))
        actual = plain_sha256((output / "tiny-fixture" / "onnx-int8" / "model_int8.onnx").read_bytes())
        if data["weights_sha256"] != actual:
            print("self-test weights_sha256 mismatch", file=sys.stderr)
            return 1
        fp32_manifest = output / "tiny-fp32-fixture" / "onnx-fp32" / "manifest.json"
        fp32_data = json.loads(fp32_manifest.read_text(encoding="utf-8"))
        if fp32_data["runtime"] != "onnx" or fp32_data["dtype"] != "f32":
            print("self-test onnx-fp32 manifest mismatch", file=sys.stderr)
            return 1
        adapter_manifest = output / "tiny-image-adapter" / "onnx-int8" / "manifest.json"
        adapter_data = json.loads(adapter_manifest.read_text(encoding="utf-8"))
        if adapter_data["runtime"] != "multimodal-adapter" or adapter_data["modality"] != "image":
            print("self-test adapter manifest mismatch", file=sys.stderr)
            return 1
        adapter_roles = {entry["role"] for entry in adapter_data["files"]}
        if not {"model", "adapter", "helper", "preprocessor"}.issubset(adapter_roles):
            print("self-test adapter files missing", file=sys.stderr)
            return 1
        log_text = (output / "conversion-log.jsonl").read_text(encoding="utf-8")
        if "unsupported_format_modality" not in log_text:
            print("self-test missing unsupported modality skip", file=sys.stderr)
            return 1
    return 0


def yaml_single_quote(path: Path) -> str:
    return "'" + str(path).replace("'", "''") + "'"


if __name__ == "__main__":
    raise SystemExit(main())
