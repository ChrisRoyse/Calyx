# PH14 · T04 — 1-bit QJL residual + unbiased dot estimator

| Field | Value |
|---|---|
| **Phase** | PH14 — TurboQuant (rotate + scalar + QJL) |
| **Stage** | S2 — Forge Math Runtime |
| **Crate** | `calyx-forge` |
| **Files** | `crates/calyx-forge/src/quant/qjl.rs` (≤500) |
| **Depends on** | T02, T03 (this phase) |
| **Axioms** | A25, A13 |
| **PRD** | `dbprdplans/23 §4.1` |

## Goal

Implement the 1-bit QJL (Quantized Johnson–Lindenstrauss) transform on the scalar-
quantization residual and the unbiased inner-product estimator. Scalar quantization
alone is biased for dot products; the QJL residual corrects this bias so that
`dot_estimate(encode(a), encode(b))` is **unbiased** over the random rotation —
the key property that makes Ward's Gτ, agreement scoring, and RRF work correctly
on quantized codes without dequantization.

## Build (checklist of concrete, code-level steps)

- [x] `src/quant/qjl.rs`: `pub struct QjlResidual { pub bits: Vec<u8>, pub rademacher_seed: SeedId }`
  — `bits` packs one bit per rotated coordinate (sign of residual after scalar quant);
  `rademacher_seed` is a separate seeded Rademacher matrix stored as a diagonal (±1)
  vector (same `RotationSeed` construction, but independent draw)
- [x] `pub fn encode_qjl_residual(rotated: &[f32], scalar_decoded: &[f32], rademacher: &RotationSeed) -> QjlResidual`
  — residual `r_i = rotated[i] - scalar_decoded[i]`; apply the rademacher diagonal:
  `r_i' = r_i * rademacher.diagonal[i]`; `bits[i] = (r_i' > 0) as u8`; pack into bytes
- [x] `pub fn dot_qjl_correction(qa: &QjlResidual, qb: &QjlResidual, rademacher: &RotationSeed, scale_a: f32, scale_b: f32) -> f32`
  — `correction = (1/d) * scale_a * scale_b * Σ_i (2*bit_a[i]-1)(2*bit_b[i]-1)`
  (bipolar decoding); this is the bias-correction term added to the scalar dot estimate
- [x] `pub fn dot_estimate_unbiased(codec: &TurboQuantCodec, qv_a: &QuantizedVec, qv_b: &QuantizedVec) -> Result<f32, ForgeError>`
  — scalar dot estimate (from dequantized codes) + QJL correction; returns the
  unbiased estimate; mismatched `seed_id` between a and b → `ForgeError::QuantError { detail: "seed_id mismatch in dot_estimate" }`
- [x] Update `TurboQuantCodec::encode` (T03): after scalar quant, encode QJL residual
  and store `QjlResidual` serialized bytes appended to `QuantizedVec::bytes` (with a
  1-byte tag `0x01` to distinguish scalar from QJL sections)
- [x] Update `TurboQuantCodec::dot_estimate`: calls `dot_estimate_unbiased`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `dot_estimate_unbiased` on a vector with itself → estimate ≈ 1.0
  (within 0.05 at Bits3p5 for dim=128) since it's the cosine of a unit vector with itself
- [x] unit: orthogonal unit vectors → estimate ≈ 0.0 (within 0.05 at Bits3p5)
- [x] proptest: over 1000 random unit-vector pairs (seed=42), mean of
  `|dot_estimate_unbiased(a,b) - true_dot(a,b)|` ≤ 0.05 at `Bits3p5` dim=128
  (unbiasedness: E[error] ≈ 0)
- [x] proptest: the same 1000-pair test at `Bits2p5` → mean error ≤ 0.10
  (higher distortion at lower bits, but still bounded)
- [x] edge (≥3): (1) parallel vectors → estimate ≈ 1.0; (2) anti-parallel → ≈ -1.0;
  (3) mismatched `seed_id` → `ForgeError::QuantError`
- [x] fail-closed: `dot_estimate_unbiased` with `seed_id` mismatch → `CALYX_FORGE_QUANT_ERROR`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `turboquant_tests::unbiased_dot_mean_error_bits3p5` on aiwonder (1000-pair run)
- **Readback:**
  ```bash
  cargo test -p calyx-forge quant::qjl unbiased_dot -- --nocapture 2>&1 \
    | grep -E "mean_err|unbiased|PASSED|FAILED"
  ```
- **Prove:** `unbiased_dot_mean_error_bits3p5` PASSED printing `mean_err=0.0XX`
  (value ≤ 0.05); `unbiased_dot_mean_error_bits2p5` PASSED with mean_err ≤ 0.10;
  absent: any mean_err > threshold or panic

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] CPU↔GPU bit-parity ≤ 1e-3 on the golden set (enforced in T06)
- [x] FSV evidence (mean_err values + test output) attached to PH14 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
