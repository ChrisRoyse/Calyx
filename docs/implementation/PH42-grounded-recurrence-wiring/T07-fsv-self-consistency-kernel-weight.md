# PH42 - T07 - FSV: self-consistency and kernel frequency weight

| Field | Value |
|---|---|
| **Phase** | PH42 - Grounded Recurrence Wiring Across Engines |
| **Stage** | S9 - Temporal & Dedup |
| **Crate** | `calyx-assay` / `calyx-lodestar` / `calyx-ward` / `calyx-loom` |
| **Files** | `crates/calyx-assay/tests/ph42_exit_gate_fsv.rs`, `crates/calyx-assay/tests/ph42_exit_gate_support/mod.rs` |
| **Depends on** | T06 |
| **Axioms** | A29, A20, A10 |
| **PRD** | `dbprdplans/25 A-4c`, `dbprdplans/07 A-3b`, `dbprdplans/08 A-2` |
| **GitHub** | #393 |
| **Status** | Complete |

## Goal

Write the formal PH42 exit-gate FSV that proves:

1. Recurring events with agreeing outcomes produce high Assay oracle self-consistency.
2. Recurring events with differing outcomes are classified as flaky.
3. Equal-betweenness kernel candidates are ranked by grounded recurrence frequency.
4. Ward surprise is retrieval-only and does not inflate stored Base CF bytes.
5. Loom temporal cross-term lead/lag is directional.

## Implementation

- Added ignored aiwonder FSV trigger:
  `cargo test -p calyx-assay --test ph42_exit_gate_fsv -- --ignored --nocapture`
- Added reusable PH42 exit-gate support helpers for deterministic CxIds, durable Aster writes, raw CF state capture, artifact writing, and BLAKE3 manifest writing.
- Added `calyx-lodestar`, `calyx-paths`, and `calyx-ward` as `calyx-assay` dev-dependencies so the exit gate can exercise the real cross-crate production surfaces from one deterministic FSV trigger.

## Deterministic Cases

- Agreeing outcomes: 5 CxIds, each with 4 repeated `stable` outcomes.
- Flaky outcomes: 5 CxIds, each with `[agree, agree, differ, differ]`.
- Mixed domain: 5 agreeing CxIds plus 5 flaky CxIds.
- Insufficient edge: 5 CxIds with only 2 outcomes each; unknown is permissive and returns 1.0.
- Kernel ranking: CxId `28282828282828282828282828282828` has frequency 50 and CxId `29292929292929292929292929292929` has frequency 1 with equal betweenness 0.80.
- Kernel zero-frequency edge: CxIds `2a...2a` and `2b...2b` both carry frequency 0 and receive zero frequency bonus.
- Ward novelty: singleton CxId `3c...3c` appears once in a 100-event domain and computes about 6.64 surprise bits, but the stored Base CF row does not contain the surprise float bytes.
- Temporal lead/lag: CxId A `46...46` occurs at `[100, 200, 300, 400, 500]`; CxId B `47...47` occurs at `[115, 215, 315, 415, 515]`.

## aiwonder FSV

Durable artifact root:

`/home/croyse/calyx/data/fsv-issue393-ph42-exit-gate-20260610-2141`

Artifacts:

- `assay-report.json`
- `kernel-weights.json`
- `ward-novelty.json`
- `temporal-cross-term.json`
- `ph42-exit-gate.json`
- `BLAKE3SUMS.txt`
- `base-cf-readback.txt`
- `recurrence-cf-readback.txt`
- `temporal-xterm-cf-readback.txt`
- `ledger-cf-readback.txt`
- `wal-readback.txt`
- `vault-tree-readback.txt`

Manifest readback:

- `BLAKE3SUMS.txt` includes the PH42 artifacts and backing vault files.
- `b3sum --check --quiet BLAKE3SUMS.txt` passed on aiwonder.

Current source-of-truth row counts from `ph42-exit-gate.json`:

- Before: base 0, recurrence 0, ledger 0, temporal_xterm 0, snapshot 0.
- After: base 23, recurrence 210, ledger 23, temporal_xterm 2, snapshot 235.

PH42 artifact readbacks:

- Assay agreeing score: `1.0` (`>= 0.90`).
- Assay flaky score: `0.3333333432674408` (`<= 0.60`).
- Assay mixed score: `0.6666667461395264` (`0.55..=0.75`).
- Assay insufficient score: `1.0`.
- Kernel rank 1: `28282828282828282828282828282828`, frequency 50, bonus `0.4268878996372223`, total `0.8640331849455833`.
- Kernel rank 2: `29292929292929292929292929292929`, frequency 1, bonus `0.07525668293237686`, total `0.8112885024398566`.
- Zero-frequency kernel edge: both frequency bonuses are `0.0`; graph still returns two ranked rows.
- Ward singleton surprise: `6.643856048583984`; Base CF contains neither the f32 bytes `40d49a78` nor the f64 bytes `401a934f00000000`.
- Temporal forward lead/lag: `15.0` seconds for A -> B.
- Temporal reverse lead/lag: `-15.0` seconds for B -> A.

## Gates

aiwonder:

- `cargo fmt --check`
- `git diff --check`
- `bash scripts/linecount.sh`
- `cargo test -p calyx-assay --test ph42_exit_gate_fsv --quiet`
- `cargo clippy -p calyx-assay --test ph42_exit_gate_fsv --quiet -- -D warnings`
- `cargo check -p calyx-cli --quiet`
- `cargo test -p calyx-assay --test ph42_exit_gate_fsv -- --ignored --nocapture`

Local Windows authoring checkout:

- `cargo fmt --check`
- `bash scripts/linecount.sh`
- `cargo test -p calyx-assay --test ph42_exit_gate_fsv --quiet`
- `cargo clippy -p calyx-assay --test ph42_exit_gate_fsv --quiet -- -D warnings`

## Done

- [x] Tests and clippy pass for the focused exit-gate target.
- [x] FSV writes durable artifacts on aiwonder.
- [x] Separate readbacks inspect persisted PH42 artifacts, Aster CF bytes, WAL bytes, vault tree, and BLAKE3 manifest.
- [x] No `.rs` file exceeds 500 lines.
- [x] Evidence is ready for the #393 GitHub closeout comment.
