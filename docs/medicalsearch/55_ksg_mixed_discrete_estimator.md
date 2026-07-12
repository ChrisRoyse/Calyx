# 55 - #1207 Mixed continuous-discrete KSG estimator

## Scope

#1207 replaces the biased discrete-anchor path that one-hot encoded class
labels and fed them into the continuous KSG estimator. Discrete anchors are on
the biomedical discovery trust path: sufficiency is only as honest as
`I(panel; anchor)`.

This is an estimator/trust-boundary fix. It does not establish biomedical
efficacy, safety, clinical actionability, or a cure.

## Code Change

Changed files:

| File | Change |
|---|---|
| `crates/calyx-assay/src/ksg/` | `ksg_mi_continuous_discrete*` uses a Ross-style mixed estimator; #1380 added explicit zero-radius refusal and a class-support-aware no-replacement root interval |
| `crates/calyx-assay/tests/ksg_mixed_discrete_fsv.rs` | Added planted-signal, fail-closed small-class, and manual real-labeled dataset FSV coverage |

Behavior now enforced:

- Class labels are not converted to fake continuous one-hot vectors.
- Each point gets its kth-neighbor radius from same-class continuous samples.
  At a positive radius, `q_i` counts every other same-class row on or inside
  that boundary, and the local term uses `psi(q_i)` rather than fixed `psi(k)`.
- The estimator fails closed with `CALYX_ASSAY_INSUFFICIENT_SAMPLES` when any
  discrete label has `class_size <= k`.
- The estimator fails closed with `CALYX_ASSAY_DEGENERATE_INPUT` when a point
  has at least `k` exact same-class continuous duplicates, rather than
  continuing with an undefined zero-radius estimate.
- Finite coordinates whose per-dimension `max-min` span cannot produce finite
  f32 Chebyshev distances fail the same degeneracy code before estimation.
- Each interval proposal is uniform SRSWOR. Rejection sampling accepts exactly
  999 support-valid roots, with at most eight proposals per root; accepted roots
  are uniform conditional on class-support validity, not unconditional uniform
  SRSWOR draws. The exact hypergeometric class-support admission bound and
  finite-population correction remain part of the contract. The interval is
  `EstimateBound::Point`, not a calibrated sufficiency lower bound.
- No feasible subsample plan or eight exhausted support-validity proposals for
  any root fails `CALYX_ASSAY_INSUFFICIENT_SAMPLES`. A raw interval wholly below
  the non-negative MI domain fails `CALYX_ASSAY_LOW_SIGNAL`.
- Grounded-anchor callers keep the public API and receive the same trust tag
  semantics as before.

The bound is preserved through `SlotAssayBits`,
`ExternalSignalAdmissionReport`, and `LensCandidateMeasurement` and is included
in versioned provenance/decision hashes. Missing legacy metadata remains unknown
`None`. Mixed `Point` evidence is refused by external admission as
`CALYX_POLY_EXTERNAL_SIGNAL_REFUSED_UNCALIBRATED_BOUND` and by lens autobuild as
`uncalibrated_estimate_bound`; only `LowerBound` may clear those gates. A
four-seed balanced/imbalanced null-and-disjoint-support regression is a small
repeated-dataset smoke audit, not a 95% coverage claim.

Primary method references used for the estimator:

- Ross, "Mutual Information between Discrete and Continuous Data Sets", PLoS
  One 9(2):e87357 (2014): https://journals.plos.org/plosone/article/file?id=10.1371/journal.pone.0087357&type=printable
- Gao et al., "Estimating Mutual Information for Discrete-Continuous Mixtures",
  NeurIPS 2017: https://papers.neurips.cc/paper_files/paper/2017/file/ef72d53990bc4805684c9b61fa64a102-Paper.pdf

## Verification

FSV root:

```text
C:\code\Calyx-Dev\target\fsv\issue1207-ksg-mixed-discrete-20260704T042854Z
```

Commands run:

```powershell
cargo test -p calyx-assay --test ksg_mixed_discrete_fsv ksg_mixed_discrete -- --nocapture

$env:CALYX_FSV_ROOT='C:\code\Calyx-Dev\target\fsv\issue1207-ksg-mixed-discrete-20260704T042854Z'
cargo test -p calyx-assay --test ksg_mixed_discrete_fsv -- --nocapture

$env:CALYX_STAGE5_CLASSIFICATION_CSV='C:\code\Calyx-Dev\target\fsv\issue1207-real-dataset\iris.data'
$env:CALYX_FSV_ROOT='C:\code\Calyx-Dev\target\fsv\issue1207-ksg-mixed-discrete-20260704T042854Z'
cargo test -p calyx-assay --test ksg_mixed_discrete_fsv ksg_mixed_discrete_real_labeled_dataset_delta_fsv -- --ignored --nocapture
```

Synthetic planted-signal artifact:

```text
C:\code\Calyx-Dev\target\fsv\issue1207-ksg-mixed-discrete-20260704T042854Z\issue1207-ksg-mixed-discrete\ross-mixed-discrete-readback.json
```

Readback summary:

```json
{
  "expected_entropy_bits": 1.5849625,
  "ross_mixed_small_scale_bits": 1.5909938,
  "ross_mixed_large_scale_bits": 1.5909938,
  "ross_scale_delta_abs": 0.0,
  "old_one_hot_small_scale_bits": 1.5909938,
  "old_one_hot_large_scale_bits": 0.0,
  "old_one_hot_scale_drop_bits": 1.5909938,
  "small_class_error": "CALYX_ASSAY_INSUFFICIENT_SAMPLES"
}
```

Real labeled dataset FSV:

```text
C:\code\Calyx-Dev\target\fsv\issue1207-ksg-mixed-discrete-20260704T042854Z\issue1207-real-labeled-ross-delta-readback.json
```

Source and CF readback:

```json
{
  "dataset": "UCI Iris",
  "dataset_blake3": "8578940c6c00041901b00392034412b20e2f574eba595bcc6b979ca4148178e6",
  "rows": 150,
  "anchor_entropy_bits": 1.5849626,
  "ross_mixed_bits": 1.1287328,
  "ross_mixed_ci_low": 0.8621819,
  "ross_trust": "trusted",
  "old_one_hot_bits": 1.8947722,
  "abs_delta_bits": 0.7660394,
  "ross_ci_low_clears_entropy": false,
  "persisted_assay_rows": 1,
  "loaded_assay_rows": 1
}
```

The historical real-data readback shows why the estimator replacement mattered:
the old one-hot path reported a point estimate above the 3-class entropy, while
the Ross point estimate did not. Its recorded historical CI predates the #1380
root-interval hardening and must not be interpreted as a calibrated lower bound.
Current callers receive an explicit `Point` interval from the mixed estimator.

## Conclusion

#1207 replaced the one-hot estimator boundary; #1380 subsequently hardened it.
Discrete anchors now use a mixed continuous-discrete estimator, deficient
classes and zero radii fail closed, and the no-replacement uncertainty output is
honestly tagged `Point`.
