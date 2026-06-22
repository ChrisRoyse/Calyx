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

CUDA_FAIL_LOUD_DETAIL = "cuda:0,error_on_failure,no_cpu_fallback"


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
    processor = load_processor(axis, processor_id, config)
    request = read_frame(sys.stdin.buffer)
    vectors = [
        embed_one(axis, processor, session, bytes(row)).tolist()
        for row in request.get("inputs", [])
    ]
    write_frame(sys.stdout.buffer, {"vectors": vectors})
    return 0


def load_session(model_file: Path, provider: str | None) -> ort.InferenceSession:
    if provider is None or provider == "cpu_explicit":
        return load_cpu_session(model_file)
    if provider in {"cuda_fail_loud", CUDA_FAIL_LOUD_DETAIL}:
        return load_cuda_session(model_file, allow_cpu_fallback=False)
    if provider in {"cuda_preferred", "cuda:0,allow_cpu_fallback"}:
        return load_cuda_session(model_file, allow_cpu_fallback=True)
    raise RuntimeError(f"unsupported provider {provider!r}")


def load_cpu_session(model_file: Path) -> ort.InferenceSession:
    available = ort.get_available_providers()
    if "CPUExecutionProvider" not in available:
        raise RuntimeError(f"CPUExecutionProvider unavailable: {available}")
    return ort.InferenceSession(str(model_file), providers=["CPUExecutionProvider"])


def load_cuda_session(model_file: Path, allow_cpu_fallback: bool) -> ort.InferenceSession:
    available = ort.get_available_providers()
    if "CUDAExecutionProvider" not in available:
        raise RuntimeError(f"CUDAExecutionProvider unavailable: {available}")
    options = ort.SessionOptions()
    providers: list[Any] = [("CUDAExecutionProvider", {"device_id": 0})]
    if allow_cpu_fallback:
        providers.append("CPUExecutionProvider")
    else:
        options.add_session_config_entry("session.disable_cpu_ep_fallback", "1")
    session = ort.InferenceSession(str(model_file), sess_options=options, providers=providers)
    if "CUDAExecutionProvider" not in session.get_providers():
        raise RuntimeError(f"CUDAExecutionProvider did not load: {session.get_providers()}")
    return session


def load_processor(axis: str, model_id: str, config: dict[str, Any]) -> Any:
    if axis == "image":
        return load_image_processor(model_id)
    if axis == "audio":
        from transformers import AutoFeatureExtractor

        return AutoFeatureExtractor.from_pretrained(model_id)
    if axis in {"protein", "dna", "molecule"}:
        return load_sequence_processor(axis, model_id, config)
    raise RuntimeError(f"unsupported multimodal axis {axis}")


def load_image_processor(model_id: str) -> dict[str, Any]:
    root = Path(model_id)
    config_path = root / "preprocessor_config.json"
    if not config_path.exists():
        raise RuntimeError(f"missing image preprocessor config {config_path}")
    config = json.loads(config_path.read_text(encoding="utf-8"))
    tokenizer = None
    if (root / "tokenizer.json").exists():
        from transformers import AutoTokenizer

        tokenizer = AutoTokenizer.from_pretrained(str(root), trust_remote_code=True)
    return {
        "kind": "manual_image",
        "config": config,
        "tokenizer": tokenizer,
        "prompt": "Represent this document page.",
    }


def load_sequence_processor(axis: str, model_id: str, config: dict[str, Any]) -> dict[str, Any]:
    from transformers import AutoTokenizer

    tokenizer = AutoTokenizer.from_pretrained(model_id, trust_remote_code=True)
    return {
        "kind": "tokenizer",
        "axis": axis,
        "tokenizer": tokenizer,
        "kmer": int(config.get("kmer") or 0),
    }


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
    if axis in {"protein", "dna", "molecule"}:
        return preprocess_sequence(processor, payload)
    raise RuntimeError(f"unsupported multimodal axis {axis}")


