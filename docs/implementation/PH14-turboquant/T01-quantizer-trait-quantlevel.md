# PH14 · T01 — `Quantizer` trait + `QuantLevel` enum + error extensions

| Field | Value |
|---|---|
| **Phase** | PH14 — TurboQuant (rotate + scalar + QJL) |
| **Stage** | S2 — Forge Math Runtime |
| **Crate** | `calyx-forge` |
| **Files** | `crates/calyx-forge/src/quant/mod.rs` (≤500) |
| **Depends on** | PH12 T01, PH13 T01 (Backend trait, ForgeError) |
| **Axioms** | A25, A16 |
| **PRD** | `dbprdplans/23 §4.1`, `dbprdplans/13 §3` |

## Goal

Define the `Quantizer` trait (encode/decode/dot-estimate), the `QuantLevel` enum
enumerating the operating points, and extend `ForgeError` with quant-specific
variants. This contract is what PH15 (MXFP4) and PH16 (autotune) depend on.

## Build (checklist of concrete, code-level steps)

- [x] `src/quant/mod.rs`: `pub enum QuantLevel { F32, Bits8, Bits3p5, Bits2p5, Bits1 }`
  — `bits_per_channel()` method returns `32.0_f32, 8.0, 3.5, 2.5, 1.0` respectively;
  `is_lossy()` returns `false` for F32/Bits8, `true` for others;
  `serde::{Serialize, Deserialize}`, `Clone`, `Copy`, `Debug`, `PartialEq`
- [x] `pub trait Quantizer: Send + Sync` with methods:
  `fn encode(&self, vec: &[f32]) -> Result<QuantizedVec, ForgeError>`;
  `fn decode(&self, qv: &QuantizedVec) -> Result<Vec<f32>, ForgeError>`;
  `fn dot_estimate(&self, a: &QuantizedVec, b: &QuantizedVec) -> Result<f32, ForgeError>`;
  `fn level(&self) -> QuantLevel`;
  `fn dim(&self) -> usize`
- [x] `pub struct QuantizedVec { pub level: QuantLevel, pub dim: usize, pub bytes: Vec<u8>, pub scale: f32, pub seed_id: SeedId }`
  — `SeedId` is a `[u8; 32]` (SHA-256 content-address of the rotation seed);
  `serde`, `Clone`, `Debug`
- [x] Extend `ForgeError` (in `src/error.rs`) with:
  `QuantError { op: String, level: String, detail: String, remediation: String }` → maps to `CALYX_FORGE_QUANT_ERROR`;
  `SeedVersionMismatch { expected: u8, got: u8 }` → `CALYX_FORGE_QUANT_SEED_VERSION`
- [x] Re-export `Quantizer`, `QuantLevel`, `QuantizedVec`, `SeedId` from `lib.rs`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `QuantLevel::Bits3p5.bits_per_channel()` == 3.5; `Bits1.is_lossy()` == true
- [x] unit: `QuantizedVec` serde round-trip: serialize to JSON and back → equal
- [x] unit: `QuantLevel` all variants round-trip through `serde_json`
- [x] proptest: any `ForgeError::QuantError` Display starts with `"CALYX_FORGE_QUANT_ERROR"`
- [x] edge (≥3): (1) `F32.is_lossy()` == false; (2) `Bits2p5.bits_per_channel()` == 2.5;
  (3) `QuantizedVec { bytes: vec![], .. }` serializes without panic
- [x] fail-closed: constructing a `ForgeError::SeedVersionMismatch { expected: 1, got: 2 }`
  → Display contains `"CALYX_FORGE_QUANT_SEED_VERSION"`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `cargo test -p calyx-forge quant::mod -- --nocapture` on aiwonder
- **Readback:** `cargo test -p calyx-forge -- quant::mod --nocapture 2>&1 | grep -E "PASSED|FAILED|CALYX_FORGE_QUANT"`
- **Prove:** all tests PASSED; `CALYX_FORGE_QUANT_ERROR` and `CALYX_FORGE_QUANT_SEED_VERSION`
  appear in output from the Display assertions

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] CPU↔GPU bit-parity ≤ 1e-3 on the golden set (enforced in T06)
- [x] FSV evidence attached to PH14 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
