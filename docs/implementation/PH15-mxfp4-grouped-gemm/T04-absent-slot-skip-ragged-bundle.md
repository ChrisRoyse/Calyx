# PH15 · T04 — Absent-slot skip + ragged-bundle correctness

| Field | Value |
|---|---|
| **Phase** | PH15 — MXFP4/Microscaling + Grouped GEMM |
| **Stage** | S2 — Forge Math Runtime |
| **Crate** | `calyx-forge` |
| **Files** | `crates/calyx-forge/src/cuda/grouped_gemm.rs` (≤500) — extension of T03 |
| **Depends on** | T03 (this phase) |
| **Axioms** | A13, A16, A25 |
| **PRD** | `dbprdplans/23 §3`, `dbprdplans/12_STAGE2_FORGE.md PH15` |

## Goal

Prove that a **mixed-completeness batch** (some constellations have some slots
absent) produces correct per-constellation results — absent slots are **skipped,
never zero-filled**. This is an architectural invariant: zero-filling an absent
slot would corrupt agreement scores and MI estimates by introducing phantom
signal. The fail-closed contract: if the caller expects a result for a `None`
slot, that is a programmer error detected by a debug assertion.

## Build (checklist of concrete, code-level steps)

- [x] Extend `GroupedGemmPlan` with `pub slot_ids: Vec<Option<usize>>` — maps each
  problem position back to the slot index in the output; `None` entries are absent
  slots whose output buffers must not be written
- [x] `pub struct RaggedBatch { pub n_constellations: usize, pub n_slots: usize, pub plan: GroupedGemmPlan }`
  — represents one microbatch of `n_constellations × n_slots` problems where any
  slot in any constellation may be absent
- [x] `pub fn build_ragged_batch(ctx: &CudaContext, problems: Vec<Vec<Option<GemmProblem>>>) -> Result<RaggedBatch, ForgeError>`
  — flattens the 2D `problems[cx][slot]` into the 1D plan; `None` → `None` problem;
  verifies that the total number of `Some` entries ≤ the slab capacity
- [x] After `execute_grouped_gemm`, verify (in debug builds via `debug_assert!`) that
  output buffer bytes at `None` slot offsets equal their initial sentinel value (a
  per-element `f32::NAN` written before dispatch); if any sentinel was overwritten →
  `debug_assert` fires with message `"absent slot {i} output was written — grouped GEMM absent-slot skip violated"`
- [x] `pub fn extract_ragged_results(batch: &RaggedBatch) -> Vec<Vec<Option<Vec<f32>>>>`
  — returns the output as a 2D `[cx][slot]` structure; `None` slot → `None` in output
  (never fabricated zeros)

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `RaggedBatch` with cx=2, slot=3, one absent slot in cx=0 slot=1:
  results for present slots correct (within 1e-4); `result[0][1]` is `None`
- [x] unit: all-absent batch (all `None`) → no kernel launch; `extract_ragged_results`
  returns all `None`; no panic
- [x] unit: all-present batch → same result as `grouped_equals_per_loop` (T03)
- [x] proptest: for random 4×4 batch with 50% absent slots (seed=42):
  present slots' results match per-loop; absent slots' results are `None`
- [x] edge (≥3): (1) cx=1, all slots absent; (2) cx=100, all slots present;
  (3) absent slot at position 0 (first in list)
- [x] fail-closed: attempting to read output from a `None` slot → caller gets `None`,
  not a zero vector (checked by the type system — `Option<Vec<f32>>`)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `grouped_gemm_tests::ragged_absent_slot_no_zero_fill` on aiwonder
- **Readback:**
  ```bash
  source $CALYX_HOME/repo/env.sh
  cargo test -p calyx-forge --features cuda ragged_absent -- --nocapture 2>&1 \
    | grep -E "absent|None|PASSED|FAILED"
  ```
- **Prove:** `ragged_absent_slot_no_zero_fill` PASSED; output prints `slot[0][1]=None`
  (absent slot); present slots show non-None values; absent: any zero-filled absent
  result appearing as `Some([0.0, 0.0, ...])`

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] CPU↔GPU bit-parity ≤ 1e-3 on the golden set
- [x] FSV evidence attached to PH15 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
