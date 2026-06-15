# PH14 · T03 — Scalar quantizer (rotate → per-coord scalar quant)

| Field | Value |
|---|---|
| **Phase** | PH14 — TurboQuant (rotate + scalar + QJL) |
| **Stage** | S2 — Forge Math Runtime |
| **Crate** | `calyx-forge` |
| **Files** | `crates/calyx-forge/src/quant/turboquant.rs` (≤500) |
| **Depends on** | T01, T02 (this phase) |
| **Axioms** | A25, A13 |
| **PRD** | `dbprdplans/23 §4.1` |

## Goal

Implement the TurboQuant rotate→scalar-quantize step: apply the Hadamard rotation
(T02) to spread the input vector into a near-Beta distribution, then apply a per-
coordinate uniform scalar quantizer to produce `QuantLevel::Bits3p5` or
`Bits2p5` packed bytes. The scalar quantizer is optimal (MSE-minimizing) for the
Beta distribution produced by the rotation. No codebook training, no dataset
statistics — fully data-oblivious.

## Build (checklist of concrete, code-level steps)

- [x] `src/quant/turboquant.rs`: `pub struct TurboQuantCodec { seed: RotationSeed, level: QuantLevel }`
- [x] `pub fn new(seed: RotationSeed, level: QuantLevel) -> Result<Self, ForgeError>`
  — only `Bits3p5` and `Bits2p5` are valid levels (others → `ForgeError::QuantError
  { detail: "TurboQuant only supports Bits3p5 and Bits2p5" }`)
- [x] `fn rotate_and_quantize_scalar(seed: &RotationSeed, vec: &[f32], level: QuantLevel) -> (Vec<u8>, f32)`
  — applies `apply_rotation` on a copy; computes global scale `s = max(|rotated[i]|)`;
  maps each rotated coord to an integer code in `[0, 2^bits)` via `round((x/s + 1) * (2^bits-1)/2)`
  clipped to range; packs into bytes; returns `(codes_bytes, scale=s)`
- [x] For `Bits3p5`: 7 codes per 7 bits; pack 8 values into 7 bytes (bit-packing, not
  byte-per-value); document the bit layout explicitly in a comment
- [x] For `Bits2p5`: 5 codes per 2.5 bits → 4 values packed into 10 bits = 2 bytes
  (document layout)
- [x] `fn dequantize_scalar(bytes: &[u8], scale: f32, dim: usize, level: QuantLevel) -> Vec<f32>`
  — unpack codes and map back: `x = code * 2s / (2^bits - 1) - s`
- [x] `impl Quantizer for TurboQuantCodec`: `encode` calls `rotate_and_quantize_scalar`
  (+ QJL in T04); `decode` calls `dequantize_scalar`; `level()` returns the configured level

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: round-trip encode→decode a dim-128 all-zeros vector: decoded output
  ≈ zeros within 1e-2 (scale clamp behavior)
- [x] unit: round-trip encode→decode a unit vector at `QuantLevel::Bits3p5`:
  `max |decoded[i] - original_rotated[i]| ≤ 2 * scale / (2^3.5_rounded - 1) * 1.5`
  (quantization error ≤ 1.5 bin widths)
- [x] proptest: for random unit f32 vectors dim=128, seed fixed:
  `max |decoded[i] - rotated[i]| ≤ scale * 2.0 / (7.0 - 1.0)` (Bits3p5 bin width)
- [x] proptest: `encoded.bytes.len()` is deterministic given `(dim, level)` — same
  dim same level → same byte length
- [x] edge (≥3): (1) `dim=1` (trivial); (2) `dim=1536` (large embedding); (3) all-
  identical input vector (degenerate scale)
- [x] fail-closed: `new(seed, QuantLevel::F32)` → `ForgeError::QuantError`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `turboquant_tests::scalar_roundtrip_bits3p5` + `scalar_encode_len_deterministic`
- **Readback:**
  ```bash
  cargo test -p calyx-forge quant::turboquant -- --nocapture 2>&1 \
    | grep -E "roundtrip|bits3p5|bytes_len|PASSED|FAILED"
  ```
- **Prove:** `scalar_roundtrip_bits3p5` PASSED; `bytes_len` test prints a fixed
  integer (e.g. `len=112` for dim=128 at Bits3p5); absent: any decode error or panic

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] CPU↔GPU bit-parity ≤ 1e-3 on the golden set (enforced in T06)
- [x] FSV evidence attached to PH14 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
