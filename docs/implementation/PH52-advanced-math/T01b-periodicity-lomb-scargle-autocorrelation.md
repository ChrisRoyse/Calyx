# PH52 T01b - Lomb-Scargle + Autocorrelation Periodicity

| Field | Value |
|---|---|
| Phase | PH52 - Advanced math |
| Issue | #584 |
| Crate | `calyx-assay` |
| Files | `crates/calyx-assay/src/periodicity.rs`, `crates/calyx-assay/tests/periodicity_fsv.rs` |
| PRD | `docs/dbprdplans/26_ADVANCED_MATH_FRONTIERS.md` section 4 |

## Purpose

Periodicity detection is a first-class build card, not a hidden helper inside the PH52 FSV. Calyx needs dominant-period detection for recurrence, next-occurrence prediction, and drift/change-point work over irregular temporal samples.

## Implementation

`src/periodicity.rs` provides:

- floating-mean generalized Lomb-Scargle periodograms for irregular samples
- event timestamp binning into count observations
- seeded permutation false-alarm probabilities
- ranked multiple peak reporting
- slotted autocorrelation as an independent fundamental-period cross-check
- fail-closed input validation with existing `CALYX_ASSAY_*` catalog codes

All randomness is deterministic through ChaCha8 seeds. The module stays under the 500-line Rust gate.

## FSV Evidence

The aiwonder FSV run for #584 wrote:

- Root: `/home/croyse/calyx/data/fsv-issue584-periodicity-20260612T151322Z`
- Period JSON SHA256: `dd7b8790d28c3df1a9cdab3f9a9df90ff2d791951c12fd4686166fe5365b6187`
- Edge JSON SHA256: `81746f8cb5f925c11d28ff709d836574807d1ad0e2b442bf7d663b9cab373a1d`

Readback values:

- planted period: `7.0`
- detected period: `6.996966632962588`
- autocorrelation dominant lag: `7.0`
- false-alarm probability: `0.009900990099009901`
- within +/-5%: `true`

Edge cases prove fail-closed behavior for empty input, below-min samples, zero-variance values, NaN values, non-monotonic times, and pure noise with no fabricated significant period.
