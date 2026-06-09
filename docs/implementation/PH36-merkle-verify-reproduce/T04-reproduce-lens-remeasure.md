# PH36 T04 - `reproduce.rs`: content-addressed lens lookup + re-measure + Forge determinism

| Field | Value |
|---|---|
| **Phase** | PH36 - Merkle checkpoints + verify_chain + reproduce() |
| **Stage** | S7 - Ledger Provenance |
| **Crate** | `calyx-ledger` |
| **Files** | `crates/calyx-ledger/src/reproduce.rs` (<=500) |
| **Depends on** | T02, PH18 frozen content-addressed lenses, PH13 Forge determinism mode |
| **Axioms** | A15, A16 |
| **PRD** | `dbprdplans/11` section 3 and section 5 |
| **Status** | DONE / FSV-signed-off in #252 |

Implementation lives in `crates/calyx-ledger/src/reproduce.rs`; focused tests
and manual aiwonder FSV live in `crates/calyx-ledger/tests/reproduce_tests.rs`.

## Goal

Implement the lens-lookup and re-measure half of `reproduce(answer_id)`. Given
an `answer_id`, find its `Answer` ledger entry, extract the recorded
`(cx_id, slot_id, lens_id, weights_sha256, input_hash, corpus_shard_hash)` for
each measured slot, retrieve the frozen content-addressed lens snapshot
matching `weights_sha256`, activate Forge determinism mode with the recorded
seed, and re-embed each input to produce the re-measured slot vectors. This
half owns everything up to "I have the re-measured vectors"; T05 owns the
fusion re-run and drift assertion.

## Build

- [x] `ReproduceContext { answer_id, ledger_entries, recorded_slots }`.
- [x] `RecordedSlot` carries `cx_id`, `slot_id`, `lens_id`,
  `weights_sha256`, `input_hash`, optional `corpus_shard_hash`, `forge_seed`,
  and optional inline test input.
- [x] `build_reproduce_context(cf_reader, answer_id)` reads the matching
  `Answer` entry, follows `measure_refs`, decodes linked `Measure` payloads,
  and fails closed on malformed required fields.
- [x] `lookup_frozen_lens(registry, lens_id, weights_sha256)` verifies the
  registry's frozen snapshot hash and returns `CALYX_LENS_FROZEN_VIOLATION` on
  mismatch.
- [x] `CALYX_REPRODUCE_NONDETERMINISTIC` added to the core error catalog with
  remediation `no determinism seed in ledger entry - cannot guarantee
  reproduce fidelity`.
- [x] `activate_forge_determinism(forge, seed)` calls the Forge determinism
  surface before each re-measure.
- [x] `remeasure_slots` and `remeasure_slots_with_input_resolver` look up the
  frozen snapshot, activate determinism, resolve content-addressed input bytes,
  verify `input_hash`, and measure through the frozen registry surface.
- [x] Missing `forge_seed` returns `CALYX_REPRODUCE_NONDETERMINISTIC`.

## Tests

- [x] Synthetic two-slot context re-measures identically across repeated calls
  with the same seeds.
- [x] Weights hash mismatch returns exact `CALYX_LENS_FROZEN_VIOLATION`.
- [x] `forge_seed=0` activates determinism and remains bit-identical on repeat.
- [x] Edge coverage: empty `recorded_slots`; retired lens with frozen snapshot;
  retired lens without snapshot; resolved input hash mismatch.
- [x] Missing `forge_seed` in linked `Measure` payload returns exact
  `CALYX_REPRODUCE_NONDETERMINISTIC`.
- [x] `build_reproduce_context` reconstructs `Answer` plus linked `Measure`
  entries from ledger bytes.

## FSV

**Source of truth:** aiwonder disk bytes under
`/home/croyse/calyx/data/fsv-issue252-reproduce-20260609`.

**Readback artifacts:**

- `reproduce-remeasure-readback.json`
  sha256 `859e1806ce0853996f113ca62c1bb7008f55018de548eb67c199d78e40d7e6fe`
- `ledger-cf/0000000000000000.ledger`
  sha256 `9c3e6300b5057758b677071e1141345a4aafdadcaf6807cf8a616bd401ae8dea`
- `ledger-cf/0000000000000001.ledger`
  sha256 `02262abecc4c04052a412adfe76a49faa83d595e2cc3b5eda4d420aa930e5423`
- `gate-final-readback.txt`
  sha256 `d2f2ac020e7c4e2ff39ee78359bb77c16186c1e8e67054a35474cf998c0027af`

**Readback facts:** `before_rows=0`, `after_rows=2`, `recorded_slots=1`,
`forge_seeds=[52]`, original slot-0 dense vector `[28.0, 97.0, 1.0]`,
re-measured slot-0 dense vector `[28.0, 97.0, 1.0]`,
`slot0_max_abs_diff=0.0`, and weights mismatch returns
`CALYX_LENS_FROZEN_VIOLATION`.

## Done

- [x] `cargo check`, `cargo test`, and `cargo clippy -D warnings` green on
  aiwonder for `calyx-core` and `calyx-ledger`.
- [x] New Rust files are <=500 lines:
  `reproduce.rs` = 356, `reproduce_tests.rs` = 398.
- [x] Re-measured golden vector diff is <=1e-3 (`0.0` observed).
- [x] FSV evidence attached to #252.
- [x] No PH36 anti-pattern: no frozen-lens mutation, no silent drift, no
  deterministic seed omission, and no harness-only verdict without reading
  disk bytes.
