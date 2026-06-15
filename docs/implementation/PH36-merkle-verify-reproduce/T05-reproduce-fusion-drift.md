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

- [x] `pub struct ReproduceResult { reproduced: bool, max_drift: f64, original_hits: Vec<HitRef>, reproduced_hits: Vec<HitRef> }`
  where `HitRef = { cx_id: CxId, score: f32 }`.
- [x] `fn rerun_fusion(remeasured: &[RemeasuredSlot], fusion_weights: &FusionWeights) -> Result<Vec<HitRef>>` —
  applies the recorded `FusionWeights` (RRF/WeightedRRF parameters from the
  `Answer` ledger entry payload) to the re-measured slot vectors; returns ranked
  hits.
- [x] `fn assert_within_tolerance(original: &[HitRef], reproduced: &[HitRef], tol: f64) -> (bool, f64)` —
  computes element-wise score diff for matched `cx_id`s; returns `(all_within_tol, max_drift)`.
  `tol = 1e-3` (hard-coded constant, matches Forge bit-parity contract).
- [x] `pub fn reproduce(cf_reader, registry, forge, answer_id) -> Result<ReproduceResult>` —
  calls `build_reproduce_context` → `remeasure_slots` → `rerun_fusion` →
  `assert_within_tolerance`; sets `reproduced = max_drift <= 1e-3`.
- [x] `CALYX_REPRODUCE_DRIFT_EXCEEDED` added to error catalog (NOT returned by
  `reproduce` — `reproduce` returns `Ok(ReproduceResult { reproduced: false, … })`
  so the caller decides; but the code must exist for explicit assertion use).
  Remediation: `"reproduce max_drift exceeded 1e-3 — possible lens drift or fusion parameter change"`.
- [x] Write a ledger entry for the reproduce call itself: `kind=Answer` (or
  add a `Reproduce` variant — if added, add `Reproduce` to the `EntryKind` enum
  and update the wire code table; alternatively use `kind=Admin` with a
  `"reproduce_v1"` payload tag to avoid extending the enum — pick one and be
  consistent). Record `{ answer_id, reproduced, max_drift, ts }`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: construct `original_hits = [(cx1, 0.9), (cx2, 0.7)]` and
  `reproduced_hits = [(cx1, 0.9005), (cx2, 0.7002)]`; `assert_within_tolerance`
  → `(true, 0.0005)` — within 1e-3.
- [x] unit: `reproduced_hits = [(cx1, 0.9015), (cx2, 0.7)]` → `(false, 0.0015)` —
  exceeds 1e-3.
- [x] unit: full `reproduce` end-to-end with a synthetic answer entry + mock
  registry + mock forge → `ReproduceResult { reproduced: true, max_drift: <1e-3 }`.
- [x] edge (≥3): original and reproduced hit sets have different cardinality
  (a cx_id appears in one but not the other) → `max_drift = 1.0` (full miss,
  `reproduced = false`); empty hit set → `(true, 0.0)`; single hit, perfect
  match → `(true, 0.0)`.
- [x] fail-closed: fusion weights absent from ledger entry → reproduce returns
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

## Implementation Evidence

Status: DONE for #253.

- API implemented in `crates/calyx-ledger/src/reproduce.rs` + `src/reproduce/fusion.rs`.
- Public exports added in `crates/calyx-ledger/src/lib.rs`.
- Error catalog now includes `CALYX_REPRODUCE_DRIFT_EXCEEDED`.
- Reproduce ledger row uses `EntryKind::Admin` with payload `"type":"reproduce_v1"`.
- CLI surfacing remains outside this card; T05 proves the public ledger API and disk ledger CF bytes directly.

## aiwonder FSV Evidence

- Root: `/home/croyse/calyx/data/fsv-issue253-reproduce-fusion-20260609`
- Readback JSON: `reproduce-fusion-readback.json`
- Readback SHA-256: `97dd9a65f4b1c4421b437247b1b2fb89d99975eae720be4521615713702bd994`
- Happy path ledger rows: before `0`, before reproduce `3`, after reproduce `4`.
- Happy path result: `reproduced=true`, `max_drift=0.0`, `chain.status=intact`, `forge_seeds=[101,102]`.
- Happy path Admin row: `happy-ledger-cf/0000000000000003.ledger`, SHA-256 `511aa792814bec2415851fcbd5e6aa7bdefc3e4761533cceab887c43a1d7a662`, payload tag `reproduce_v1`.
- Drift edge: wrote a reproduce row with `reproduced=false`, `max_drift=0.009999999776482582`, and `assert_error=CALYX_REPRODUCE_DRIFT_EXCEEDED`.
- Missing `fusion_weights` edge: `CALYX_LEDGER_CORRUPT`, rows stayed `3 -> 3`.
- Re-measure/frozen-weight edge: `CALYX_LENS_FROZEN_VIOLATION`, rows stayed `3 -> 3`.
- Raw row readback included `xxd -p` of the happy Admin row; the encoded payload contains matching original/reproduced hit scores.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] CPU↔GPU bit-parity ≤ 1e-3 on the golden reproduce set (Forge determinism mode)
- [x] FSV evidence (readback output / screenshot) attached to the PH36 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
