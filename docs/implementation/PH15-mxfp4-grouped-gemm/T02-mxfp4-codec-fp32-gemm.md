# PH15 · T02 — `MxFp4Codec` + fp32-accumulate GEMM path

| Field | Value |
|---|---|
| **Phase** | PH15 — MXFP4/Microscaling + Grouped GEMM |
| **Stage** | S2 — Forge Math Runtime |
| **Crate** | `calyx-forge` |
| **Files** | `crates/calyx-forge/src/quant/mxfp4_codec.rs` (≤500) |
| **Depends on** | T01 (this phase), PH14 T01 (Quantizer trait) |
| **Axioms** | A25, A13, A16 |
| **PRD** | `dbprdplans/23 §4.2`, `dbprdplans/23 §4.4` |

## Goal

Implement `MxFp4Codec` as a `Quantizer` for the MXFP4 path and wire it into
the CUDA GEMM path with fp32 accumulate. The codec exposes an Assay-safety gate:
it returns `ForgeError::QuantError` if the slot has not been deemed quant-safe
(`23 §4.4` contract — FP4 only where Assay later proves quant-safe; in PH15 the
gate is a placeholder that defaults to safe with a TODO marker for PH29 Assay).

## Build (checklist of concrete, code-level steps)

- [x] `src/quant/mxfp4_codec.rs`: `pub struct MxFp4Codec { dim: usize }`
- [x] `impl Quantizer for MxFp4Codec`:
  `encode`: calls `encode_mxfp4(vec)`, serializes `Vec<MxFp4Block>` to bytes, sets
  `QuantizedVec { level: QuantLevel::Bits4Fp, bytes: ..., scale: 0.0, seed_id: [0u8;32] }`;
  add `QuantLevel::Bits4Fp` variant to the enum (value `4.0` bits/channel)
  `decode`: deserializes `Vec<MxFp4Block>` from bytes, calls `decode_mxfp4`
  `dot_estimate`: decode both, compute raw dot (no unbiased correction needed for FP4 path
  since Assay validates the slot; document this)
  `level()`: `QuantLevel::Bits4Fp`; `dim()`: `self.dim`
- [x] `pub fn assay_safety_check_placeholder(slot_id: &str) -> bool`
  — returns `true` always; annotated `// TODO(PH29): replace with real Assay bits check
  (accept_quant §4.4)`; if `false` → `ForgeError::QuantError { detail: "slot {slot_id} not Assay-safe for FP4" }`
- [x] CUDA GEMM path extension in `src/cuda/gemm.rs`: `pub fn gemm_mxfp4_fp32_accum(ctx: &CudaContext, a_blocks: &[MxFp4Block], b_blocks: &[MxFp4Block], m: usize, k: usize, n: usize, out: &mut CudaSlice<f32>) -> Result<(), ForgeError>`
  — sm version check: `ctx.compute_capability()` → if `< (12, 0)` → `ForgeError::DeviceUnavailable { detail: "MXFP4 requires sm_120 (Blackwell). Got sm_XY" }`;
  call cuBLAS fp4 GEMM with fp32 accumulate (or CUTLASS path if cuBLAS doesn't expose fp4 on 13.2)
- [x] Document in `gemm_mxfp4_fp32_accum`: if CUDA 13.3 does not yet expose fp4-native
  GEMM via cuBLAS C API, use CUTLASS 3.x grouped GEMM with MxFp4 dtype; cite the CUTLASS
  `examples/` path in a comment

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `MxFp4Codec::new(128).encode(&vec)` → `QuantizedVec` with `level == Bits4Fp`
- [x] unit: encode→decode roundtrip for a dim-128 unit vector: `cosine(decoded, original) ≥ 0.95`
- [x] unit: `gemm_mxfp4_fp32_accum` result for identity inputs agrees with f32 gemm
  within 5% (fp4 quantization error budget)
- [x] proptest: encode→decode preserves vector sign for random unit vectors (same as T01)
- [x] edge (≥3): (1) sm < 12.0 path → `DeviceUnavailable`; (2) `assay_safety_check_placeholder`
  returns false → `QuantError`; (3) `dim=1536` encode → no panic
- [x] fail-closed: `gemm_mxfp4_fp32_accum` on non-sm120 device → `CALYX_FORGE_DEVICE_UNAVAILABLE`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `mxfp4_tests::mxfp4_codec_roundtrip` + `gemm_mxfp4_within_5pct` on aiwonder
- **Readback:**
  ```bash
  source $CALYX_HOME/repo/env.sh
  cargo test -p calyx-forge --features cuda mxfp4_codec -- --nocapture 2>&1 \
    | grep -E "cosine|within_5pct|PASSED|FAILED"
  ```
- **Prove:** `mxfp4_codec_roundtrip` PASSED with cosine ≥ 0.95; `gemm_mxfp4_within_5pct`
  PASSED; absent: any `DeviceUnavailable` on sm_120 (aiwonder is Blackwell), any cosine < 0.95

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] CPU↔GPU bit-parity ≤ 1e-3 on the golden set (fp4 GEMM vs f32 GEMM within 5% for fp4-safe slots)
- [x] FSV evidence attached to PH15 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
