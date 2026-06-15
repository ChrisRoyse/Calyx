# PH37 · T02 — `SlotVerdict` + `GuardVerdict` types + `WardError` catalog

| Field | Value |
|---|---|
| **Phase** | PH37 — Gτ Guard Math + GuardProfile |
| **Stage** | S8 — Ward Gτ Guard |
| **Crate** | `calyx-ward` |
| **Files** | `crates/calyx-ward/src/verdict.rs` (≤500), `crates/calyx-ward/src/error.rs` (≤500) |
| **Depends on** | T01 (this phase) |
| **Axioms** | A12, A16 |
| **PRD** | `dbprdplans/09 §1`, `09 §8` |

**STATUS:** DONE / FSV-signed-off in #259. The unchecked checklist rows below
are historical build prompts; the authoritative evidence is the #259 closeout
and `/home/croyse/calyx/data/fsv-issue259-ph37-t02-20260609`.

## Goal

Define the structured output types that every `guard()` call returns — a
per-slot breakdown `SlotVerdict { slot, cos, tau, pass }` and an aggregate
`GuardVerdict { overall_pass, per_slot, action }` — plus the `WardError` error
catalog that provides `CALYX_GUARD_OOD` and `CALYX_GUARD_PROVISIONAL` as typed,
fail-closed codes. Downstream callers always receive full decomposition even on
an overall pass.

## Post-implementation note

Implemented in `crates/calyx-ward/src/verdict.rs` and
`crates/calyx-ward/src/error.rs`, with re-exports from
`crates/calyx-ward/src/lib.rs`. aiwonder FSV root:
`/home/croyse/calyx/data/fsv-issue259-ph37-t02-20260609`. Readback artifacts:
`verdict.json`, `errors.json`, and `ward-error-fsv.log`; the durable JSON
readback proves all Ward error codes and the complete per-slot verdict
breakdown.

## Build (checklist of concrete, code-level steps)

- [ ] Define `SlotVerdict` struct:
      `slot: SlotId`, `cos: f32`, `tau: f32`, `pass: bool` — serde, `Clone`,
      `Debug`, `PartialEq`
- [ ] Define `GuardVerdict` struct:
      `overall_pass: bool`, `per_slot: Vec<SlotVerdict>`,
      `action: Option<NoveltyAction>` (set when `overall_pass == false`),
      `guard_id: GuardId` — serde, `Clone`, `Debug`
- [ ] `GuardVerdict::failing_slots(&self) -> Vec<&SlotVerdict>` — returns every
      `SlotVerdict` where `pass == false`
- [ ] `GuardVerdict::all_slot_details(&self) -> &[SlotVerdict]` — full breakdown
      regardless of overall outcome (no pruning on pass)
- [ ] Define `WardError` enum with variants:
      - `Ood { guard_id: GuardId, failing: Vec<SlotVerdict> }` →
        `CALYX_GUARD_OOD`; display includes per-slot `(slot, cos, tau)`
      - `Provisional { guard_id: GuardId }` → `CALYX_GUARD_PROVISIONAL`; display
        names the guard and advises calibrate before high-stakes use
      - `MissingSlot { slot: SlotId }` → `CALYX_GUARD_MISSING_SLOT`; produced
        vector did not include a required slot
      - `PolicyViolation { k: usize, n_required: usize }` →
        `CALYX_GUARD_POLICY_VIOLATION`; `KofN.k > required_slots.len()`
- [ ] Implement `std::error::Error` + `Display` for `WardError`; each message
      includes the `CALYX_*` code string verbatim as remediation prefix
- [ ] Wire both modules into `src/lib.rs`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `GuardVerdict` with two `SlotVerdict`s (one pass, one fail);
      `failing_slots()` returns exactly the failing one; `all_slot_details()`
      returns both
- [ ] unit: `WardError::Ood` `Display` output contains the string
      `"CALYX_GUARD_OOD"` and lists each failing slot's cos/tau values
- [ ] proptest: `SlotVerdict` serde round-trip; `cos` in `[-1.0, 1.0]` preserved
      to f32 precision
- [ ] edge: `GuardVerdict` with empty `per_slot` (no required slots) serializes
      cleanly; `failing_slots()` returns empty vec
- [ ] edge: `WardError::PolicyViolation { k: 5, n_required: 3 }` formats
      correctly including `CALYX_GUARD_POLICY_VIOLATION`
- [ ] fail-closed: `WardError::Provisional` display contains
      `"CALYX_GUARD_PROVISIONAL"` and the advice "calibrate before high-stakes use"

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** durable aiwonder evidence root
  `/home/croyse/calyx/data/fsv-issue259-ph37-t02-<date>/` containing
  `verdict.json`, `errors.json`, the captured cargo log, and SHA-256 manifest.
- **Readback:** read the JSON bytes with `xxd`, `sha256sum`, grep, and parsed
  JSON; stdout is only a captured artifact.
- **Prove:** durable `errors.json` contains `CALYX_GUARD_OOD`,
  `CALYX_GUARD_PROVISIONAL`, `CALYX_GUARD_MISSING_SLOT`, and
  `CALYX_GUARD_POLICY_VIOLATION`; `verdict.json` contains the full per-slot
  pass/fail breakdown.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH37 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
