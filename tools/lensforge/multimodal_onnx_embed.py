#!/usr/bin/env python3
"""Framed ONNX inference helper for Calyx multimodal adapter lenses."""

from __future__ import annotations

import argparse
import io
import json
import math
import struct
import sys
import wave
from pathlib import Path
from typing import Any

import numpy as np
import onnxruntime as ort
from scipy import signal


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--config", required=True)
    args = parser.parse_args()
    config_path = Path(args.config)
    config = json.loads(config_path.read_text(encoding="utf-8"))
    base = config_path.parent
    axis = config["axis"]
    session = load_session(resolve(base, config["model_file"]), config.get("provider"))
    processor_id = processor_reference(base, config.get("processor_model_id") or config["model_id"])
    processor = load_processor(axis, processor_id)
    request = read_frame(sys.stdin.buffer)
    vectors = [
        embed_one(axis, processor, session, bytes(row)).tolist()
        for row in request.get("inputs", [])
    ]
    write_frame(sys.stdout.buffer, {"vectors": vectors})
    return 0


def load_session(model_file: Path, provider: str | None) -> ort.InferenceSession:
    if provider != "cpu_explicit":
        raise RuntimeError(f"unsupported provider {provider!r}")
    available = ort.get_available_providers()
    if "CPUExecutionProvider" not in available:
        raise RuntimeError(f"CPUExecutionProvider unavailable: {available}")
    return ort.InferenceSession(str(model_file), providers=["CPUExecutionProvider"])


def load_processor(axis: str, model_id: str) -> Any:
    if axis == "image":
        return load_image_processor(model_id)
    if axis == "audio":
        from transformers import AutoFeatureExtractor

        return AutoFeatureExtractor.from_pretrained(model_id)
    raise RuntimeError(f"unsupported multimodal axis {axis}")


def load_image_processor(model_id: str) -> dict[str, Any]:
    config_path = Path(model_id) / "preprocessor_config.json"
    if not config_path.exists():
        raise RuntimeError(f"missing image preprocessor config {config_path}")
    config = json.loads(config_path.read_text(encoding="utf-8"))
    if config.get("image_processor_type") != "SiglipImageProcessor":
        raise RuntimeError(f"unsupported image processor {config.get('image_processor_type')!r}")
    return config


def embed_one(axis: str, processor: Any, session: ort.InferenceSession, payload: bytes) -> np.ndarray:
    features = preprocess(axis, processor, payload)
    feed = build_feed(session, features)
    outputs = session.run(None, feed)
    vector = select_vector(axis, session, outputs)
    return normalize(vector.astype(np.float32, copy=False))


def preprocess(axis: str, processor: Any, payload: bytes) -> dict[str, np.ndarray]:
    if axis == "image":
        return preprocess_image(processor, payload)
    if axis == "audio":
        samples, sampling_rate = decode_wav(payload)
        target_rate = int(getattr(processor, "sampling_rate", sampling_rate))
        if sampling_rate != target_rate:
            samples = resample(samples, sampling_rate, target_rate)
            sampling_rate = target_rate
        return dict(processor(samples, sampling_rate=sampling_rate, return_tensors="np"))
    raise RuntimeError(f"unsupported multimodal axis {axis}")


def preprocess_image(config: dict[str, Any], payload: bytes) -> dict[str, np.ndarray]:
    from PIL import Image

    size = config.get("size")
    if not isinstance(size, dict) or "height" not in size or "width" not in size:
        raise RuntimeError("image preprocessor config missing size.height/size.width")
    height = int(size["height"])
    width = int(size["width"])
    if height <= 0 or width <= 0:
        raise RuntimeError(f"invalid image processor size {height}x{width}")

    image = Image.open(io.BytesIO(payload))
    if config.get("do_convert_rgb") is not False:
        image = image.convert("RGB")
    if config.get("do_resize", True):
        image = image.resize((width, height), image_resample(config.get("resample", 2)))

    pixels = np.asarray(image, dtype=np.float32)
    if pixels.ndim != 3 or pixels.shape[2] != 3:
        raise RuntimeError(f"image payload decoded to unsupported shape {pixels.shape}")
    if config.get("do_rescale", True):
        pixels = pixels * float(config.get("rescale_factor", 1.0 / 255.0))
    if config.get("do_normalize", True):
        mean = np.asarray(config.get("image_mean", [0.5, 0.5, 0.5]), dtype=np.float32)
        std = np.asarray(config.get("image_std", [0.5, 0.5, 0.5]), dtype=np.float32)
        if mean.shape != (3,) or std.shape != (3,) or np.any(std == 0.0):
            raise RuntimeError("image normalization config must contain three nonzero std values")
        pixels = (pixels - mean) / std
    pixel_values = np.transpose(pixels, (2, 0, 1))[np.newaxis, ...].astype(np.float32, copy=False)
    return {"pixel_values": pixel_values}


