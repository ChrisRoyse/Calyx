# PH73 T06 - Capability-Card Gating

## Scope

Converted lenses are admitted through an explicit capability gate after profiling. The gate consumes one `CapabilityCard`, the candidate lens's maximum pairwise correlation against the current panel, and configurable thresholds.

## Contract

- Default minimum grounded signal: `0.05` bits (`CALYX_CAPABILITY_MIN_SIGNAL_BITS`).
- Default maximum pairwise correlation: `0.6` (`CALYX_CAPABILITY_MAX_PAIRWISE_CORR`).
- `Admit`: grounded assay signal is at least the bit threshold, the lens is not collapsed, and max panel correlation is within threshold.
- `Park`: grounded bits are missing, bits are too low, or the lens is collapsed/low-spread. Parked lenses are retained but not active search slots.
- `Retire`: max panel correlation is above threshold. Duplicate behavior is removed from the active panel.

## Source of Truth

- Capability-card JSON emitted by the registry profile gate.
- Panel slot state after applying the gate through `SwapController` lifecycle transitions.
- Ledger `EntryKind::Assay` rows containing the same gate JSON payload.

## Required FSV

On aiwonder, build a labeled synthetic probe corpus with known decisions and read back:

- at least one `Admit`, one `Park`, and one `Retire` capability-card JSON file;
- the panel listing after gate application, proving active/parked/retired slot states;
- the ledger rows after append, proving one `assay` entry per decision;
- edge outputs for missing grounded signal, collapsed/low-spread parking, and empty-probe `CALYX_ASSAY_INSUFFICIENT_SAMPLES`.

Gates:

- `cargo fmt --all -- --check`
- `scripts/linecount.sh`
- `cargo check --workspace`
- `cargo clippy --workspace --tests -- -D warnings`
- `cargo test --workspace -- --nocapture`
