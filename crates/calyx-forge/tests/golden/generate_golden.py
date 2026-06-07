#!/usr/bin/env python3
"""Generate immutable PH12 Forge golden fixtures."""

from __future__ import annotations

import json
from pathlib import Path

import numpy as np
import scipy
from scipy.spatial import distance


SEED_LABEL = "0xCALYX12"
# Mnemonic seed label above is canonical for Calyx; Python hex literals cannot
# contain Y, so the numeric seed is derived from the exact label bytes.
SEED = int.from_bytes(SEED_LABEL.encode("ascii"), "little") & ((1 << 63) - 1)
SEED_VERSION = 1
N_VECS = 64
DIM = 128
GEMM_M = 128
GEMM_K = 64
GEMM_N = 32
TOPK = 8


def write_f32(path: Path, values: np.ndarray) -> None:
    path.write_bytes(np.asarray(values, dtype="<f4").tobytes(order="C"))


def main() -> None:
    out_dir = Path(__file__).resolve().parent
    rng = np.random.default_rng(seed=SEED)

    vectors = rng.uniform(-0.25, 0.25, size=(N_VECS, DIM)).astype(np.float32)
    gemm_a_row = rng.uniform(-0.25, 0.25, size=(GEMM_M, GEMM_K)).astype(np.float32)
    gemm_b_row = rng.uniform(-0.25, 0.25, size=(GEMM_K, GEMM_N)).astype(np.float32)
    gemm_c_row = np.dot(gemm_a_row, gemm_b_row).astype(np.float32)

    query = vectors[0]
    candidates = vectors[1:]
    cosine_ref = np.array(
        [1.0 - distance.cosine(query, candidate) for candidate in candidates],
        dtype=np.float32,
    )
    topk_ref = np.argsort(-cosine_ref, kind="stable")[:TOPK].astype(np.float32)

    write_f32(out_dir / "vectors_128d.bin", vectors)
    write_f32(out_dir / "gemm_A.bin", np.asfortranarray(gemm_a_row).ravel(order="F"))
    write_f32(out_dir / "gemm_B.bin", np.asfortranarray(gemm_b_row).ravel(order="F"))
    write_f32(out_dir / "gemm_C_ref.bin", np.asfortranarray(gemm_c_row).ravel(order="F"))
    write_f32(out_dir / "cosine_ref.bin", cosine_ref)
    write_f32(out_dir / "topk_ref.bin", topk_ref)

    manifest = {
        "seed": SEED_LABEL,
        "seed_numeric": SEED,
        "seed_version": SEED_VERSION,
        "numpy_version": np.__version__,
        "scipy_version": scipy.__version__,
        "n_vecs": N_VECS,
        "dim": DIM,
        "gemm_m": GEMM_M,
        "gemm_k": GEMM_K,
        "gemm_n": GEMM_N,
        "topk": TOPK,
        "vectors_layout": "row-major f32 little-endian",
        "gemm_layout": "column-major f32 little-endian",
        "regeneration_policy": "never regenerate without bumping seed_version",
    }
    (out_dir / "golden_manifest.json").write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


if __name__ == "__main__":
    main()
