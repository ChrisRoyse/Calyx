# PH38 · T06 — Sextant InRegionOnly guarded search

| Field | Value |
|---|---|
| **Phase** | PH38 — τ Calibration (Conformal) + Novelty → New Region |
| **Stage** | S8 — Ward Gτ Guard |
| **Crate** | `calyx-sextant` + `calyx-ward` |
| **Issue** | #276 |
| **Depends on** | PH37 guard math, PH38 calibrated `GuardProfile`, PH24 search hits |
| **PRD** | `09 §6`, `10 §1` |

**STATUS:** DONE / FSV-signed-off in #276. Implementation commit:
`c0b5d7f1c5348b8914f2b2aa41ce0061564096d6`. Evidence root:
`/home/croyse/calyx/data/fsv-issue276-ph38-t06-20260609-c0b5d7f`.

## Goal

When a Sextant `Query` carries `QueryGuard::InRegionOnly(GuardProfile)`, search
must call Ward over candidate hits and return only hits whose stored
constellation slot vectors pass the guard. OOD candidates are dropped with a
structured reason; surviving hits carry the full `GuardVerdict`.

## Build

- [x] Add `QueryGuard::InRegionOnly(GuardProfile)` with serde defaulting so old
      unguarded query JSON still deserializes.
- [x] Attach structured guard evidence to surviving `Hit` rows.
- [x] Add `GuardedSearchReport` and dropped-hit evidence for OOD and missing
      stored constellation cases.
- [x] Expand the candidate window before final `k` truncation so a top OOD hit
      cannot starve an in-region candidate behind it.
- [x] Keep guarded search dense-slot-only and fail closed with
      `CALYX_SEXTANT_VECTOR_SHAPE` for non-dense guarded query vectors.

## FSV

- **SoT:** durable aiwonder evidence root
  `/home/croyse/calyx/data/fsv-issue276-ph38-t06-<date>-<commit>/`.
- **Trigger:** run the ignored `calyx-sextant` guarded-search fixture with
  `CALYX_SEXTANT_PH38_T06_FSV_DIR` set to that root.
- **After-read:** inspect JSON bytes for before unguarded hits, after guarded
  hits, dropped guard hits, missing-doc edge, non-dense query error, and hashes.
- **Prove:** before set contains the OOD candidate; after set excludes it;
  surviving hit has `mode=in_region_only` and `overall_pass=true`; dropped
  evidence includes the OOD verdict and missing-constellation reason.

**Actual #276 readback:** before unguarded hits =
`02020202020202020202020202020202`, `01010101010101010101010101010101`,
`03030303030303030303030303030303`; after guarded hits =
`01010101010101010101010101010101`; dropped guard hits = OOD
`02020202020202020202020202020202` with `cos=0.0`, `tau=0.7`, `pass=false`,
plus missing-constellation `03030303030303030303030303030303`. Non-dense edge
returns `CALYX_SEXTANT_VECTOR_SHAPE`.

## Post-T06 Hardening

#356 is signed off at implementation commit
`cfea3acedd83390c48eba12d4104de6a982a6c2e`. `Query` now has optional
`guard_vectors: BTreeMap<SlotId, SlotVector>` for slot-aware produced vectors.
For multi-slot `QueryGuard::InRegionOnly`, Sextant requires a dense vector for
each required guard slot, uses the matching query vector per slot, and fails
closed with `CALYX_SEXTANT_VECTOR_SHAPE` if those vectors are absent. The
single-slot compatibility path still accepts the legacy dense `Query.vector`.

**Actual #356 readback:** evidence root
`/home/croyse/calyx/data/fsv-issue356-sextant-multislot-guard-20260609-cfea3ac`
contains before unguarded hits `04040404040404040404040404040404` and
`05050505050505050505050505050505`; after guarded hits keep only
`04040404040404040404040404040404`; dropped guard hits contain
`05050505050505050505050505050505` with slot 8 passing and slot 9 failing; the
missing-guard-vectors edge returns `CALYX_SEXTANT_VECTOR_SHAPE`.

**Supplemental #359 readback:** evidence root
`/home/croyse/calyx/data/fsv-issue359-sextant-guard-vector-readback-20260609-cf8d4b3`
adds the missing source bytes. `guard-query.json` contains `guard_vectors` keys
`8` and `9`; slot 8 is dense `[1.0, 0.0]`, slot 9 is dense
`[0.0, 1.0, 0.0]`. `candidate-slot-readback.json` contains both candidate rows:
`0404...` has matching slot 8 and slot 9 vectors, while `0505...` has the
style-slot mismatch `[1.0, 0.0, 0.0]`. Edge readback proves partial and sparse
slot-aware guard-vector maps return `CALYX_SEXTANT_VECTOR_SHAPE`.

## Done When

- [x] focused + workspace cargo gates pass on aiwonder
- [x] all `.rs` files remain <=500 lines
- [x] manual FSV before/trigger/after readback is attached to #276
- [x] manual #356 FSV proves slot-aware multi-slot guard vectors and fail-closed
      missing-vector behavior
- [x] manual #359 FSV reads back query guard-vector bytes, candidate slot-vector
      bytes, and missing/sparse slot-aware guard-vector edge errors
- [x] PH38/Stage 8 rollups and epic #257 point to the next active task
