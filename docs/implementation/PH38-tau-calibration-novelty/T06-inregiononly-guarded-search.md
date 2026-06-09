# PH38 · T06 — Sextant InRegionOnly guarded search

| Field | Value |
|---|---|
| **Phase** | PH38 — τ Calibration (Conformal) + Novelty → New Region |
| **Stage** | S8 — Ward Gτ Guard |
| **Crate** | `calyx-sextant` + `calyx-ward` |
| **Issue** | #276 |
| **Depends on** | PH37 guard math, PH38 calibrated `GuardProfile`, PH24 search hits |
| **PRD** | `09 §6`, `10 §1` |

**STATUS:** ACTIVE in #276 until aiwonder FSV is attached.

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

## Done When

- [ ] focused + workspace cargo gates pass on aiwonder
- [ ] all `.rs` files remain <=500 lines
- [ ] manual FSV before/trigger/after readback is attached to #276
- [ ] PH38/Stage 8 rollups and epic #257 point to the next active task
