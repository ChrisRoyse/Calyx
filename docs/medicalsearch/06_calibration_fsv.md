# Calibration FSV

## Objective

Issue #873 verifies that planted known signals are recovered before any
biomedical discovery output is trusted.

Known-pair seed: `metformin -> type_2_diabetes`, sourced from MedlinePlus Drug
Information for metformin:
https://medlineplus.gov/druginfo/meds/a696005.html

## FSV Scope

The calibration proof creates real durable Calyx state and then reads source of
truth bytes back:

- Base CF: 100 planted constellations with grounded label anchors.
- Assay CF: `calyx bits` calculation persisted and read back as JSON bytes.
- Kernel CF: anchored kernel report persisted and read back as JSON bytes.
- XTerm CF: Loom agreement xterm persisted through the Aster CF router and
  re-opened.
- Oracle Assay CF: sufficient and insufficient gate evidence persisted as
  scoped `AssayRow` rows, then read by `VaultSufficiencyAssay`.

## Expected Results

- Identical vectors produce agreement `1.0` and agreement weight `1.0`.
- The planted metformin/type-2-diabetes slot recovers `0.5` bits.
- The control/no-signal slot recovers `0.0` bits and fails closed when used
  alone.
- The planted anchored kernel grounds with `recall = 1.0`.
- Oracle sufficient case returns a sufficient bound.
- Oracle insufficient case refuses with `CALYX_ORACLE_INSUFFICIENT`.
- Edge cases preserve source-of-truth row counts after refusal:
  insufficient samples, low signal, and ungrounded kernel.

## Evidence

aiwonder FSV root:

`/home/croyse/calyx/fsv/issue873-calibration-20260625T093450Z`

Readback artifact:

`/home/croyse/calyx/fsv/issue873-calibration-20260625T093450Z/issue873_calibration_fsv_readback.json`

Artifact bytes: `1865`

Artifact SHA256:

`36ba305c870b8ff618f62b41ac2687fb5d1a40e6882b1f06cc5f9b1af93b83c4`

Commands and statuses:

- `cargo fmt --all -- --check`: status `0`, stdout `0` bytes, stderr `0` bytes.
- `git diff --check`: status `0`, stdout `0` bytes, stderr `0` bytes.
- `bash scripts/linecount.sh`: status `0`, stdout `26` bytes, stderr `0` bytes.
- `CALYX_FSV_ROOT=<root> cargo test -p calyx-cli cmd::intelligence::calibration_fsv_tests::planted_calibration_signals_roundtrip_from_durable_state -- --nocapture`: status `0`, stdout `3841` bytes, stderr `3637` bytes.
- `cargo build -p calyx-cli`: status `0`, stdout `0` bytes, stderr `1154` bytes.

Bounded artifact leaves:

- `schema = calyx-medicalsearch-calibration-fsv-v1`
- `known_pair = metformin -> type_2_diabetes`
- `bits.base_rows_after = 100`
- `bits.slot0_bits = 0.5`
- `bits.slot1_bits = 0.0`
- `kernel.recall = 1.0`
- `kernel.kernel_size = 1`
- `loom.persisted_agreement = 1.0`
- `oracle.sufficient.sufficient = true`
- `oracle.insufficient.code = CALYX_ORACLE_INSUFFICIENT`
- `edge_cases.low_signal_code = CALYX_ASSAY_LOW_SIGNAL`
- `edge_cases.insufficient_samples.code = CALYX_ASSAY_INSUFFICIENT_SAMPLES`
- `edge_cases.ungrounded_kernel.code = CALYX_KERNEL_UNGROUNDED`

The FSV root contained `28` files and `368281` bytes after readback.
