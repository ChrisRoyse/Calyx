# PH44 · T05 — τ recalibration trigger + lens park on decay

| Field | Value |
|---|---|
| **Phase** | PH44 — Self-Heal (Rebuild Derived, Degrade Flags) |
| **Stage** | S10 — Anneal + Intelligence Objective J |
| **Crate** | `calyx-anneal` |
| **Files** | `crates/calyx-anneal/src/heal/recalibrate.rs` (≤500) |
| **Depends on** | T01 (DegradeRegistry), T02 (TauDriftDetector + SignalDecayDetector fire faults) |
| **Axioms** | A16, A14 |
| **PRD** | `dbprdplans/12 §2`, `dbprdplans/12 §6` |

## Goal

Handle two specific fault cases: (1) a drifted `τ` (FAR creep) triggers Ward
recalibration via the `WardRecalibrate` trait — the new `τ` is shadow-tested
through the PH43 substrate before being promoted; (2) a lens whose signal has
decayed below `0.05 bits` (per Assay's `bits_per_anchor`) is auto-parked — the
lens stays in the registry but is removed from all search routing until manually
un-parked or a new differentiation measurement clears `≥ 0.05 bits`. Every
action is reversible + Ledger-logged.

## Build (checklist of concrete, code-level steps)

- [ ] `trait WardRecalibrate: Send + Sync { fn recalibrate(&self, slot_id: SlotId, snapshot: MvccSnapshot, budget: BudgetHandle) -> Result<NewTau, CalyxError>; }` — implemented by the Ward crate (bridged); Anneal calls this via the trait, Ward does the conformal calibration.
- [ ] `fn trigger_tau_recalibration(&self, slot_id: SlotId, drift_event: &TauDriftEvent, substrate: &mut AnnealSubstrate) -> Result<RecalibrationOutcome, CalyxError>` — calls `WardRecalibrate::recalibrate`, wraps the new `τ` as a config change, passes through `substrate.propose_change` (shadow+tripwire); on `Promote`: update Ward's live `τ`, write Ledger `action=TauRecalibrated`; on `Revert`: keep current `τ`, write Ledger `action=TauRecalibrationReverted`.
- [ ] `fn park_decayed_lens(&self, lens_id: LensId, bits: f64, registry: &mut DegradeRegistry, ledger: &AnnealLedger) -> Result<(), CalyxError>` — asserts `bits < 0.05` (fails with `CALYX_ANNEAL_PARK_THRESHOLD_NOT_MET` if not); calls `registry.set_health(LensEndpoint{lens_id}, Parked{reason: "signal_decayed"})`, writes Ledger `action=LensPark`; updates `DegradeRegistry::active_lenses` immediately.
- [ ] `fn unpark_lens(&self, lens_id: LensId, new_bits: f64, registry: &mut DegradeRegistry, ledger: &AnnealLedger) -> Result<(), CalyxError>` — only if `new_bits >= 0.05`; sets health back to `Ok`; writes Ledger `action=LensUnpark`.
- [ ] Alert path: both actions write a structured alert to `alerts.jsonl` (same as T04).
- [ ] Clock-injected; no frozen-lens weight modification anywhere in this file.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `τ` drift event fires; `trigger_tau_recalibration` called; new `τ` beats tripwires in shadow → `Promote`; Ledger has `TauRecalibrated`; `DegradeRegistry` shows `GuardProfile: Ok`.
- [ ] unit: new `τ` fails shadow (FAR worse than incumbent) → `Revert`; Ledger has `TauRecalibrationReverted`; Ward's live `τ` unchanged.
- [ ] unit: `park_decayed_lens` with `bits=0.04` → health transitions to `Parked`; `active_lenses` no longer includes this lens_id; Ledger has `LensPark` entry.
- [ ] edge: `park_decayed_lens` with `bits=0.06` → `CALYX_ANNEAL_PARK_THRESHOLD_NOT_MET`; `unpark_lens` with `bits=0.03` → `CALYX_ANNEAL_UNPARK_THRESHOLD_NOT_MET`; park an already-parked lens → idempotent (no double-entry in Ledger).
- [ ] fail-closed: `WardRecalibrate::recalibrate` returns `Err` → `CALYX_WARD_RECALIBRATE_FAILED`; current `τ` unchanged; Ledger records the failure.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** Ledger `TauRecalibrated` / `LensPark` entries; `DegradeRegistry` health; Ward's live `τ` value; `active_lenses` output.
- **Readback:** `calyx anneal status --health` (shows parked lenses); `calyx readback ledger --kind Anneal --action LensPark --last 1`; `calyx ward tau --slot 0` (shows current τ).
- **Prove:** inject `bits=0.02` for lens `L1` via `SignalDecayDetector`; confirm `park_decayed_lens` fires; `status --health` shows `L1: Parked`; `calyx search` with panel including `L1` succeeds without `L1`'s results (degraded mode, no hang); Ledger has `LensPark` entry with `bits=0.02`.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH44 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
