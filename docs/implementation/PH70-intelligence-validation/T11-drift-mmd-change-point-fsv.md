# PH70 T11 - Drift / MMD Change-Point FSV

| Field | Value |
|---|---|
| Phase | PH70 - Intelligence validation on real corpora |
| Issue | #609 |
| Crate | `calyx-assay` |
| Files | `crates/calyx-assay/src/mmd.rs`, `crates/calyx-assay/tests/drift_mmd_fsv.rs`, `scripts/acquire_drift_pair_issue609.sh` |
| PRD | `docs/dbprdplans/28_FSV_AND_TEST_DATA.md` row 12, `docs/dbprdplans/26_ADVANCED_MATH_FRONTIERS.md` section 7, `docs/dbprdplans/17_JOHARI_BLINDSPOTS.md` section 8 |

## Purpose

PH69 row 12 requires a labeled drift pair: month-A vs month-B distributions. PH70 must prove the drift capability by reading a persisted MMD/change-point report, not by trusting a harness return.

## Implementation

`calyx-assay::mmd` provides:

- Gaussian-kernel MMD two-sample reports with seeded permutation p-values.
- Median-distance bandwidth selection with fail-closed zero-signal behavior.
- Ordered-stream change-point scan that reruns the MMD significance test at the winning split.
- Exact `CALYX_ASSAY_*` error codes for empty input, below-min samples, dimension mismatch, non-finite values, invalid config, and zero-signal inputs.

`scripts/acquire_drift_pair_issue609.sh` materializes a deterministic AG News-derived drift pair under `/zfs/archive/calyx/datasets/drift_pair` on aiwonder. Month labels are deterministic split labels over real AG News text rows; features are text-derived lexical counts, not class one-hot labels.

## FSV Evidence

The ignored PH70 FSV test reads:

- `month_a.tsv`
- `month_b.tsv`
- `month_a_control.tsv`
- `manifest.json`

It writes:

- `ph70_drift_mmd.json`
- `ph70_drift_mmd_edges.json`

The required evidence is:

- month-A vs month-B: significant MMD shift.
- month-A vs month-A-control: non-significant no-shift control.
- concatenated month-A/month-B stream: change-point near the known split boundary.
- edge readbacks: empty input, below-min samples, dimension mismatch, NaN/non-finite value, zero-signal input, and no-shift control.
