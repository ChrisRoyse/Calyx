# PH14 — TurboQuant (rotate + scalar + QJL)

**Stage:** S2 — Forge Math Runtime  ·  **Crate:** `calyx-forge`  ·
**PRD roadmap:** P4b  ·  **Axioms:** A25, A13, A16

## Objective

Implement Calyx's default slot-vector quantizer — TurboQuant (Google ICLR 2026):
data-oblivious random rotation → optimal per-coordinate scalar quantizer → 1-bit
QJL (Quantized Johnson–Lindenstrauss) residual = **unbiased, low-distortion inner-
product estimator**. Operating points: quality-neutral ≈ **3.5 bits/channel**;
marginal degradation ≈ **2.5 bits/channel** (Google's KV-cache result). The
rotation seed is **versioned and content-addressed** so re-quantizing with the
recorded seed is bit-identical (replay-safe, `24 §7 row 11`). Encode and decode
are data-oblivious and online — no codebook training, zero indexing time — so a
hot-swapped lens is immediately quantizable.

## Dependencies

- **Phases:** PH13 (CUDA sm_120 backend must be DONE — TurboQuant's rotation uses
  the GPU GEMM path for high-dim vectors; CPU path as fallback)
- **Provides for:** PH15 (MXFP4 paired with TurboQuant's per-block scales), PH16
  (autotune cache keyed on `(op="turboquant", dtype, shape, recall_tgt)`), PH23
  (HNSW index uses TurboQuant-encoded slots), PH37 (Ward Gτ cosine on quantized
  codes using unbiased estimator)

## Current state (build off what exists)

`calyx-forge` has CPU SIMD + CUDA backends from PH12/PH13 and the PH14 quant
module is implemented. `TurboQuantCodec`, content-addressed rotation seeds,
QJL residual handling, encode/decode, and seeded replay tests are in-tree.
Build and FSV run natively on aiwonder. The rotation matrix is a random
orthogonal matrix drawn from a seeded PRNG — the seed is the content-addressed
handle.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `src/quant/mod.rs` | Module declarations; `Quantizer` trait; `QuantLevel` enum (Bits1/Bits2p5/Bits3p5/Bits8/F32) |
| `src/quant/turboquant.rs` | `TurboQuantCodec`: seed gen, rotation matrix build, encode (rotate→scalar-quant+QJL), decode, unbiased dot estimator |
| `src/quant/rotation.rs` | Content-addressed `RotationSeed`: versioned seed generation, rotation matrix construction (Hadamard + diagonal random signs), storage |
| `src/quant/qjl.rs` | 1-bit QJL transform on residual; unbiased inner-product correction |
| `tests/turboquant_tests.rs` | Unit + proptest + FSV parity tests; seed replay bit-identity test |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | `Quantizer` trait + `QuantLevel` enum + error extensions | — |
| T02 | Content-addressed `RotationSeed` + rotation matrix construction | T01 |
| T03 | Scalar quantizer (rotate → per-coord scalar quant) | T02 |
| T04 | 1-bit QJL residual + unbiased dot estimator | T03 |
| T05 | Encode / decode roundtrip + operating-point FSV | T03, T04 |
| T06 | Seed replay bit-identity + cosine error bound | T02, T05 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

Run on aiwonder:

```bash
source $CALYX_HOME/repo/env.sh
cargo test -p calyx-forge turboquant -- --nocapture 2>&1 | tee /tmp/ph14_fsv.txt

grep -E "unbiased|cosine_err|replay|bit_identical|PASSED|FAILED" /tmp/ph14_fsv.txt

# Seed replay byte comparison:
# Test prints the first 16 bytes of encode(v, seed_v1) twice — must be identical
grep "seed_replay_bytes" /tmp/ph14_fsv.txt
```

Proof: `turboquant_unbiased_inner_product` PASSED (mean inner-product error ≤
distortion bound ε over 1000 random vector pairs); `seed_replay_bit_identical`
PASSED (two separate encode calls with the recorded seed produce the same bytes —
printed as hex and grep'd); `cosine_err_within_bound` PASSED showing error ≤ ε_cos.

## Risks / landmines

- **Rotation matrix size:** for dim=768 (gte-multilingual-base), the rotation
  matrix is 768×768 f32 = 2.25 MB per seed. Use a structured Hadamard + diagonal
  random-sign construction (O(d log d) apply, O(d) storage for the diagonal) rather
  than storing the full matrix. Document this in `rotation.rs`.
- **Seed versioning:** the content-address is `SHA-256(seed_bytes || dim_bytes)`;
  bump a `seed_version: u8` field if the construction algorithm ever changes —
  old-version codecs must remain decodable.
- **QJL bias correction:** the 1-bit QJL transform requires the Rademacher matrix
  to be fixed (seeded) and stored with the codec so decoding is reproducible.
- **Unbiased estimator variance:** at 3.5 bits/channel the variance of the estimator
  is bounded; at 2.5 bits it increases. The FSV test must use at least 1000 random
  vector pairs to get a stable mean error estimate.
- **CPU↔GPU parity:** the rotation and scalar-quant steps use the `Backend` trait's
  `gemm` for the rotation; GPU path must produce bit-identical encoded bytes to
  the CPU path on the golden set (≤ 1e-3 rel inner-product error after decode).
