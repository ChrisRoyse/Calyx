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
| `crates/calyx-assay/src/ksg.rs` | `ksg_mi_continuous_discrete*` now uses a Ross-style mixed estimator: same-label kth continuous radius, all-sample continuous neighbor count, and class-size digamma correction |
| `crates/calyx-assay/tests/ksg_mixed_discrete_fsv.rs` | Added planted-signal, fail-closed small-class, and manual real-labeled dataset FSV coverage |

Behavior now enforced:

- Class labels are not converted to fake continuous one-hot vectors.
- Each point gets its kth-neighbor radius from same-class continuous samples.
- The estimator fails closed with `CALYX_ASSAY_INSUFFICIENT_SAMPLES` when any
  discrete label has `class_size <= k`.
- Grounded-anchor callers keep the public API and receive the same trust tag
  semantics as before.

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

The real-data readback shows why the fix matters: the old one-hot path reports a
point estimate above the 3-class entropy, while the Ross lower bound does not
clear entropy. Any sufficiency decision consuming this path is now grounded on
the mixed estimator, not on the over-optimistic one-hot value.

## Conclusion

#1207 is complete for the assay estimator boundary. Discrete anchors now use a
mixed continuous-discrete estimator, deficient classes fail closed with a named
label/class size, and both synthetic and real-labeled readbacks persist the
evidence for the estimator delta.
