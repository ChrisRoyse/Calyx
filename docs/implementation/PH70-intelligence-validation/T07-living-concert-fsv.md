# PH70 T07 - Living concert FSV - A31 one-corpus loop

| Field | Value |
|---|---|
| Phase | PH70 - Intelligence validation on real corpora |
| Stage | S18 - Datasets & Intelligence FSV |
| Crates | `calyx-cli` integration test over Aster, Registry, Assay, Anneal, Loom, Oracle, Ward, Ledger |
| Issue | #641 |
| Axioms | A2, A15, A26, A31, A32 |
| PRD | `DOCTRINE.md` section 1b; `00_INDEX.md` `BUILD_DONE` LIVING clause; `28_FSV_AND_TEST_DATA.md` |

## Goal

Prove the A31 living-system clause as a concert, not as separate engine demos:
perceive with Registry lenses, metabolize and remember in Aster, differentiate
with Assay, heal and sleep through Anneal evidence, grow the lens panel, foresee
with Oracle recurrence prediction, and defend with Ward, all in one bounded loop
over one corpus and one durable vault.

## Implementation

- `crates/calyx-cli/tests/living_concert_fsv.rs`
  - `living_concert_synthetic_known_io_smoke`: fast deterministic known-I/O proof.
  - `living_concert_aiwonder_scifact_fsv`: ignored aiwonder FSV using BEIR SciFact.
- `crates/calyx-cli/tests/support/living_concert*.rs`
  - Loads SciFact qrels/corpus bytes and records BLAKE3 digests.
  - Writes one durable Aster vault.
  - Persists base, slot, anchors, assay, online, recurrence, ledger, and WAL rows.
  - Runs CLI readback commands against the persisted vault.
- `crates/calyx-cli/src/ops.rs`
  - Adds `assay` to the generic `readback --cf` surface.

## Source Of Truth

Evidence root:

`/home/croyse/calyx/data/fsv-issue641-living-concert-<timestamp>`

Primary bytes:

- `vault/cf/base`, `slot_00`, `slot_01`, `slot_02`, `slot_03`
- `vault/cf/anchors`, `assay`, `online`, `recurrence`, `ledger`
- `vault/wal`
- CLI readbacks: `readback-*.txt`, `living-concert-readback.json`
- `fsv-b3sum-manifest.txt`

## Required FSV

- Run on aiwonder with:

```bash
CALYX_ISSUE641_FSV_ROOT=/home/croyse/calyx/data/fsv-issue641-living-concert-<timestamp> \
cargo test -p calyx-cli --test living_concert_fsv living_concert_aiwonder_scifact_fsv -- --ignored --nocapture
```

- Manually read the SoT bytes after the test:

```bash
cat "$ROOT/living-concert-readback.json"
cat "$ROOT/readback-base.txt"
cat "$ROOT/readback-assay.txt"
cat "$ROOT/readback-online.txt"
cat "$ROOT/readback-recurrence.txt"
cat "$ROOT/readback-ledger.txt"
cat "$ROOT/readback-wal.txt"
cargo run -p calyx-cli --quiet -- verify-chain --vault "$ROOT/vault" --range 0..<ledger_end>
cargo run -p calyx-cli --quiet -- merkle-root --vault "$ROOT/vault" --range 0..<ledger_end>
find "$ROOT/vault" -type f -ls
```

## Done When

- `cargo check`, `cargo test`, and `cargo clippy -D warnings` pass on aiwonder.
- Every `.rs` file remains at or below 500 lines.
- The ignored SciFact FSV writes a named evidence root.
- Manual readback proves:
  - Assay admitted a differentiating growth lens.
  - Aster stored base, slot, anchor, assay, online, recurrence, ledger, and WAL bytes.
  - Oracle predicted the next recurrence time from persisted recurrence rows.
  - Ward rejected an injected mismatch with a Guard ledger entry.
  - Anneal recorded degraded-to-rebuilt healing and over-budget yielding.
  - The conflicting-anchor recurrence edge persisted separate CxIds and `merged=false`.
  - The Ledger chain verifies over the complete concert range.