def image_resample(value: Any) -> Any:
    from PIL import Image

    mapping = {
        0: Image.Resampling.NEAREST,
        1: Image.Resampling.LANCZOS,
        2: Image.Resampling.BILINEAR,
        3: Image.Resampling.BICUBIC,
        4: Image.Resampling.BOX,
        5: Image.Resampling.HAMMING,
    }
    code = int(value)
    if code not in mapping:
        raise RuntimeError(f"unsupported PIL image resample code {code}")
    return mapping[code]


def decode_wav(payload: bytes) -> tuple[np.ndarray, int]:
    with wave.open(io.BytesIO(payload), "rb") as handle:
        channels = handle.getnchannels()
        sample_width = handle.getsampwidth()
        sampling_rate = handle.getframerate()
        frames = handle.readframes(handle.getnframes())
    if sample_width == 1:
        data = (np.frombuffer(frames, dtype=np.uint8).astype(np.float32) - 128.0) / 128.0
    elif sample_width == 2:
        data = np.frombuffer(frames, dtype="<i2").astype(np.float32) / 32768.0
    elif sample_width == 4:
        data = np.frombuffer(frames, dtype="<i4").astype(np.float32) / 2147483648.0
    else:
        raise RuntimeError(f"unsupported WAV sample width {sample_width}")
    if channels > 1:
        data = data.reshape(-1, channels).mean(axis=1)
    return data.astype(np.float32, copy=False), sampling_rate


def resample(samples: np.ndarray, source_rate: int, target_rate: int) -> np.ndarray:
    divisor = math.gcd(source_rate, target_rate)
    return signal.resample_poly(samples, target_rate // divisor, source_rate // divisor).astype(
        np.float32,
        copy=False,
    )


def build_feed(session: ort.InferenceSession, features: dict[str, np.ndarray]) -> dict[str, np.ndarray]:
    feed = {}
    for spec in session.get_inputs():
        if spec.name not in features:
            raise RuntimeError(f"processor did not produce required ONNX input {spec.name}")
        value = np.asarray(features[spec.name])
        if "int64" in spec.type:
            value = value.astype(np.int64, copy=False)
        elif "float" in spec.type:
            value = value.astype(np.float32, copy=False)
        elif "bool" in spec.type:
            value = value.astype(np.bool_, copy=False)
        else:
            raise RuntimeError(f"unsupported ONNX input type {spec.type} for {spec.name}")
        feed[spec.name] = value
    return feed


def select_vector(axis: str, session: ort.InferenceSession, outputs: list[np.ndarray]) -> np.ndarray:
    by_name = {meta.name: np.asarray(value) for meta, value in zip(session.get_outputs(), outputs)}
    names = (
        ["image_embeds", "pooler_output", "last_hidden_state"]
        if axis == "image"
        else ["audio_embeds", "pooler_output", "last_hidden_state"]
    )
    for name in names:
        if name in by_name:
            return flatten_output(by_name[name])
    raise RuntimeError(f"no supported embedding output in {list(by_name)}")


def flatten_output(value: np.ndarray) -> np.ndarray:
    value = np.asarray(value)
    if value.ndim == 1:
        return value
    if value.ndim == 2:
        return value[0]
    if value.ndim == 3:
        return value[0].mean(axis=0)
    raise RuntimeError(f"unsupported embedding output rank {value.ndim}")


def normalize(vector: np.ndarray) -> np.ndarray:
    if not np.isfinite(vector).all():
        raise RuntimeError("embedding contains NaN or Inf")
    norm = float(np.linalg.norm(vector))
    if norm <= 0.0 or not math.isfinite(norm):
        raise RuntimeError("embedding norm is zero or non-finite")
    return vector / norm


def read_frame(stream: Any) -> dict[str, Any]:
    header = stream.read(4)
    if len(header) != 4:
        raise RuntimeError("missing request frame header")
    length = struct.unpack(">I", header)[0]
    body = stream.read(length)
    if len(body) != length:
        raise RuntimeError("truncated request frame")
    return json.loads(body.decode("utf-8"))


def write_frame(stream: Any, value: dict[str, Any]) -> None:
    body = json.dumps(value, separators=(",", ":")).encode("utf-8")
    stream.write(struct.pack(">I", len(body)))
    stream.write(body)
    stream.flush()


def resolve(base: Path, path: str) -> Path:
    candidate = Path(path)
    return candidate if candidate.is_absolute() else base / candidate


def processor_reference(base: Path, value: str) -> str:
    if value.startswith(".") or value.startswith("/") or value.startswith("\\"):
        return str(resolve(base, value))
    return value


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:  # noqa: BLE001 - helper stderr is surfaced by Rust.
        print(f"CALYX_MULTIMODAL_ONNX_HELPER_FAILED: {exc}", file=sys.stderr)
        raise SystemExit(1)
