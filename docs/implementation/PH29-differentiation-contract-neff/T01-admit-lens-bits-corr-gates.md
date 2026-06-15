# PH29 · T01 — `admit_lens`: bits gate + corr gate + error codes

| Field | Value |
|---|---|
| **Phase** | PH29 — Differentiation contract + n_eff |
| **Stage** | S5 — Loom + Assay (DDA & Bits) |
| **Crate** | `calyx-assay` |
| **Files** | `crates/calyx-assay/src/contract.rs` (≤500) |
| **Depends on** | PH28 T06 (`lens_signal`, `pair_redundancy`) · PH27 T04 (agreement_graph for max-corr lookup) |
| **Axioms** | A7, A16 |
| **PRD** | `dbprdplans/07 §3` |

## Goal

Implement `admit_lens` — the differentiation contract's enforcement gate. A
candidate lens is admitted iff it carries ≥ 0.05 bits about a real outcome AND
its maximum pairwise correlation with any existing panel member is ≤ 0.6. The
function returns `AdmitResult::Admit { bits: f32 }` or `AdmitResult::Reject {
reason: LowSignal | Redundant, bits: f32, max_corr: f32 }`. These thresholds
are the paper's verbatim values (`0.05`, `0.6`) and are load-bearing.

## Build (checklist of concrete, code-level steps)

- [x] Define `AdmitResult`:
  ```rust
  pub enum AdmitResult {
      Admit { bits: MiEstimate },
      Reject { reason: RejectReason, bits: MiEstimate, max_corr: f32 },
  }
  pub enum RejectReason { LowSignal, Redundant }
  ```
- [x] Wire error codes: `CALYX_ASSAY_LOW_SIGNAL` maps to `RejectReason::LowSignal`; `CALYX_ASSAY_REDUNDANT` maps to `RejectReason::Redundant`
- [x] Post-sweep #340: reject non-finite `signal_bits` with
  `CALYX_ASSAY_LOW_SIGNAL` and non-finite `max_pairwise_corr` with
  `CALYX_ASSAY_REDUNDANT` before threshold comparisons.
- [x] Implement `admit_lens(candidate: SlotId, anchor: AnchorKind, panel: &Panel, vault, forge, clock) -> Result<AdmitResult, CalyxError>`:
  ```
  bits = lens_signal(candidate, anchor, vault, forge, clock)?
  if bits.bits < 0.05 -> return Reject { reason: LowSignal, bits, max_corr: 0.0 }
  max_corr = max over k in panel of linear_corr(candidate, slot_k)
  if max_corr > 0.6:
      if max_corr in (0.5, 0.7):  // borderline: promote to NMI
          nmi = pair_redundancy_nmi(candidate, argmax_slot, vault, clock)?
          if nmi > 0.6 -> return Reject { reason: Redundant, bits, max_corr }
      else:  // clearly redundant
          return Reject { reason: Redundant, bits, max_corr }
  return Admit { bits }
  ```
- [x] Persist the decision to the assay CF: `(slot_id, anchor, result: Admit|Reject, ts, seq)` — these are the "stored decision rows" read in the FSV
- [x] `linear_corr(a: &[f32], b: &[f32]) -> f32`: Pearson r on the flat slot vectors; fast (no KSG needed for this gate)
- [x] Thresholds `0.05` and `0.6` as named constants `ASSAY_MIN_SIGNAL_BITS: f32 = 0.05` and `ASSAY_MAX_CORR: f32 = 0.6`; config-overridable per vault but default = verbatim paper values

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: candidate with `bits = 0.04` → `Reject { reason: LowSignal }`; with `bits = 0.06` and `max_corr = 0.3` → `Admit`
- [x] unit: candidate with `bits = 0.2` but `max_corr = 0.7` → `Reject { reason: Redundant }`; corr = 0.55 (borderline) + NMI = 0.7 → `Reject`; corr = 0.55 + NMI = 0.4 → `Admit`
- [x] proptest: `Admit` iff `bits >= 0.05 AND max_corr <= 0.6` (or NMI ≤ 0.6 in borderline); the compound condition is total (no missing cases)
- [x] edge: panel with zero members → `max_corr = 0.0`; single-element panel → check only against that one slot; candidate with n < 50 labeled samples → `bits = CALYX_ASSAY_INSUFFICIENT_SAMPLES` propagated (not silently coerced to 0.0)
- [x] fail-closed: missing slot data for a panel member → `CALYX_ASTER_NOT_FOUND`; never returns `Admit` on missing data
- [x] fail-closed: NaN/Inf signal bits or pairwise correlation cannot admit a
  lens (#340).

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** the assay CF decision row for a planted-redundant candidate (corr > 0.6) and a planted-low-signal candidate (bits < 0.05)
- **Readback:**
  ```
  calyx readback --cf assay --decisions --since 0
  ```
  Must show `Rejected(Redundant)` for the high-corr candidate and `Rejected(LowSignal)` for the low-bits candidate.
- **Prove:** run both rejection tests on aiwonder; read the CF rows; confirm both `RejectReason` values match. Also confirm an admitted lens has `status: Admitted { bits: f32 > 0.05 }` in the CF.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH29 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