def preprocess_image(processor: dict[str, Any], payload: bytes) -> dict[str, np.ndarray]:
    from PIL import Image

    config = processor["config"]
    image = Image.open(io.BytesIO(payload))
    if config.get("do_convert_rgb") is not False:
        image = image.convert("RGB")
    if config.get("do_image_splitting"):
        return preprocess_split_image(processor, image)
    if config.get("do_resize", True):
        width, height = image_resize_size(image, image_resize_config(config))
        image = image.resize((width, height), image_resample(config.get("resample", 2)))
    if config.get("do_center_crop", False):
        width, height = image_crop_size(config)
        image = center_crop(image, width, height)
    if config.get("do_pad", False):
        width, height = image_pad_size(image, config)
        image = pad_to_size(image, width, height)

    pixel_values = image_pixels(config, image)[np.newaxis, ...]
    features = {"pixel_values": pixel_values}
    add_image_tokens(features, processor)
    return features


def preprocess_split_image(processor: dict[str, Any], image: Any) -> dict[str, np.ndarray]:
    config = processor["config"]
    tile_size = split_tile_size(config)
    canvas_width, canvas_height = split_canvas_size(config, image, tile_size)
    images = split_page_images(config, image)
    pixel_values = np.stack([image_pixels(config, item) for item in images], axis=0)[
        np.newaxis, ...
    ]
    features = {"pixel_values": pixel_values.astype(np.float32, copy=False)}
    add_image_tokens(
        features,
        processor,
        grid=(canvas_height // tile_size, canvas_width // tile_size),
    )
    return features


def image_pixels(config: dict[str, Any], image: Any) -> np.ndarray:
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
    return np.transpose(pixels, (2, 0, 1)).astype(np.float32, copy=False)


def add_image_tokens(
    features: dict[str, np.ndarray],
    processor: dict[str, Any],
    grid: tuple[int, int] | None = None,
) -> None:
    tokenizer = processor.get("tokenizer")
    if tokenizer is not None:
        prompt = image_prompt(processor, grid)
        features.update(dict(tokenizer(prompt, return_tensors="np", truncation=True)))


def image_prompt(processor: dict[str, Any], grid: tuple[int, int] | None = None) -> str:
    config = processor["config"]
    prompt = processor.get("prompt", "")
    image_seq_length = int(config.get("image_seq_length") or config.get("num_image_tokens") or 0)
    if image_seq_length <= 0:
        if grid is not None:
            split_tokens = split_image_tokens(processor, grid)
            if split_tokens:
                return f"{split_tokens}{prompt}"
        return prompt
    image_token = config.get("image_token") or "<image>"
    return f"{image_token * image_seq_length}{prompt}"


def split_image_tokens(processor: dict[str, Any], grid: tuple[int, int]) -> str:
    tokenizer = processor.get("tokenizer")
    if tokenizer is None:
        return ""
    vocab = tokenizer.get_vocab()
    if "<global-img>" not in vocab:
        return ""
    rows, cols = grid
    tokens = ["<global-img>"]
    for row in range(1, rows + 1):
        for col in range(1, cols + 1):
            token = f"<row_{row}_col_{col}>"
            if token not in vocab:
                return ""
            tokens.append(token)
    return "".join(tokens)


def split_page_images(config: dict[str, Any], image: Any) -> list[Any]:
    tile_size = split_tile_size(config)
    canvas_width, canvas_height = split_canvas_size(config, image, tile_size)
    canvas = fit_to_canvas(image, canvas_width, canvas_height)
    tiles = []
    for top in range(0, canvas_height, tile_size):
        for left in range(0, canvas_width, tile_size):
            tiles.append(canvas.crop((left, top, left + tile_size, top + tile_size)))
    global_image = fit_to_canvas(image, tile_size, tile_size)
    return [global_image, *tiles]


def split_tile_size(config: dict[str, Any]) -> int:
    max_size = config.get("max_image_size")
    if isinstance(max_size, dict) and "longest_edge" in max_size:
        return validate_image_size(max_size["longest_edge"], max_size["longest_edge"])[0]
    return image_pad_size_from_config(config)[0]


def split_canvas_size(config: dict[str, Any], image: Any, tile_size: int) -> tuple[int, int]:
    size = config.get("size")
    if isinstance(size, dict) and "longest_edge" in size:
        long_edge = int(size["longest_edge"])
    else:
        long_edge = tile_size * 4
    long_tiles = max(1, round(long_edge / tile_size))
    short_tiles = max(1, long_tiles - 1)
    width, height = image.size
    if width >= height:
        return tile_size * long_tiles, tile_size * short_tiles
    return tile_size * short_tiles, tile_size * long_tiles


def fit_to_canvas(image: Any, width: int, height: int) -> Any:
    from PIL import Image

    source_width, source_height = image.size
    if source_width <= 0 or source_height <= 0:
        raise RuntimeError(f"invalid decoded image size {source_width}x{source_height}")
    scale = min(width / source_width, height / source_height)
    resized = image.resize(
        validate_image_size(round(source_width * scale), round(source_height * scale)),
        Image.Resampling.BILINEAR,
    )
    return pad_to_size(resized, width, height)


def preprocess_sequence(processor: dict[str, Any], payload: bytes) -> dict[str, np.ndarray]:
    text = payload.decode("utf-8")
    if processor.get("axis") == "dna" and processor.get("kmer", 0) > 0:
        text = dna_kmers(text, int(processor["kmer"]))
    tokenizer = processor["tokenizer"]
    return dict(tokenizer(text, return_tensors="np", truncation=True))


def dna_kmers(text: str, kmer: int) -> str:
    sequence = "".join(text.split()).upper()
    if kmer <= 0:
        return sequence
    if len(sequence) < kmer:
        return sequence
    return " ".join(sequence[index : index + kmer] for index in range(len(sequence) - kmer + 1))


def image_resize_size(image: Any, size: Any) -> tuple[int, int]:
    if isinstance(size, int):
        return validate_image_size(size, size)
    if not isinstance(size, dict):
        raise RuntimeError("image preprocessor config missing size")
    if "height" in size and "width" in size:
        return validate_image_size(size["width"], size["height"])
    if "longest_edge" in size:
        longest_edge = int(size["longest_edge"])
        if longest_edge <= 0:
            raise RuntimeError(f"invalid image longest_edge {longest_edge}")
        width, height = image.size
        if width <= 0 or height <= 0:
            raise RuntimeError(f"invalid decoded image size {width}x{height}")
        if width >= height:
            return validate_image_size(longest_edge, round(height * longest_edge / width))
        return validate_image_size(round(width * longest_edge / height), longest_edge)
    if "shortest_edge" in size:
        shortest_edge = int(size["shortest_edge"])
        if shortest_edge <= 0:
            raise RuntimeError(f"invalid image shortest_edge {shortest_edge}")
        width, height = image.size
        if width <= 0 or height <= 0:
            raise RuntimeError(f"invalid decoded image size {width}x{height}")
        if width <= height:
            return validate_image_size(shortest_edge, round(height * shortest_edge / width))
        return validate_image_size(round(width * shortest_edge / height), shortest_edge)
    raise RuntimeError("image preprocessor config missing size.height/width or size.shortest_edge")


def image_resize_config(config: dict[str, Any]) -> Any:
    if config.get("do_image_splitting") and isinstance(config.get("max_image_size"), dict):
        return config["max_image_size"]
    return config.get("size")


def image_crop_size(config: dict[str, Any]) -> tuple[int, int]:
    crop_size = config.get("crop_size")
    if isinstance(crop_size, int):
        return validate_image_size(crop_size, crop_size)
    if isinstance(crop_size, dict) and "height" in crop_size and "width" in crop_size:
        return validate_image_size(crop_size["width"], crop_size["height"])
    size = config.get("size")
    if isinstance(size, int):
        return validate_image_size(size, size)
    if isinstance(size, dict) and "height" in size and "width" in size:
        return validate_image_size(size["width"], size["height"])
    if isinstance(size, dict) and "longest_edge" in size:
        edge = int(size["longest_edge"])
        return validate_image_size(edge, edge)
    if isinstance(size, dict) and "shortest_edge" in size:
        edge = int(size["shortest_edge"])
        return validate_image_size(edge, edge)
    raise RuntimeError("image preprocessor config missing crop_size")


def image_pad_size(image: Any, config: dict[str, Any]) -> tuple[int, int]:
    configured = image_pad_size_from_config(config)
    if configured is not None:
        return configured
    width, height = image.size
    edge = max(width, height)
    return validate_image_size(edge, edge)


def image_pad_size_from_config(config: dict[str, Any]) -> tuple[int, int] | None:
    for key in ("max_image_size", "size"):
        size = config.get(key)
        if isinstance(size, dict) and "height" in size and "width" in size:
            return validate_image_size(size["width"], size["height"])
        if isinstance(size, dict) and "longest_edge" in size:
            edge = int(size["longest_edge"])
            return validate_image_size(edge, edge)
        if isinstance(size, dict) and "shortest_edge" in size:
            edge = int(size["shortest_edge"])
            return validate_image_size(edge, edge)
        if isinstance(size, int):
            return validate_image_size(size, size)
    return None


def validate_image_size(width: Any, height: Any) -> tuple[int, int]:
    width = int(width)
    height = int(height)
    if width <= 0 or height <= 0:
        raise RuntimeError(f"invalid image processor size {width}x{height}")
    return width, height


def pad_to_size(image: Any, width: int, height: int) -> Any:
    if image.size == (width, height):
        return image
    if image.size[0] > width or image.size[1] > height:
        image = center_crop(image, width, height)
        if image.size == (width, height):
            return image
    from PIL import Image

    padded = Image.new(image.mode, (width, height), color=0)
    left = max((width - image.size[0]) // 2, 0)
    top = max((height - image.size[1]) // 2, 0)
    padded.paste(image, (left, top))
    return padded


def center_crop(image: Any, width: int, height: int) -> Any:
    from PIL import ImageOps

    image_width, image_height = image.size
    pad_width = max(width - image_width, 0)
    pad_height = max(height - image_height, 0)
    if pad_width > 0 or pad_height > 0:
        left = pad_width // 2
        top = pad_height // 2
        image = ImageOps.expand(
            image,
            border=(left, top, pad_width - left, pad_height - top),
            fill=0,
        )
        image_width, image_height = image.size

    left = max((image_width - width) // 2, 0)
    top = max((image_height - height) // 2, 0)
    return image.crop((left, top, left + width, top + height))


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
            features[spec.name] = synthesize_feature(spec.name, features)
        value = np.asarray(features[spec.name])
        if spec.name == "pixel_values" and value.ndim == 4 and len(spec.shape) == 5:
            value = value[:, np.newaxis, ...]
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


def synthesize_feature(name: str, features: dict[str, np.ndarray]) -> np.ndarray:
    if name == "attention_mask" and "input_ids" in features:
        return np.ones_like(np.asarray(features["input_ids"]), dtype=np.int64)
    if name == "token_type_ids" and "input_ids" in features:
        return np.zeros_like(np.asarray(features["input_ids"]), dtype=np.int64)
    if name == "pixel_attention_mask" and "pixel_values" in features:
        pixels = np.asarray(features["pixel_values"])
        if pixels.ndim == 4:
            return np.ones((pixels.shape[0], 1, pixels.shape[2], pixels.shape[3]), dtype=np.int64)
        if pixels.ndim == 5:
            return np.ones(
                (pixels.shape[0], pixels.shape[1], pixels.shape[3], pixels.shape[4]),
                dtype=np.int64,
            )
    raise RuntimeError(f"processor did not produce required ONNX input {name}")


def select_vector(axis: str, session: ort.InferenceSession, outputs: list[np.ndarray]) -> np.ndarray:
    by_name = {meta.name: np.asarray(value) for meta, value in zip(session.get_outputs(), outputs)}
    if axis == "image":
        names = [
            "l2norm_image_embeddings",
            "image_embeddings",
            "image_embeds",
            "embeddings",
            "pooler_output",
            "last_hidden_state",
        ]
    elif axis == "audio":
        names = [
            "l2norm_audio_embeddings",
            "audio_embeddings",
            "audio_embeds",
            "embeddings",
            "pooler_output",
            "last_hidden_state",
        ]
    else:
        names = [
            "l2norm_text_embeddings",
            "text_embeddings",
            "sentence_embedding",
            "embeddings",
            "pooler_output",
            "last_hidden_state",
        ]
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
