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
| `crates/calyx-assay/src/ksg.rs` | Continuous KSG now computes CI from deterministic m-out-of-n no-replacement subsamples instead of paired replacement bootstrap |

Behavior now enforced:

- Continuous KSG CI draws distinct row indices for every resample.
- The duplicate-index invariant is checked at runtime and fails closed if it is
  ever violated.
- Subsample size is `floor(4n/5)`.
- The interval is widened by a deterministic coarse-grain allowance
  `abs(point_bits) * (1 - m/n)` so the reduced-size subsample does not become an
  over-tight safety bound.
- If the subsample cannot still satisfy the assay quorum
  (`m >= MIN_ASSAY_SAMPLES` and `0 < k < m`), KSG returns
  `CALYX_ASSAY_INSUFFICIENT_SAMPLES`.
- The generic paired bootstrap helper remains available for non-KSG estimators.
- The #1207 mixed continuous-discrete path remains on its local-term CI; it does
  not reintroduce replacement-bootstrap duplicate points.

Primary method reference used:

- Holmes and Nemenman, "Estimation of mutual information for real-valued data
  with error bars and controlled bias" (2019):
  https://arxiv.org/pdf/1903.09280

The paper explicitly identifies replacement-bootstrap duplicates as KSG-visible
fine-scale/high-information artifacts and recommends subsampling rather than
ordinary bootstrap for KSG error bars.

## Verification

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

#1208 is complete for the continuous KSG CI path. The CI no longer creates
duplicate rows, the duplicate invariant is asserted, planted-signal intervals
cover the known value on the tested seeds, and small subsamples fail closed
instead of emitting a bound from a degenerate resample.
