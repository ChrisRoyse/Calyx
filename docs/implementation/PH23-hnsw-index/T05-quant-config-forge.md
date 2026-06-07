# PH23 · T05 — Per-slot quant config + Forge integration

| Field | Value |
|---|---|
| **Phase** | PH23 — Per-slot HNSW index |
| **Stage** | S4 — Sextant Search & Navigation |
| **Crate** | `calyx-sextant` |
| **Files** | `crates/calyx-sextant/src/index/quant_config.rs` (≤500) |
| **Depends on** | T04 (this phase) · PH14 (TurboQuant) · PH20 (slot definitions) |
| **Axioms** | A25, A16 |
| **PRD** | `dbprdplans/10 §3`, `dbprdplans/12 §4` |

## Goal

Bind a per-slot quantization config to each index (Qdrant-style per-vector
config) so search cost is paid only on participating slots. Vectors are
quantized via Forge TurboQuant before insertion; the config is immutable after
first insert (fail-closed if changed). Distance computation uses the Forge CPU
SIMD path for embedded vaults and the GPU path when the slot's config requests
it.

## Build (checklist of concrete, code-level steps)

- [ ] `SlotQuantConfig` struct:
  ```rust
  pub struct SlotQuantConfig {
      pub dim: usize,
      pub metric: DistanceMetric,   // Cosine | L2 | DotProduct
      pub quant: QuantKind,         // None | Scalar8 | QJL(bits) | MXFP4
      pub use_gpu: bool,
      pub rotate_seed: Option<u64>, // TurboQuant rotation seed
  }
  ```
- [ ] `QuantKind` enum with `Default = None`
- [ ] `fn quantize(cfg: &SlotQuantConfig, vec: &[f32]) -> Result<QuantVec, CalyxError>`:
      calls `calyx_forge::turbo_quant::quantize(vec, cfg.quant, cfg.rotate_seed)`
      (or returns the raw `f32` vec when `QuantKind::None`)
- [ ] `fn distance(cfg: &SlotQuantConfig, a: &QuantVec, b: &QuantVec) -> f32`:
      delegates to Forge CPU/GPU path per `cfg.use_gpu`
- [ ] Wire into `HnswGraph`: store `SlotQuantConfig`; `insert` calls `quantize`;
      `search` quantizes the query then calls `distance` per comparison
- [ ] `CALYX_SEXTANT_QUANT_CONFIG_IMMUTABLE` if a second distinct config is
      supplied after first insert (immutability invariant)
- [ ] `CALYX_SEXTANT_DIM_MISMATCH` if `cfg.dim ≠ vec.len()` on insert or search

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: insert f32 vec with `QuantKind::Scalar8`, search → recall vs unquantized
      brute-force ≥ 0.90 (quantization degrades recall slightly; document floor)
- [ ] unit: `QuantKind::None` path — distance is exact, same as Forge golden
- [ ] unit: `rotate_seed=Some(42)` produces identical quantized bytes on two calls
      with the same input (determinism)
- [ ] proptest: `quantize` then `distance` is non-negative for any unit vectors
      under cosine metric
- [ ] edge: change config after insert → `CALYX_SEXTANT_QUANT_CONFIG_IMMUTABLE`
- [ ] edge: `use_gpu=true` on a machine without CUDA → `CALYX_FORGE_NO_GPU` from
      Forge propagated upward, not silently demoted to CPU
- [ ] fail-closed: `dim=0` in config → `CALYX_SEXTANT_DIM_MISMATCH` at
      construction time, before any insert

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** test output + Forge bit-parity check on aiwonder
- **Readback:** `cargo test -p calyx-sextant quant -- --nocapture 2>&1`
- **Prove:** Scalar8 recall printout shows ≥ 0.90; the bit-parity check prints
  `cpu_gpu_delta=NNN` where NNN ≤ 1e-3 (re-using PH13's golden set via the
  same Forge call path); both printed lines captured as FSV evidence

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] CPU↔GPU bit-parity ≤ 1e-3 on the golden set
- [ ] FSV evidence (readback output / screenshot) attached to the PH23 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
