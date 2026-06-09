# PH21 · T04 — Cost measurement (ms/input, VRAM)

| Field | Value |
|---|---|
| **Phase** | PH21 — Capability cards / profile |
| **Stage** | S3 — Registry / Lenses |
| **Crate** | `calyx-registry` |
| **Files** | `crates/calyx-registry/src/profile.rs` (≤500) |
| **Depends on** | T01 (this phase) |
| **Axioms** | A6 |
| **PRD** | `dbprdplans/05 §5` |

## Goal

Measure `ms_per_input` and `vram_mb_estimated` for a lens by timing
`measure_batch` over the probe set. These numbers go into the `CostMetrics`
field of `CapabilityCard` so operators can make budget decisions.

## Build (checklist of concrete, code-level steps)

- [x] `pub fn measure_cost(registry: &Registry, lens_id: LensId, probe_set: &ProbeSet) -> Result<CostMetrics>`:
  - warm-up: call `registry.measure_batch(lens_id, &probe_set.inputs[..min(8, n)])` once
    (discard result; this warms GPU/JIT).
  - timing run: call `measure_batch` on all probe inputs; measure wall time via
    `std::time::Instant::now()` / `.elapsed()`.
  - `ms_per_input = elapsed_ms / probe_set.inputs.len() as f32`.
  - VRAM: call `nvml_lite::nvml_device_get_memory_info` (or equivalent) before
    and after the timing run; `vram_mb_estimated = (after.used - before.used) /
    1024^2` (clamp to 0 if negative — VRAM may fluctuate).
  - populate `CostMetrics { ms_per_input, vram_mb_estimated, batch_ceiling:
    spec.cost.batch_ceiling }`.
  - return `Ok(cost)`.
- [x] If NVML is unavailable (CPU-only build) → `vram_mb_estimated = 0`.
- [x] Use `#[cfg(feature = "nvml")]` for the VRAM measurement path; compile
  without it for non-CUDA environments.
- [x] `coverage` metric (also in this function):
  - count probe inputs where `measure` returns a valid `SlotVector` (not an
    error); `coverage = valid_count / total_count`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit (mock lens): `measure_cost` on a mock lens that returns immediately;
  `ms_per_input > 0.0` and `< 1000.0` (sanity bounds).
- [x] unit: if all probes return errors → `coverage = 0.0`.
- [x] unit: if all probes succeed → `coverage = 1.0`.
- [x] edge (≥3): (1) probe set of 1 input → `ms_per_input` is the time for
  one call; (2) probe set of 0 inputs → `coverage = 0.0`, `ms_per_input = 0.0`
  (no timing run); (3) NVML unavailable → `vram_mb_estimated = 0`, no panic.
- [x] fail-closed: NVML error is logged and swallowed; function returns
  `vram_mb_estimated = 0`, not an error.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** integration test on aiwonder with real `TeiHttpLens` at `:8088`
- **Readback:**
  `cargo test -p calyx-registry cost_measurement -- --include-ignored --nocapture 2>&1`
- **Prove:** output shows `ms_per_input=<X>ms coverage=1.00` for `:8088` on a
  probe set of 32 inputs; `vram_mb_estimated=<Y>` (may be 0 for TEI since
  VRAM is the TEI server's, not measured by Calyx NVML); screenshot attached
  to PH21 GitHub issue

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH21 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
