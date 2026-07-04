# 57 - #1209 Blind-spot calibration

## Scope

#1209 replaces the blind-spot sweep's hardcoded absolute delta gate with a
per-lens-pair empirical calibration. Blind-spot candidates seed downstream
biomedical hypothesis hunts, so the novelty signal must be comparable across
heterogeneous lens-pair similarity scales.

This is a ranking/trust-boundary fix. It does not establish biomedical
efficacy, safety, clinical actionability, or a cure.

## Code Change

Changed files:

| File | Change |
|---|---|
| `crates/calyx-loom/src/blind_spot.rs` | Added `BlindSpotCalibration`, calibrated alert evidence, and `detect_blind_spot_calibrated` |
| `crates/calyx-loom/src/error.rs` | Added `CALYX_LOOM_UNCALIBRATED_BLINDSPOT` |
| `crates/calyx-lodestar/src/blind_spot_sweep.rs` | Sweep now builds per-pair empirical delta distributions and skips uncalibrated pairs |
| `crates/calyx-loom/tests/blind_spot_calibration_fsv.rs` | Direct scale-normalization, null-FDR, and under-sampled edge FSV |
| `crates/calyx-lodestar/tests/issue875_blind_spot_sweep_tests.rs` | Sweep FSV updated to calibrated evidence |

Behavior now enforced:

- Alerts are emitted by one-sided empirical `p_value <= alpha`, not by
  `delta >= 0.5`.
- Alert evidence carries `sample_count`, `threshold_delta`, `percentile`,
  `p_value`, `alpha`, and scale-free `score`.
- Sweep ranking uses calibrated score when present.
- Pair calibration defaults to `min_samples=50`, `alpha=0.05`.
- Under-sampled pairs skip with explicit uncalibrated accounting instead of
  applying the old global threshold.

## Verification

FSV root:

```text
C:\code\Calyx-Dev\target\fsv\issue1209-blind-spot-calibration-20260704T045900Z
```

Commands:

```powershell
cargo test -p calyx-loom --test blind_spot_calibration_fsv -- --nocapture

$env:CALYX_FSV_ROOT='C:\code\Calyx-Dev\target\fsv\issue1209-blind-spot-calibration-20260704T045900Z'
cargo test -p calyx-loom --test blind_spot_calibration_fsv -- --nocapture

$env:CALYX_FSV_ROOT='C:\code\Calyx-Dev\target\fsv\issue1209-blind-spot-calibration-20260704T045900Z'
cargo test -p calyx-lodestar --test issue875_blind_spot_sweep_tests writes_fsv_readback_when_root_is_set -- --nocapture

cargo test -p calyx-lodestar --test issue875_blind_spot_sweep_tests -- --nocapture
cargo check -p calyx-loom
cargo check -p calyx-lodestar
cargo check -p calyx-cli
bash scripts/linecount.sh
git diff --check
```

Direct Loom calibration artifact:

```text
C:\code\Calyx-Dev\target\fsv\issue1209-blind-spot-calibration-20260704T045900Z\issue1209_blind_spot_calibration_readback.json
SHA256 680CA10E50CEA928D2E3B107987AA483166A659F7A4BE84FAE5403D8BD1E7DE5
```

Readback summary:

```json
{
  "params": { "min_samples": 50, "alpha": 0.05 },
  "compressed_delta": 0.18,
  "wide_delta": 0.98,
  "compressed_percentile": 1.0,
  "wide_percentile": 1.0,
  "compressed_p_value": 0.016666668,
  "wide_p_value": 0.016666668,
  "legacy_compressed_alert": false,
  "legacy_wide_alert": true,
  "null_alert_count": 3,
  "null_sample_count": 60,
  "under_sampled_error": "CALYX_LOOM_UNCALIBRATED_BLINDSPOT"
}
```

Calibrated Lodestar sweep artifact:

```text
C:\code\Calyx-Dev\target\fsv\issue1209-blind-spot-calibration-20260704T045900Z\issue875_blind_spot_sweep_readback.json
SHA256 D61031DE680D87FB5AD390A6E52EAB09CD621817BF892927DC495229F7DC1578
```

Sweep readback summary:

```json
{
  "observation_count": 64,
  "detected_alert_count": 3,
  "uncalibrated_observation_count": 0,
  "gate_refused_count": 1,
  "severity_filtered_count": 1,
  "candidate_count": 1,
  "top_delta": 0.98,
  "top_percentile": 1.0,
  "top_p_value": 0.015625,
  "top_threshold_delta": 0.66
}
```

## Conclusion

#1209 is complete for the blind-spot detector and sweep path. The production
sweep no longer ranks novelty from a global absolute delta; it ranks from
per-lens-pair empirical calibration and fails closed for uncalibrated pairs.
