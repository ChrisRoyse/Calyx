# PH23 · T05 — Per-slot quant config + explicit GPU-unavailable state

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
config) so search cost is paid only on participating slots. Current Sextant code
quantizes locally for the in-RAM HNSW path and keeps the config immutable after
first insert (fail-closed if changed). Forge TurboQuant and CUDA parity remain
Stage 2 capabilities; Sextant does not claim a wired GPU quantization path until
that integration exists. Any Sextant CPU/GPU quantization parity request must
fail loud with `CALYX_SEXTANT_GPU_PARITY_UNAVAILABLE`.

## Build (checklist of concrete, code-level steps)

- [x] `QuantConfig` struct:
  ```rust
  pub struct QuantConfig {
      pub kind: QuantKind,     // None | Scalar8 | Binary
      pub scale: f32,
      pub zero_point: i8,
      locked: bool,
  }
  ```
- [x] `QuantKind` enum with `None`, `Scalar8`, and `Binary`
- [x] `fn quantize(&self, values: &[f32]) -> QuantizedVector` returns raw values
      for `None`, scalar bytes + approximate values for `Scalar8`, and sign bits
      for `Binary`
- [x] Wire into `HnswIndex`: store `QuantConfig`; `insert` locks the config after
      first insert and stores the quantized approximate vector for search
- [x] `cpu_gpu_delta` returns `CALYX_SEXTANT_GPU_PARITY_UNAVAILABLE` instead of a
      CPU-self comparison
- [ ] Future integration: add a real Forge GPU quantization path, then replace
      the unavailable state with CPU/GPU byte-readback parity evidence
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
- [x] edge: Sextant GPU parity requested before a GPU quant path exists →
      `CALYX_SEXTANT_GPU_PARITY_UNAVAILABLE`, not a silent CPU comparison
- [ ] fail-closed: `dim=0` in config → `CALYX_SEXTANT_DIM_MISMATCH` at
      construction time, before any insert

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** aiwonder FSV artifact bytes for the unavailable parity state and
  search fan-out result
- **Readback:** `CALYX_FSV_ROOT=/home/croyse/calyx/data/fsv-issue299-gpu-parity-fanout-20260608 cargo test -p calyx-sextant gpu_parity_and_fanout_aiwonder_fsv -- --ignored --nocapture`
- **Prove:** `gpu-parity-fanout-readback.json` contains
  `quant_cpu_gpu_delta.available=false`,
  `code="CALYX_SEXTANT_GPU_PARITY_UNAVAILABLE"`, and
  `forge_grouped_fanout_wired=false`

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] Sextant GPU parity overclaim removed; GPU parity requests fail loud until
      a real Forge path is wired
- [ ] FSV evidence (readback output / screenshot) attached to the PH23 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
