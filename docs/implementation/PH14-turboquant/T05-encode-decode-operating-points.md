# PH14 · T05 — Encode / decode roundtrip + operating-point FSV

| Field | Value |
|---|---|
| **Phase** | PH14 — TurboQuant (rotate + scalar + QJL) |
| **Stage** | S2 — Forge Math Runtime |
| **Crate** | `calyx-forge` |
| **Files** | `crates/calyx-forge/tests/turboquant_tests.rs` (≤500) |
| **Depends on** | T03, T04 (this phase) |
| **Axioms** | A25, A13 |
| **PRD** | `dbprdplans/23 §4.1`, `dbprdplans/23 §4.4` |

## Goal

Write the integration tests that prove the two TurboQuant operating points
(quality-neutral ≈ **3.5 bits/channel**; marginal ≈ **2.5 bits/channel**) meet
the inner-product distortion bounds over random vectors. These are the FSV
evidence for the intelligence-preservation contract (`23 §4.4`): the quantizer
is accepted only if `cosine_error ≤ ε_cos`.

## Build (checklist of concrete, code-level steps)

- [x] `tests/turboquant_tests.rs`: helper `fn run_cosine_error_trial(level: QuantLevel, dim: usize, n_pairs: usize, seed: u64) -> f32`
  — generate `n_pairs` random unit-vector pairs via `ChaCha8Rng(seed)`; for each pair
  compute `true_cosine = dot(a, b)`; encode both with `TurboQuantCodec` (fixed rotation
  seed `RotationSeed::new_seed(dim, b"ph14_fsv")`); compute `estimated = dot_estimate_unbiased`;
  return `mean(|estimated - true_cosine|)`
- [x] Test `operating_point_bits3p5_dim128`: `run_cosine_error_trial(Bits3p5, 128, 1000, 42)`
  → assert result ≤ 0.05; print `cosine_err_bits3p5={result:.4}`
- [x] Test `operating_point_bits2p5_dim128`: `run_cosine_error_trial(Bits2p5, 128, 1000, 42)`
  → assert result ≤ 0.10; print `cosine_err_bits2p5={result:.4}`
- [x] Test `operating_point_bits3p5_dim768`: same at dim=768 (real embedding dim from
  gte-multilingual-base on aiwonder :8088); tolerance ≤ 0.03 (higher dim → better concentration)
- [x] Test `encode_decode_roundtrip_bits3p5`: encode a unit vector, decode it, compute
  `1 - cosine(decoded, original)` ≤ 0.01 (decoded vector approximately preserves direction)
- [x] Test `encode_decode_roundtrip_bits2p5`: same, tolerance ≤ 0.05
- [x] Print bytes summary for FSV attachment: first 16 bytes of `encoded.bytes` as hex,
  `encoded.bytes.len()`, scale value — this is the "SoT bytes" for the issue

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] All trials seeded (seed=42 for all) — results are deterministic, not statistical
- [x] proptest: `encode(v).seed_id == codec.seed.id` (seed identity preserved in encoded vec)
- [x] proptest: `decode(encode(v)).len() == v.len()` (dimension preserved through roundtrip)
- [x] edge (≥3): (1) n_pairs=1 (single pair, no mean needed); (2) dim=1 (trivial case);
  (3) all-zeros vector → `ForgeError::NumericalInvariant` (zero-norm after rotation)
- [x] fail-closed: encoding a non-finite vector → `CALYX_FORGE_NUMERICAL_INVARIANT`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `turboquant_tests::operating_point_bits3p5_dim128` + `operating_point_bits3p5_dim768`
- **Readback:**
  ```bash
  source $CALYX_HOME/repo/env.sh
  cargo test -p calyx-forge turboquant_tests -- --nocapture 2>&1 \
    | grep -E "cosine_err|bytes=|len=|scale=|PASSED|FAILED" \
    | tee /tmp/ph14_operating_points.txt
  cat /tmp/ph14_operating_points.txt
  ```
- **Prove:** `cosine_err_bits3p5=0.0XXX` where 0.0XXX ≤ 0.05 printed;
  `cosine_err_bits2p5=0.0XXX` where ≤ 0.10 printed; `cosine_err_bits3p5_dim768`
  ≤ 0.03; first 16 bytes of encoded vec printed as hex (non-zero); all PASSED

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] CPU↔GPU bit-parity ≤ 1e-3 on the golden set (enforced in T06)
- [x] FSV evidence (`/tmp/ph14_operating_points.txt` content) attached to PH14 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
