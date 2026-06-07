# PH36 · T05 — `reproduce.rs`: re-run fusion + drift assertion + `ReproduceResult`

| Field | Value |
|---|---|
| **Phase** | PH36 — Merkle checkpoints + verify_chain + reproduce() |
| **Stage** | S7 — Ledger Provenance |
| **Crate** | `calyx-ledger` |
| **Files** | `crates/calyx-ledger/src/reproduce.rs` (≤500) |
| **Depends on** | T04 (this phase) · PH24 (RRF/WeightedRRF fusion) · PH35 (`Answer` entry payload) |
| **Axioms** | A15, A16 |
| **PRD** | `dbprdplans/11 §3`, `11 §5` |

## Goal

Complete `reproduce(answer_id)` by re-running the recorded fusion (using the
recorded fusion weights from the `Answer` ledger entry), re-asserting the
resulting hit set against the original, and returning a structured
`ReproduceResult` that proves whether the answer was measured or fabricated.
`max_drift ≤ 1e-3` is the pass criterion (bit-parity within tolerance from
Forge determinism mode). This is the honesty gate for every claim Calyx makes.

## Build (checklist of concrete, code-level steps)

- [ ] `pub struct ReproduceResult { reproduced: bool, max_drift: f64, original_hits: Vec<HitRef>, reproduced_hits: Vec<HitRef> }`
  where `HitRef = { cx_id: CxId, score: f32 }`.
- [ ] `fn rerun_fusion(remeasured: &[RemeasuredSlot], fusion_weights: &FusionWeights) -> Result<Vec<HitRef>>` —
  applies the recorded `FusionWeights` (RRF/WeightedRRF parameters from the
  `Answer` ledger entry payload) to the re-measured slot vectors; returns ranked
  hits.
- [ ] `fn assert_within_tolerance(original: &[HitRef], reproduced: &[HitRef], tol: f64) -> (bool, f64)` —
  computes element-wise score diff for matched `cx_id`s; returns `(all_within_tol, max_drift)`.
  `tol = 1e-3` (hard-coded constant, matches Forge bit-parity contract).
- [ ] `pub fn reproduce(cf_reader, registry, forge, answer_id) -> Result<ReproduceResult>` —
  calls `build_reproduce_context` → `remeasure_slots` → `rerun_fusion` →
  `assert_within_tolerance`; sets `reproduced = max_drift <= 1e-3`.
- [ ] `CALYX_REPRODUCE_DRIFT_EXCEEDED` added to error catalog (NOT returned by
  `reproduce` — `reproduce` returns `Ok(ReproduceResult { reproduced: false, … })`
  so the caller decides; but the code must exist for explicit assertion use).
  Remediation: `"reproduce max_drift exceeded 1e-3 — possible lens drift or fusion parameter change"`.
- [ ] Write a ledger entry for the reproduce call itself: `kind=Answer` (or
  add a `Reproduce` variant — if added, add `Reproduce` to the `EntryKind` enum
  and update the wire code table; alternatively use `kind=Admin` with a
  `"reproduce_v1"` payload tag to avoid extending the enum — pick one and be
  consistent). Record `{ answer_id, reproduced, max_drift, ts }`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: construct `original_hits = [(cx1, 0.9), (cx2, 0.7)]` and
  `reproduced_hits = [(cx1, 0.9005), (cx2, 0.7002)]`; `assert_within_tolerance`
  → `(true, 0.0005)` — within 1e-3.
- [ ] unit: `reproduced_hits = [(cx1, 0.9015), (cx2, 0.7)]` → `(false, 0.0015)` —
  exceeds 1e-3.
- [ ] unit: full `reproduce` end-to-end with a synthetic answer entry + mock
  registry + mock forge → `ReproduceResult { reproduced: true, max_drift: <1e-3 }`.
- [ ] edge (≥3): original and reproduced hit sets have different cardinality
  (a cx_id appears in one but not the other) → `max_drift = 1.0` (full miss,
  `reproduced = false`); empty hit set → `(true, 0.0)`; single hit, perfect
  match → `(true, 0.0)`.
- [ ] fail-closed: fusion weights absent from ledger entry → reproduce returns
  `Err(CALYX_LEDGER_CORRUPT)` (missing required field, not a silent zero-weight
  fusion); `remeasure_slots` returns error → propagated, no partial result.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `ReproduceResult` printed to stdout + ledger CF row for the
  reproduce call on aiwonder
- **Readback:**
  1. `calyx reproduce --vault test --answer <answer_id>` →
     prints `{ "reproduced": true, "max_drift": 0.000XYZ }` where
     `max_drift ≤ 1e-3`.
  2. `calyx scan --cf ledger | jq 'select(.payload.type=="reproduce_v1")' | tail -1` →
     confirms a reproduce ledger entry was written with the same `max_drift`.
  3. Read both original and reproduced score vectors via `xxd` and compute
     max element-wise diff manually to confirm ≤ 1e-3.
- **Prove:** bit-parity ≤ 1e-3 confirmed from raw bytes; `reproduced=true`
  printed; reproduce ledger entry present in CF.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] CPU↔GPU bit-parity ≤ 1e-3 on the golden reproduce set (Forge determinism mode)
- [ ] FSV evidence (readback output / screenshot) attached to the PH36 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
