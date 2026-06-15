# PH37 · T07 — `guard_query`: incoming-query OOD gate

| Field | Value |
|---|---|
| **Issue** | #275 |
| **Phase** | PH37 — Gτ Guard Math + GuardProfile |
| **Stage** | S8 — Ward Gτ Guard |
| **Crate** | `calyx-ward` |
| **Files** | `crates/calyx-ward/src/query.rs`, `crates/calyx-ward/tests/guard_query.rs` |
| **Axioms** | A3, A12 |
| **PRD** | `dbprdplans/09 §8` |

## Status

DONE / FSV-signed-off on aiwonder for #275. Implemented in commit
`8b71024896a8`. Evidence root:
`/home/croyse/calyx/data/fsv-issue275-ph37-t07-20260609-8b71024`.

Readback facts:
- `query-pass.json` shows an in-region query passes with nearest
  `01010101010101010101010101010101`, `gap=0.0`, and both required slots pass.
- `query-ood.json` shows an OOD query returns nearest
  `01010101010101010101010101010101`, `gap=0.099999964`, and both slots fail.
- `query-average-attack.json` shows the no-flatten proof for incoming queries:
  slot 1 passes at `cos=0.95`, slot 2 fails at `cos=0.45`, overall status is
  `ood`, and `gap=0.25`.
- `query-no-regions.json` fails closed as OOD with `nearest_cx=null`.
- `missing-slot-error.json` shows `CALYX_GUARD_MISSING_SLOT` for a missing
  required query slot.
- `source-readback.json` shows `line_count=111` and
  `aggregate_vector_gate_markers=[]`.
- #650 hardens trusted-query profile validation: empty required-slot profiles
  and `KofN { k: 0 }` now return `CALYX_GUARD_INERT_PROFILE` before region
  evaluation, including the no-trusted-regions edge.

## Goal

Gate incoming query vectors against trusted regions using the same per-slot Ward
predicate as `guard()`. The API is storage-agnostic:
`guard_query(profile, query_slots, trusted_regions) -> QueryVerdict`, where
`QueryVerdict` is `Pass { nearest_cx, gap, per_slot }` or
`Ood { nearest_cx, gap, per_slot, action }`.

## Done When

- [x] In-region query returns `Pass` and the expected nearest `CxId`.
- [x] OOD query returns `Ood` with nearest `CxId`, gap, per-slot verdicts, and
      `NoveltyAction`.
- [x] Average-pass / slot-fail incoming-query attack remains OOD.
- [x] No trusted regions returns OOD without inventing a nearest region.
- [x] Inert profiles fail closed before no-region OOD/pass selection.
- [x] Missing required query slot fails closed with `CALYX_GUARD_MISSING_SLOT`.
- [x] aiwonder cargo/check/clippy/test gates pass and `.rs` files remain
      <=500 lines.
