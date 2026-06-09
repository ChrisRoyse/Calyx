# PH15 · T01 — MXFP4 block quantization (32-elt blocks, E8M0 scales)

| Field | Value |
|---|---|
| **Phase** | PH15 — MXFP4/Microscaling + Grouped GEMM |
| **Stage** | S2 — Forge Math Runtime |
| **Crate** | `calyx-forge` |
| **Files** | `crates/calyx-forge/src/cuda/mxfp4.rs` (≤500) |
| **Depends on** | PH13 T01 (CudaContext), PH14 T01 (Quantizer trait) |
| **Axioms** | A25, A13 |
| **PRD** | `dbprdplans/23 §4.2`, `dbprdplans/13 §4` |

## Goal

Implement MXFP4/NVFP4 block quantization: partition a f32 vector into **32-element
blocks**, compute per-block **E8M0 (power-of-2) scale**, and encode each element as
a 4-bit mantissa value. E8M0 scales are 8-bit unsigned exponent-only (no mantissa
bit, implicit 1.0; covers range 2^-127 to 2^127). fp32 accumulate means the GEMM
accumulates in fp32 even when multiply operands are fp4. This is the compute-
compression path for Blackwell tensor cores.

## Build (checklist of concrete, code-level steps)

- [x] `pub const MXFP4_BLOCK_SIZE: usize = 32;` — never a magic number in any other file
- [x] `pub struct MxFp4Block { pub codes: [u8; 16], pub scale_e8m0: u8 }`
  — `codes` packs 32 nibbles into 16 bytes (two 4-bit codes per byte, low nibble first);
  `scale_e8m0` is the E8M0 exponent byte: `scale = 2^(scale_e8m0 - 127)` (bias 127)
- [x] `pub fn encode_mxfp4_block(block: &[f32; 32]) -> MxFp4Block`
  — compute `abs_max = block.iter().map(|x| x.abs()).fold(0.0_f32, f32::max)`;
  `exp_biased = (abs_max.log2().floor() as i32).clamp(-127, 127) + 127` as `u8`;
  `scale = 2.0_f32.powi(exp_biased as i32 - 127)`;
  quantize each element: `code = (x / scale).clamp(-7.0, 7.0).round() as i8 + 7` (→ 0..14,
  with 15 reserved for NaN); pack two codes per byte (low nibble = even index)
- [x] `pub fn decode_mxfp4_block(block: &MxFp4Block) -> [f32; 32]`
  — unpack nibbles; `code 15` → return 0.0 (NaN code); `x = (code - 7) as f32 * scale`
- [x] `pub fn encode_mxfp4(vec: &[f32]) -> Result<Vec<MxFp4Block>, ForgeError>`
  — pads with zeros if `vec.len()` not divisible by 32; returns one block per 32 elements
- [x] `pub fn decode_mxfp4(blocks: &[MxFp4Block], original_dim: usize) -> Vec<f32>`
  — decode all blocks and truncate to `original_dim`
- [x] sm version gate: `encode_mxfp4` is CPU-side packing only; the actual tensor-core
  GEMM dispatch (T02) checks `sm >= 12.0` at runtime

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `encode_mxfp4_block([1.0; 32])` → `scale_e8m0 = 127` (2^0 = 1.0);
  all codes = 14 (value 7, the max); decode back → `[1.0; 32]` exactly
- [x] unit: `encode_mxfp4_block([0.0; 32])` → `scale_e8m0 = 0` (or handled as zero-block);
  all codes = 7 (zero value); decode back → `[0.0; 32]`
- [x] proptest: `max |decode(encode(v))[i] - v[i]| ≤ 2 * scale` (quantization error
  ≤ 1 LSB on each element) for random f32 vectors with all-positive values
- [x] proptest: encode→decode preserves sign: `sign(decode(encode(v))[i]) == sign(v[i])`
  for non-zero elements
- [x] edge (≥3): (1) block with one outlier value (rest near zero) — scale captures outlier;
  (2) `vec.len() = 31` (padding); (3) block with all-equal values
- [x] fail-closed: block containing `f32::NAN` → `ForgeError::NumericalInvariant`
  (check before encode, not after)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `mxfp4_tests::encode_ones_block` + `encode_decode_roundtrip` on aiwonder
- **Readback:**
  ```bash
  cargo test -p calyx-forge mxfp4 -- --nocapture 2>&1 \
    | grep -E "scale_e8m0|codes|roundtrip|PASSED|FAILED"
  ```
- **Prove:** `encode_ones_block` PASSED printing `scale_e8m0=127` and `codes=[14,14,...]`;
  `encode_decode_roundtrip` PASSED; absent: any decode producing NaN or values outside
  expected range

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] CPU↔GPU bit-parity ≤ 1e-3 on the golden set (fp4 decode on CPU matches GPU decode)
- [x] FSV evidence attached to PH15 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
