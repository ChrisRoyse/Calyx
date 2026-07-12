# 56 - #1208 KSG no-replacement subsample CI

## Scope

#1208 removes ordinary with-replacement bootstrap from the continuous KSG
confidence interval path. Replacement bootstrap creates duplicate rows, and KSG
interprets duplicate coordinates as fine-scale structure. That is unsafe for a
lower-bound sufficiency gate.

This is an estimator/trust-boundary fix. It does not establish biomedical
efficacy, safety, clinical actionability, or a cure.

## Code Change

Changed file:

| File | Change |
|---|---|
| `crates/calyx-assay/src/ksg/mod.rs` | Continuous KSG computes CI from deterministic m-out-of-n no-replacement subsamples instead of paired replacement bootstrap |

Behavior now enforced:

- Continuous KSG CI draws distinct row indices for every resample.
- The duplicate-index invariant is checked at runtime and fails closed if it is
  ever violated.
- Subsample size is `floor(4n/5)`.
- The interval transforms empirical 2.5%/97.5% subsample percentiles as
  `point + (percentile - point) * sqrt(m/n)`.
- If the subsample cannot still satisfy the assay quorum
  (`m >= MIN_ASSAY_SAMPLES` and `0 < k < m`), KSG returns
  `CALYX_ASSAY_INSUFFICIENT_SAMPLES`.
- The generic paired bootstrap helper remains available for non-KSG estimators.
- The mixed continuous-discrete path is separate in `ksg/mixed_ci.rs`. Each
  proposal is uniform SRSWOR; rejection sampling accepts exactly 999
  support-valid roots with at most eight proposals per root. Accepted roots are
  therefore uniform conditional on class-support validity, not unconditional
  uniform SRSWOR draws. The exact hypergeometric class-support bound rejects
  rather than forcing class quotas. No feasible plan or exhausted root fails
  `CALYX_ASSAY_INSUFFICIENT_SAMPLES`; a wholly negative interval fails
  `CALYX_ASSAY_LOW_SIGNAL`. The result is explicit `EstimateBound::Point` and
  does not reuse this continuous-path interval.

The continuous path still inherits the default `EstimateBound::LowerBound`
metadata even though this scaled-percentile interval is not calibrated as a
one-sided lower bound; #1449 tracks that separate contract gap.

Both KSG paths reject finite input coordinates whose per-dimension `max-min`
span cannot produce finite f32 Chebyshev distances. Mixed bound metadata is
preserved through `SlotAssayBits`, `ExternalSignalAdmissionReport`, and
`LensCandidateMeasurement` and participates in versioned hashes; missing legacy
metadata remains unknown `None`. External admission and lens autobuild refuse
mixed `Point` evidence, and only `LowerBound` may clear those gates. A four-seed
balanced/imbalanced repeated-dataset regression is a smoke audit, not a 95%
coverage claim.

Primary method reference used:

- Holmes and Nemenman, "Estimation of mutual information for real-valued data
  with error bars and controlled bias" (2019):
  https://arxiv.org/pdf/1903.09280

The paper explicitly identifies replacement-bootstrap duplicates as KSG-visible
fine-scale/high-information artifacts and recommends subsampling rather than
ordinary bootstrap for KSG error bars.

## Verification

The readbacks below are the historical #1208 evidence captured on 2026-07-04.
They predate the current scaled-percentile implementation and the separate #1380
mixed root interval, so their numeric bounds are not current calibration
evidence.

FSV root:

```text
C:\code\Calyx-Dev\target\fsv\issue1208-ksg-subsample-ci-20260704T044500Z
```

Command:

```powershell
$env:CALYX_FSV_ROOT='C:\code\Calyx-Dev\target\fsv\issue1208-ksg-subsample-ci-20260704T044500Z'
cargo test -p calyx-assay ksg_no_replacement -- --nocapture
```

FSV artifact:

```text
C:\code\Calyx-Dev\target\fsv\issue1208-ksg-subsample-ci-20260704T044500Z\issue1208-ksg-subsample-ci\ksg-subsample-ci-readback.json
```

Readback summary:

```json
{
  "independent_true_mi_bits": 0.0,
  "independent_point_bits": 0.02191284,
  "old_with_replacement_ci_low": 0.0,
  "old_with_replacement_ci_high": 2.1993752,
  "new_no_replacement_ci_low": 0.0,
  "new_no_replacement_ci_high": 0.2483080,
  "old_mean_duplicates_per_resample": 57.96,
  "old_max_duplicates_per_resample": 64,
  "new_no_replacement_duplicate_free": true,
  "subsample_m": 128,
  "small_sample_error": "CALYX_ASSAY_INSUFFICIENT_SAMPLES"
}
```

Note: the repository's existing widened CI calculation already clamps the
independent-control old lower bound to zero in this fixture. The replacement
pathology is still physically visible in the resample interval: the old
replacement-bootstrap high bound is about 9x wider than the no-replacement
bound, and the old resamples average about 58 duplicate draws per 160-row
resample. The new path removes the duplicate source and keeps the independent
lower bound at zero.

Planted-signal coverage readback:

```json
{
  "samples": 180,
  "point_bits": 1.8794093,
  "known_gaussian_bits": 1.9886955,
  "covered_seed_count": 5,
  "seed_count": 5
}
```

Edge case:

```json
{
  "case": "n_just_above_min_but_subsample_below_min",
  "before": { "n": 60, "k": 3, "subsample_m": 48 },
  "after": {
    "error": "CALYX_ASSAY_INSUFFICIENT_SAMPLES"
  }
}
```

## Conclusion

#1208 removed replacement duplicates from the continuous KSG CI path. Its
historical fixtures showed the expected behavior at that revision. The current
continuous scaled-percentile interval remains no-replacement but is not a
calibrated lower bound (#1449); the current mixed path is the separately
fail-closed `Point` interval described above.
