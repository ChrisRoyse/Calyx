# PH29 · T02 — Stratified bits + recurrence anchor + no-multiplier invariant

| Field | Value |
|---|---|
| **Phase** | PH29 — Differentiation contract + n_eff |
| **Stage** | S5 — Loom + Assay (DDA & Bits) |
| **Crate** | `calyx-assay` |
| **Files** | `crates/calyx-assay/src/stratified.rs` (≤500) |
| **Depends on** | T01 (AdmitResult, bits gate) · PH28 T01 (KSG per-stratum) |
| **Axioms** | A7, A8 |
| **PRD** | `dbprdplans/07 §3c`, `26 §9` |

## Goal

Implement stratified bits so a lens carrying a rare-but-critical outcome class
is not lost to the global ≥ 0.05 bits gate. Compute `I(lens; outcome)` per
outcome stratum; admit a lens if it clears ≥ 0.05 bits on **some** grounded
stratum (the sole-carrier check). Implement the recurrence anchor
`AnchorKind::Recurrence` (Bayesian rate). Add the no-raw-frequency-multiplier
regression test. This refines A7 without changing the threshold.

Post-sweep #340 tracking note: Stage 5 implements stratified bits and the
no-frequency-multiplier invariant, but typed recurrence anchor/rate/CI semantics
are not complete in PH29. They are tracked as PH42 grounded-recurrence wiring
(`docs/implementation/PH42-grounded-recurrence-wiring/T01-assay-oracle-self-consistency.md`)
and must be FSV-proven there against recurrence series data.

## Build (checklist of concrete, code-level steps)

- [x] Define `StratifiedBits`: `{ strata: Vec<StratumResult>, sole_carrier_flag: bool }` where `StratumResult = { stratum: AnchorStratum, bits: MiEstimate, n_samples: usize }`
- [x] Implement `stratified_lens_signal(slot: SlotId, anchor: AnchorKind, vault, forge, clock) -> Result<StratifiedBits, CalyxError>`:
  - partition the labeled samples by outcome class (for binary anchors: {Pass, Fail}; for multi-class: each class)
  - for each stratum with n ≥ 50: call `ksg_with_ci` on the within-stratum samples
  - for strata with n < 50: record `CALYX_ASSAY_INSUFFICIENT_SAMPLES` for that stratum and continue
  - set `sole_carrier_flag = true` iff at least one stratum has `bits ≥ 0.05` while the global aggregate MI < 0.05
- [x] Refine `admit_lens` (from T01) to call `stratified_lens_signal` after a global `bits < 0.05` rejection:
  - if global bits < 0.05 BUT `stratified_bits.sole_carrier_flag` → **override to `Admit`** with the per-stratum bits attached; log `"admitted as rare-class sole carrier"`
  - if global bits < 0.05 AND no stratum clears 0.05 bits → keep `Reject { reason: LowSignal }`
- [x] Add `AnchorKind::Recurrence { rate: f32, ci_low: f32, ci_high: f32 }` to the anchor enum in `calyx-core`; this is the Bayesian Gamma–Poisson rate estimate (full Bayesian posterior in `26 §6` is a later task; for now use the raw Poisson MLE rate with a CI from `26 §6` formula)
- [x] Wire `AnchorKind::Recurrence` into `lens_signal`: treated as a grounded anchor (A2 — it is a real count); tagged `Trusted` in `MiEstimate`
- [x] **No-multiplier regression test:** assert `lens_signal(slot, anchor).bits == ksg_with_ci(slot, anchor).bits` — bits are NOT multiplied by any frequency scalar; this test is a hard regression guard

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: slot with 80% majority class (MI < 0.05 globally) but MI ≥ 0.07 on the 20% minority class (n_minority=60, n=300, seed=42) → `sole_carrier_flag = true`; `admit_lens` returns `Admit`
- [x] unit: slot with MI < 0.05 globally AND all strata < 0.05 → `Reject { reason: LowSignal }` (no sole carrier)
- [x] unit: `AnchorKind::Recurrence` lens signal: synthetic lens vectors correlated with a Poisson rate (simulated recurrence counts, seed=42) → `MiEstimate { trust: Trusted, bits > 0.0 }`
- [x] regression: `lens_signal(slot, anchor).bits == ksg_with_ci(slot, anchor).bits` — passes exactly (no frequency multiplication)
- [x] edge: all strata below quorum (50 samples each) → `sole_carrier_flag = false`; all strata empty → `Reject { reason: LowSignal }` (no infinite loop)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** planted rare-class sole carrier — 300-sample dataset, 60 minority-class samples, minority-stratum MI ≈ 0.08 bits (planted, seed=42)
- **Readback:**
  ```
  cargo test stratified_sole_carrier_admitted -- --nocapture
  ```
  Output must show: `global_bits < 0.05`, `stratum[minority].bits ≥ 0.05`, `sole_carrier_flag: true`, `AdmitResult: Admit`.
- **Prove:** run on aiwonder; confirm the rare-class sole carrier is admitted. Also run the no-multiplier regression test and confirm it passes (bits unchanged from KSG output).

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH29 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
