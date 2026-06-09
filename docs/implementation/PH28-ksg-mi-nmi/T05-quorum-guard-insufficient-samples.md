# PH28 · T05 — Quorum guard + `CALYX_ASSAY_INSUFFICIENT_SAMPLES`

| Field | Value |
|---|---|
| **Phase** | PH28 — KSG MI + partitioned NMI |
| **Stage** | S5 — Loom + Assay (DDA & Bits) |
| **Crate** | `calyx-assay` |
| **Files** | `crates/calyx-assay/src/ksg.rs` (≤500), `crates/calyx-assay/src/nmi.rs` (≤500) |
| **Depends on** | T01 (KSG), T04 (NMI) |
| **Axioms** | A16 |
| **PRD** | `dbprdplans/07 §2` |

## Goal

Enforce the quorum rule `n ≥ 50` as a centralized, consistent guard across all
Assay estimators. The guard must fire before any computation and return
`CALYX_ASSAY_INSUFFICIENT_SAMPLES` with the actual n and the required n=50 in
the error payload. This is the fail-closed (A16) enforcement: never a noisy
point estimate when n < 50.

## Build (checklist of concrete, code-level steps)

- [x] Define `AssayInsufficientSamples { n_actual: usize, n_required: usize }` as a variant in `CalyxError`; error code `CALYX_ASSAY_INSUFFICIENT_SAMPLES`; remediation message: `"Provide at least {n_required} grounded samples; only {n_actual} available"`
- [x] Implement `quorum_guard(n: usize, required: usize) -> Result<(), CalyxError>`:
  - if `n < required` → `Err(CalyxError::AssayInsufficientSamples { n_actual: n, n_required: required })`
  - else `Ok(())`
- [x] `ASSAY_DEFAULT_QUORUM: usize = 50` constant in `calyx-assay/src/lib.rs`; all estimators call `quorum_guard(n, ASSAY_DEFAULT_QUORUM)` at entry; quorum is config-overridable per vault but default = 50 (No-Compress List)
- [x] Wire `quorum_guard` into `ksg_estimate_continuous`, `ksg_estimate_discrete_y`, and `partitioned_histogram_nmi_v1` as the first step
- [x] Add `n_samples: usize` field to `CalyxError::AssayInsufficientSamples` for downstream consumers to read the actual count without parsing the message string

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `quorum_guard(49, 50)` → `Err(AssayInsufficientSamples { n_actual: 49, n_required: 50 })`
- [x] unit: `quorum_guard(50, 50)` → `Ok(())`; `quorum_guard(51, 50)` → `Ok(())`
- [x] unit: `ksg_estimate_continuous` with n=49 pairs → returns `CALYX_ASSAY_INSUFFICIENT_SAMPLES` without computing any k-NN distances
- [x] unit: `partitioned_histogram_nmi_v1` with n=30 → returns `CALYX_ASSAY_INSUFFICIENT_SAMPLES` before any bin accumulation
- [x] edge: n=0 → `CALYX_ASSAY_INSUFFICIENT_SAMPLES { n_actual: 0 }`; n=50 exactly → not rejected; `required=0` (overridden) → always passes (edge case for testing)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** calling `ksg_estimate_continuous` with exactly n=49 samples on aiwonder
- **Readback:**
  ```
  cargo test quorum_guard_fail_closed_n49 -- --nocapture
  ```
  Output must be `Err(CALYX_ASSAY_INSUFFICIENT_SAMPLES { n_actual: 49, n_required: 50 })`.
- **Prove:** run on aiwonder; confirm the error is returned (not a panic, not a noisy f32, not a zero). Also run with n=50 and confirm `Ok(MiEstimate { n_samples: 50, … })` is returned.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH28 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
