# Issue 1060 - cx-list --include-slots physical slot readback

## Problem

`readback cx-list --include-slots` reported `dense_slots: 0` / `absent_slots: N`
(`payload_source: "base_absent"`, reason `not_applicable`) for vaults whose
slot CFs physically contained dense payload rows, contradicting direct
`readback --cf slot_XX` output and the weave-loom dense coverage preflight.

## Root Cause

Two divergent read paths, both in `crates/calyx-cli`:

1. **Dead readback branch.** Base CF rows persist only `slot_id + blake3(payload)`
   per slot; `decode_constellation_base` therefore reconstructs *every* slot as
   `SlotVector::Absent { NotApplicable }`. `decoded_slot_entries` skipped the
   slot CF lookup whenever the Base placeholder was `Absent` — which was
   always. The physical lookup below the skip was unreachable, so cx-list
   could only ever print fabricated absent placeholders.
2. **Divergent near-seq reader.** cx-list used the single-row
   `latest_cf_row_near_seq`, which matched SST file seqs *exactly* against the
   zero-based provenance seq. The grouped reader (`latest_cf_rows_near_seqs`,
   proven by #1058 weave-loom) probes `{seq, seq+1}` to bridge one-based
   storage seqs. Even without bug 1, single-row lookups missed rows.

## Fix

- `--include-slots` now hydrates slot state exclusively from physical slot CF
  rows via the shared grouped near-seq reader (`latest_cf_rows_near_seqs`) —
  the same read path weave-loom dense coverage uses. Base placeholders are
  never reported as slot state.
- Near-seq misses retry against the full SST set (including compacted files)
  plus WAL via `latest_cf_rows_for_keys`, labeled `slot_cf_full_set`. This
  retry is a correctness backstop, not just a compaction path: bulk
  group-committed ingest packs many provenance seqs into one durable-batch
  SST whose file seq does not align with each row's provenance seq, so
  near-seq point reads miss on such vaults (verified on the 80G aiwonder
  corpus vault: all slot payloads resolved via `slot_cf_full_set`).
- Undecodable slot CF bytes fall through to the `slot_raw_XX` CF (compression
  persists opaque compressed bytes in `slot_XX` and the decodable payload in
  `slot_raw_XX`), labeled `slot_raw_cf`.
- Physical tombstones are reported as `kind: "tombstoned"` with a
  `tombstoned_slots` summary count, never as `absent`.
- **Fail closed:** a Base-listed slot with no physical payload row anywhere
  (slot CF near-seq, full set, WAL, slot_raw) or with undecodable bytes and no
  decodable raw pair errors with `CALYX_ASTER_CORRUPT_SHARD`, naming the cx id,
  slot, provenance seq, and vault path. Ingest stages one physical slot row
  per Base-listed slot in the same WAL batch as the Base row, so this state is
  real corruption, not a reportable condition.
- The divergent `latest_cf_row_near_seq` was deleted; the grouped reader is
  the only near-seq read path left in `cf_read.rs`.

`payload_source` values: `slot_cf`, `slot_cf_full_set`, `slot_raw_cf`,
`slot_raw_cf_full_set`, `slot_cf_tombstone`, `slot_cf_full_set_tombstone`.
`slot_payload_decode_mode` is now `physical_slot_cf_readback` (was
`explicit_include_slots`).

## Verification

- `crates/calyx-cli/tests/cx_list_include_slots_readback.rs`: real durable
  vaults through the production ingest path + real binary execution; edge
  cases write real SSTs with the production encoders (missing slot rows,
  undecodable bytes with/without raw pair, tombstones, empty vault).
- FSV on aiwonder against vault `01KWE0V8QQREZZXWP836ZXMGW2` (the #1058/#1060
  evidence vault): cx-list dense/sparse/absent counts now match direct
  `readback --cf slot_XX` physical rows, including the true persisted absent
  reason (`lens_inactive`).
