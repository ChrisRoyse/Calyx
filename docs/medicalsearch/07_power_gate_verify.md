# 07 - Assay power-calibration gate verification

- **Issue:** #874 (epic #867)   **Date (UTC):** 2026-06-25   **FSV host:** aiwonder
- **Goal:** verify the Assay MI power-calibration gate is active so an underpowered estimator cannot be treated as a grounded/sufficient verdict, and verify the target-entropy floor fails closed.

## What was run (exact commands)

FSV root:

```
/home/croyse/calyx/fsv/issue874-power-gate-20260625T090958Z
```

Commands run on aiwonder from `/home/croyse/calyx/repo` with `CALYX_FSV_ROOT` set to the root above:

```
cargo fmt --all -- --check
git diff --check
bash scripts/linecount.sh
cargo test -p calyx-assay --test power_gate_fsv -- --nocapture
cargo test -p calyx-cli assay_bits_validation::power_gate_tests -- --nocapture
cargo build -p calyx-cli
./target/debug/calyx assay bits-validate --corpus-dir <FSV_ROOT>/success_corpus --metrics-dir <FSV_ROOT>/success_metrics --cf-root <FSV_ROOT>/success_cf --target-class 0 --domain issue874_power_success
./target/debug/calyx assay bits-validate --corpus-dir <FSV_ROOT>/entropy_floor_corpus --metrics-dir <FSV_ROOT>/entropy_metrics --cf-root <FSV_ROOT>/entropy_cf --target-class 0 --domain issue874_entropy_floor
```

## Raw evidence / FSV

Code and state:

- aiwonder repo head before FSV: `d04a139189b8641929af7dcff25e52d8e24776d3` plus the uncommitted #874 patch.
- Remote worktree status count before/after FSV: `3` changed files (the #874 patch only).
- `cargo fmt --all -- --check`: status `0`.
- `git diff --check`: status `0`.
- `bash scripts/linecount.sh`: status `0`.
- `cargo test -p calyx-assay --test power_gate_fsv -- --nocapture`: status `0`.
- `cargo test -p calyx-cli assay_bits_validation::power_gate_tests -- --nocapture`: status `0`.
- `cargo build -p calyx-cli`: status `0`.

Direct estimator/sufficiency gate artifact:

```
/home/croyse/calyx/fsv/issue874-power-gate-20260625T090958Z/issue874_power_gate_readback.json
bytes 614
sha256 07dafc5fcbd3c7ccf05fd88fc5a6799ff85f0b694fb29cc8f5f89960371b4cc5
schema calyx-assay-power-gate-fsv-v1
```

Readback fields:

- Entropy-floor labels: `total=200`, `positives=1`, `negatives=199`.
- Entropy-floor error code: `CALYX_ASSAY_DEGENERATE_TARGET_ENTROPY`.
- Deliberately underpowered case: `n_samples=64`, `n_features=4096`, `recovery_ratio=0.25`.
- Underpowered sufficiency error code: `CALYX_ASSAY_ESTIMATOR_UNDERPOWERED`.
- Missing-calibration sufficiency error code: `CALYX_ASSAY_ESTIMATOR_UNDERPOWERED`.
- Passing calibration control: `sufficient=true`.

Real `calyx assay bits-validate` success case:

- Input corpus bytes:
  - `success_corpus/vectors.jsonl`: `102890` bytes, sha256 `d58ff4da3632217c0906fbd5a98c93fbe0275d98116d10a0bce408e2d64e5655`.
  - `success_corpus/manifest.json`: `379` bytes, sha256 `f812dbb6e1a633267feae00392ac1552c5aabad3d86b7ef4a288ce6f292b4b94`.
- Command status: `0`.
- Stdout artifact: `4304` bytes, sha256 `e9b1dd2892ba9cd935d991039823982c2589f0c468de00ba2b3eed68141dc24a`.
- Assay CF rows persisted: `3`.
- Assay CF rows read back after reopen: `3`.
- Anchor entropy: `1.000000` bits.
- Panel power status: `passed`.
- Panel power recovery: `1.000000`.
- Lens power statuses: `real_a:passed`, `real_b:passed`, `redundant:passed`.
- Redundant lens rejection: `redundant:CALYX_ASSAY_REDUNDANT`.
- Abundance artifact:
  - bytes `3393`
  - sha256 `c07b6efb71698b3cd52404d63279e4b2a1419570d27e1dca76e7b7e3768e2d15`
  - readback rows `3`
  - readback panel power status `passed`
- Stderr bytes: `0`.

Real `calyx assay bits-validate` entropy-floor refusal:

- Input corpus bytes:
  - `entropy_floor_corpus/vectors.jsonl`: `17289` bytes, sha256 `1756c1a4235f78f955b98f42f5a5a8f2591176b07ba8a564f872cf402be8ecf4`.
  - `entropy_floor_corpus/manifest.json`: `254` bytes, sha256 `dca21bd1c31a8b85a6485cfa9fc0acd6a70ba97bf06e66dd043546c9de862e25`.
- Command status: `2` (expected refusal).
- Stdout bytes: `0`.
- Stderr bytes: `219`, sha256 `4e5d8d89ffe04cba5b23d8898eebcfdb8b8866b928b3da5e662d1c697232e2fc`.
- Error code present in stderr: `CALYX_ASSAY_DEGENERATE_TARGET_ENTROPY`.
- Metrics directory exists after refusal: `false`.
- CF directory exists after refusal: `false`.

## Findings (honest)

- The estimator power gate is active at the Assay API boundary: an underpowered `(n=64, dim=4096)` calibration with only `0.25` recovery is rejected by `PowerCalibration::require_passed()` and cannot pass through `panel_sufficiency_from_estimate()`.
- Missing power calibration also fails closed with `CALYX_ASSAY_ESTIMATOR_UNDERPOWERED`; sufficiency cannot be claimed from an uncalibrated MI estimate.
- A passing calibration control does produce `sufficient=true`, proving the test is not just rejecting all inputs.
- The real CLI `bits-validate` path rejects a low-entropy target with `CALYX_ASSAY_DEGENERATE_TARGET_ENTROPY` before writing metrics or Assay CF state.
- The real CLI `bits-validate` success path persisted and reloaded `3` Assay CF rows, and its metrics artifact separately read back `panel.power_calibration_status=passed`.
- Scope note: for valid non-empty vectors, `bits-validate` plants a strong binary signal in the last feature column, so the deliberately underpowered `(n, dim)` proof is exercised at the public Assay/sufficiency boundary rather than by corrupting a valid corpus into a different error class.

## Conclusion & next step

#874 acceptance is met by aiwonder FSV: the underpowered estimator code path returns `CALYX_ASSAY_ESTIMATOR_UNDERPOWERED`, the entropy floor returns `CALYX_ASSAY_DEGENERATE_TARGET_ENTROPY`, and successful calibrated measurements persist/read back Assay CF rows. This is a gate verification only; it produces no grounded biomedical discovery claim.
