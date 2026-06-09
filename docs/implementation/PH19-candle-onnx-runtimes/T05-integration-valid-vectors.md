# PH19 · T05 — Integration: candle + ONNX each produce valid vectors on aiwonder

| Field | Value |
|---|---|
| **Phase** | PH19 — candle-local + onnx runtimes |
| **Stage** | S3 — Registry / Lenses |
| **Crate** | `calyx-registry` |
| **Files** | `crates/calyx-registry/tests/local_runtimes.rs` (≤500) |
| **Depends on** | T02, T03, T04 (this phase) |
| **Axioms** | A4 |
| **PRD** | `13_STAGE3_REGISTRY.md §PH19 FSV gate` |

## Goal

End-to-end FSV integration test on aiwonder: load a real model via both
`CandleLocalLens` and `OnnxLens`, measure a known text input, assert finite
unit-norm vectors, and confirm the `.hf-cache` path. This is the PH19 exit
gate test; all prior cards feed into it.

## Build (checklist of concrete, code-level steps)

- [x] Test `candle_gte_fsv` (`#[ignore]`):
  - instantiate `HfCacheConfig::from_env()`.
  - `CandleLocalLens::load` with a small real GTE spec (model id
    `"BAAI/bge-small-en-v1.5"` or equivalent available in `.hf-cache`).
  - `lens.measure(Input::new(Modality::Text, b"the quick brown fox"))`.
  - assert `data.len() == dim`, `data.iter().all(|v| v.is_finite())`.
  - compute L2 norm; assert `(norm - 1.0).abs() < 1e-4`.
  - `println!("candle FSV: dim={} norm={:.6}", dim, norm)`.
- [x] Test `onnx_gte_fsv` (`#[ignore]`):
  - same structure, using the explicit CPU `OnnxLens` policy and the
    corresponding `.onnx` file.
  - print `ONNX_FSV_PROVIDER_POLICY=cpu_explicit,no_cuda`,
    `ONNX_FSV_DIM`, `ONNX_FSV_NORM`, and first floats.
- [x] Test `onnx_cuda_fail_loud_fsv` (`#[ignore]`):
  - use default `OnnxLens` construction.
  - print `ONNX_CUDA_PROVIDER_POLICY=cuda:0,error_on_failure,no_cpu_fallback`.
  - assert either a finite unit-norm CUDA vector or
    `CALYX_LENS_UNREACHABLE`; never accept silent CPU fallback.
- [x] Test `hf_cache_path_exists`:
  - `HfCacheConfig::from_env()`.
  - assert `config.root.exists()` and `config.root.is_dir()`.
  - list contents with `std::fs::read_dir`; print each entry name.
- [x] Both FSV tests call `determinism_probe(lens, input)` and assert `Ok(())`.
- [x] Both FSV tests register the lens in a `Registry` and call
  `Registry::measure(id, input)` to confirm the dispatch path works end-to-end.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] `candle_gte_fsv` (integration, `#[ignore]`): passes on aiwonder.
- [x] `onnx_gte_fsv` explicit CPU compatibility path (integration,
  `#[ignore]`): passes on aiwonder and prints `cpu_explicit,no_cuda`.
- [x] `onnx_cuda_fail_loud_fsv` (integration, `#[ignore]`): CUDA succeeds or
  fails loud with `CALYX_LENS_UNREACHABLE`; no CPU fallback.
- [x] `hf_cache_path_exists` (non-ignored if `CALYX_HOME` is set): passes.
- [x] `determinism_probe` called within each FSV test → `Ok(())`.
- [x] edge: both tests print the first 4 float values in hex for the record.
- [x] fail-closed: if `CALYX_HOME` not set → `hf_cache_path_exists` is skipped
  with a `println!("CALYX_HOME not set; skipping")` (not a test failure, since
  the test is gated `#[ignore]`).

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `crates/calyx-registry/tests/local_runtimes.rs` output on aiwonder
- **Readback:**
  `cargo test -p calyx-registry -- --include-ignored --nocapture 2>&1 | grep -E 'FSV|norm|hf.cache'`
- **Prove:** output shows:
  `candle FSV: dim=384 norm=1.000000` (or dim=768 depending on model);
  `ONNX_FSV_PROVIDER_POLICY=cpu_explicit,no_cuda`;
  `ONNX_FSV_DIM=384`;
  `ONNX_FSV_NORM=1.000...`;
  `ONNX_CUDA_PROVIDER_POLICY=cuda:0,error_on_failure,no_cpu_fallback`;
  `hf-cache: $CALYX_HOME/.hf-cache/<model>/`;
  screenshot and the first-4-floats hex dump attached to PH19 GitHub issue

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] [Forge-touching] CPU↔GPU bit-parity ≤ 1e-3 on the golden set
- [x] FSV evidence (readback output / screenshot) attached to the PH19 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
