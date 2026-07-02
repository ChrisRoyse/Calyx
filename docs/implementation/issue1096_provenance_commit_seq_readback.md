# Issue 1096 - Exact provenance→commit-seq slot readback (near-seq heuristic removed)

## Problem

The 80G-vault probe during #1060 FSV showed that near-seq slot CF point reads
miss on group-committed vaults: 99,498 durable-batch SSTs, zero compacted
files, and 0/75 near-seq hits — every payload only resolved through the
full-SST-set retry. cx-list survived via that backstop (at O(all files) per
missing key); weave-loom dense coverage had **no** backstop and counted every
miss as `missing_rows`, silently undercounting dense coverage and skewing
candidate slot selection on bulk-ingested vaults.

## Root Cause

`Constellation.provenance.seq` and durable-batch SST file seqs come from two
independent counters:

1. `provenance.seq` is the **ledger seq**, assigned when the put is staged
   (`vault/store.rs`: `constellation.provenance = staged.first().ledger_ref()`).
2. Durable-batch SSTs are named `{commit_seq:020}-{first_index:04}.sst` where
   `commit_seq` is the **WAL commit seq** returned by the group-commit batcher
   (`vault/commit.rs` → `DurableVault::write_rows`).

`latest_cf_rows_near_seqs` guessed the containing file via
`storage_seqs_for_provenance(seq) = {seq, seq+1}`. That alignment is
coincidental — it holds only for tiny one-put-per-commit vaults — and is
structurally wrong under bulk/group-committed ingest. No widening of the
window can fix it: the drift is unbounded.

## Fix

New module `crates/calyx-cli/src/provenance_read.rs` (`VaultReadContext`),
used by both weave-loom dense coverage and cx-list `--include-slots`:

1. **Commit-batch resolution (exact, not heuristic).** Every put stages its
   `ledger_key(provenance.seq)` row (8-byte big-endian, lexicographic ==
   numeric order) in the **same WAL batch** as its base/slot rows, so each
   durable-batch ledger SST covers one contiguous provenance range while its
   file name carries the commit seq. Binary search over ledger SST footer key
   ranges (loaded lazily, cached for the context lifetime) maps provenance
   seq → commit seq; the row is then read from exactly that batch's SST(s).
   Router-named and compacted ledger files are excluded (their file seqs are
   not commit seqs of contained rows).
2. **Metadata-pruned full-level lookup.** Keys stage 1 cannot resolve read
   through `SstLevel::from_oldest_first_with_lookup` (first/last key + bloom
   pruning, newest-first, compacted files included) — the semantic source of
   truth, built once per CF per context. Stage 1 is bypassed entirely for CFs
   containing compacted files, because compaction rewrites row history.
3. **WAL tail overlay** last (unchanged semantics, now decoded once per
   context instead of once per chunk).

Every resolved row carries a `RowSource` (`commit_batch` / `full_set` /
`wal_tail`) and per-call `ProvenanceReadStats`, so readers log exactly which
stage produced each row.

Consumer changes:

- **weave-loom coverage** (`cmd/weave/coverage/scan.rs`): a base-listed slot
  whose physical row no stage can resolve **fails closed** with
  `CALYX_ASTER_CORRUPT_SHARD` (same doctrine as cx-list #1060) instead of
  being counted as missing coverage. Counters are additive by construction —
  `dense + non_dense + absent + tombstoned + missing == candidate_rows` (the
  scan errors on any accounting mismatch) — where `missing_rows` now means
  only "candidate's base row does not list this slot". Tombstone values are
  recognized explicitly instead of aborting the scan with a decode error.
  Each coverage row serializes its `read_stats`.
- **cx-list** (`dedup_audit_readback/physical_slots.rs`): migrated to the
  same primitive. Commit-batch/WAL-tail rows keep the `slot_cf` /
  `slot_raw_cf` payload_source labels; full-level rows keep the
  `slot_cf_full_set` / `slot_raw_cf_full_set` labels from #1060. The
  per-missing-key full-set scan pathology (one O(all-SSTs) pass per miss) is
  gone; the full level is built once per CF with pruning metadata.
- The `{seq, seq+1}` near-seq machinery (`latest_cf_rows_near_seqs`,
  `storage_seqs_for_provenance`, `same_seq_sst_files_for_seqs`) was deleted.
- `calyx-aster` gains `SstReader::key_range()` (public footer first/last key
  accessor) for the ledger index.

## Why this holds going forward

- The ledger mapping is written by the same atomic WAL batch as the data
  rows, so it exists for every put on every vault shape — no heuristic
  alignment assumption remains anywhere in the readback layer.
- Any vault shape the ledger index cannot serve (malformed, compacted,
  gapped) degrades to the metadata-pruned newest-first full level, which is
  correct by definition; a malformed ledger key errors loudly
  (`non-canonical ledger key`) instead of silently degrading.
- A base-listed slot row that cannot be physically resolved is now a typed
  corruption error in *both* consumers — it can never again present as a
  coverage statistic.

## Verification

- Unit tests (`provenance_read_tests.rs`): the exact #1096 misaligned shape
  resolves via commit batch; no-ledger vaults resolve via full level
  (newest-first); router ledger files are never used as a commit index;
  compacted CFs bypass stage 1; provenance seq 0 / u64::MAX boundaries;
  unresolvable keys report `None` + `unresolved` stats; malformed ledger keys
  fail loud.
- `cx_list_include_slots_readback` integration suite: real-ingest vaults
  resolve every payload via `slot_cf` (commit batch — stage 1 proven on real
  vault layout); synthetic no-ledger fixtures carry honest
  `*_full_set` labels; fail-closed and tombstone paths unchanged.
- aiwonder FSV on the 80G corpus vault
  (`/home/croyse/calyx/vaults/01KVYX0KYVBQSGVC6N2S00FX6J`): before/after
  weave-loom coverage comparison and cx-list payload_source distribution —
  see issue #1096 comments for the evidence log.

## Latent hazard: stage-1 write-once slot-row assumption (issue #1107)

Stage 1 reads a slot row from the durable-batch SST of its ORIGINAL commit
(located via the ledger provenance index) and deliberately does not consult
newer batches. This is correct **only** while slot CF rows are write-once for
live constellations: ingest stages them once, anchor-merge rewrites only
base/anchors, deletion uses tombstones, compacted CFs bypass stage 1 entirely,
and WAL-tail rows override.

If a future feature rewrites an existing slot CF key in a **later** durable
batch without compaction (in-place slot recompression, quantization migration,
payload GC), stage 1 would return the original-batch value — a stale read —
and cx-list / weave-loom would report the superseded payload.

Guardrails to land **with** any such feature (do not ship the feature without
one):

- have the rewriter also tombstone/compact the CF (compaction presence already
  disables stage 1), or
- version slot rows / bump provenance on rewrite, or
- add a newer-batch overlay to stage 1 that fails closed when a resolved key is
  superseded.

Tripwire: `provenance_read_tests::later_batch_slot_rewrite_is_missed_by_stage1_write_once_assumption`
constructs the exact later-batch-rewrite shape and asserts stage 1 still
returns the original value. When that assertion flips, the assumption has been
violated and a guardrail is required before merge. The stage-1 call site
(`read_from_commit_batches` in `provenance_read.rs`) carries the same warning.
