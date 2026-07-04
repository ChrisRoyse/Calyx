# #1204 Oracle Honesty Gate CI-Low / Calibration Hardening

## Scope

#1204 fixes the Oracle honesty gate so sufficiency is decided from the calibrated
lower-bound basis, not the MI point estimate.

Before this slice, the vault-backed oracle path loaded only `estimate.bits` from
assay rows and rebuilt a diagnostic sufficiency report. That discarded
`MiEstimate.ci_low` and `PowerCalibration`, so a panel could pass when its point
estimate cleared entropy even if the lower bound did not.

## Code Change

Changed files:

| File | Change |
|---|---|
| `crates/calyx-assay/src/sufficiency.rs` | added context-preserving `panel_sufficiency_from_estimate_with_context` and kept sufficiency basis from `estimate.ci_low` |
| `crates/calyx-assay/src/sufficiency/joint.rs` | moved panel-joint union-floor helper out of `sufficiency.rs` to keep linecount under 500 |
| `crates/calyx-assay/src/lib.rs` | exported the new context-preserving sufficiency constructor |
| `crates/calyx-oracle/src/honesty_gate.rs` | gate now compares `sufficiency_basis_bits >= anchor_entropy_bits`; vault path loads the full panel `MiEstimate` and requires assay calibration |
| `crates/calyx-oracle/src/honesty_gate_tests.rs` | added FSV tests for point-estimate pass / lower-bound fail and missing calibration fail-closed |

Behavior now enforced:

- `bits >= H` but `ci_low < H` returns `CALYX_ORACLE_INSUFFICIENT`.
- Missing panel power calibration returns `CALYX_ASSAY_ESTIMATOR_UNDERPOWERED`.
- Passing requires `ci_low >= anchor_entropy_bits` and
  `PowerCalibrationStatus::Passed`.
- The bound exposed by the oracle uses the same lower-bound basis used for the
  pass/fail decision.

## Real FSV

Final FSV root:

```text
C:\code\Calyx-Dev\target\fsv\issue1204-oracle-honesty-ci-low-20260704T031838Z
```

Persisted readback:

| Field | Value |
|---|---|
| Status | `ok` |
| `cargo test -p calyx-oracle honesty_gate -- --nocapture` | exit 0 |
| `cargo test -p calyx-assay sufficiency -- --nocapture` | exit 0 |
| `cargo check -p calyx-oracle` | exit 0 |
| `cargo check -p calyx-assay` | exit 0 |
| `bash scripts/linecount.sh` | exit 0 |
| `git diff --check` | exit 0 |
| `persisted_readback.json` SHA-256 | `2c928a7318255c9f2c87e96a93356c4a5e0884551ae098c3b1f6f5a3e2fdc7af` |

FSV log files:

| Artifact | Bytes | SHA-256 |
|---|---:|---|
| `calyx-oracle-honesty-gate.stdout.log` | 1,383 | `a93f88ad8183032bba78554705a8dd462ce882b2950d6398def9b73cca1f2685` |
| `calyx-oracle-honesty-gate.stderr.log` | 716 | `bc716fb042e502ac42bc17bfc004cebb3ad6d3511fd644ec17cf0e0533ffcc2a` |
| `calyx-assay-sufficiency.stdout.log` | 2,602 | `3dcabc26c524aa14de8e61148c5e07f048afcbcade8e6970e7ccaed4cc4bc1b6` |
| `calyx-assay-sufficiency.stderr.log` | 2,318 | `c7a113dd558492bfdfb85cc19c9be14b424db65cc8946fb1b459e1f54628687d` |
| `calyx-oracle-check.stderr.log` | 601 | `f5ea4994ddd4ea009e3fa7bbf6c0d2d12bec9039f23df6c2f1e4b2a97fc6eac0` |
| `calyx-assay-check.stderr.log` | 225 | `d902a2e263a3eddc599dc85a1e3ed971a09c4f6ab7a07eabb1996b4eb1492e3f` |
| `linecount.stdout.log` | 26 | `2ca9608a7e23755e5f4038d3d0e6ae4482f4acf00be03489434b68af163e79f1` |

The oracle tests read the assay rows back through `VaultSufficiencyAssay`, so
the source of truth for the pass/fail decision is the persisted `AssayStore`
inside the vault, not a direct return value from a hand-built report.

## Conclusion

#1204 is complete for the oracle honesty gate:

- point-estimate-only sufficiency no longer passes;
- missing/underpowered calibration fails closed;
- the assay constructor and oracle gate now use the same lower-bound basis;
- the linecount split keeps the repository under the enforced structural gate.

This hardens the trust boundary before additional biomedical hunts consume
oracle sufficiency verdicts.
