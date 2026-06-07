# PH07 — Column families + key encoding

**Stage:** S1 — Aster storage core  ·  **Crate:** `calyx-aster`  ·
**PRD roadmap:** P0  ·  **Axioms:** A16

## Objective

Finalize the association-native column families (`base`, `slot_00..NN`,
`slot_NN.raw`, `xterm`, `scalars`, `anchors`, `ledger`, `online`) with their
per-CF big-endian key codecs, `CxId` 16-byte blake3 prefix construction,
collision-check on write, and range-scan helper `KeyRange`. Wire these codecs
into the on-disk SST CF directories so a write to CF `base` physically lands in
`vault/cf/base/` and a write to `slot_00` lands in `vault/cf/slot_00/`. Produce
`CALYX_ASTER_CORRUPT_SHARD` on any hash mismatch.

## Dependencies

- **Phases:** PH06 (SST writer/reader, `SstLevel`), PH04 (CxId, AnchorKind,
  SlotId types)
- **Provides for:** PH08 (MVCC wraps CF dispatch), PH09 (vault write uses CF
  keys), PH10 (manifest lists CF SST files)

## Current state (build off what exists)

`cf/mod.rs`, `cf/key.rs`, and `cf/family.rs` are already written with:
- All CF variants: `Base`, `Slot { slot, kind }`, `XTerm`, `Scalars`, `Anchors`,
  `Ledger`, `Online`.
- Key encoders: `base_key`, `slot_key`, `xterm_key`, `scalar_key`, `anchor_key`,
  `ledger_key`, `online_key` — all big-endian.
- `KeyRange`, `prefix_range`, range helpers for each CF.
- `full_content_hash`, `cx_id_from_full_hash`, `verify_cx_hash_prefix` (blake3
  collision check returning `CALYX_ASTER_CORRUPT_SHARD`).
- `cf/tests.rs` exists (noted in `cf/mod.rs`) but may lack full coverage.

**What remains:**
- A `CfRouter` (or `CfDiskLayout`) that maps `ColumnFamily` → a `SstLevel` rooted
  at the correct on-disk directory (e.g., `vault/cf/base/`, `vault/cf/slot_00/`).
- Per-CF `put` and `get` dispatch that uses `SstLevel` + a per-CF memtable, so
  writes and reads go through actual files (not the in-memory `VersionedCfStore`).
- Full proptest coverage of all key codecs: encode+decode round-trip; key
  ordering; prefix range containment.
- Integration test: write one row to each CF → read each back byte-exact;
  verify key ordering supports range scans in the intended direction.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `src/cf/mod.rs` | Re-exports, `CfRouter`, wires family → disk path |
| `src/cf/family.rs` | `ColumnFamily` enum, `name()`, slot/raw helpers — already done |
| `src/cf/key.rs` | All key codecs, big-endian ordering, `KeyRange`, collision check — already done |
| `src/cf/router.rs` | `CfRouter`: maps CF → `SstLevel` path, per-CF memtable, put/get dispatch |
| `src/cf/tests.rs` | Full codec proptest suite + CF router integration tests |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | Key codec proptest suite (all CFs) | — |
| T02 | CxId blake3 prefix + collision check | T01 |
| T03 | CF router: per-CF SstLevel + on-disk put/get | T01 |
| T04 | CF round-trip + range-scan FSV | T02, T03 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

Write one row per CF to the router; read each back:

```
calyx readback --cf base  --vault /home/croyse/calyx/test-vault
calyx readback --cf slot_00 --vault /home/croyse/calyx/test-vault
calyx readback --cf anchors --vault /home/croyse/calyx/test-vault
xxd /home/croyse/calyx/test-vault/cf/base/000001.sst | head -2
xxd /home/croyse/calyx/test-vault/cf/slot_00/000001.sst | head -2
```

Expected: each CF directory contains a valid SST file; `calyx readback` returns
the written key/value byte-exact; range scan on `base` returns keys in big-endian
ascending order. `CALYX_ASTER_CORRUPT_SHARD` on hash mismatch verified by test.
Evidence posted to PH07 GitHub issue.

## Risks / landmines

- `CxId` is 16 bytes (blake3 prefix); two different inputs could collide on the
  first 16 bytes with astronomically low but non-zero probability. The
  `verify_cx_hash_prefix` check catches the case where a stored CxId and a
  recomputed full hash disagree — ensure this is called on every read from base CF.
- `AnchorKind::Label(string)` has variable-length key encoding; test that
  `anchor_key` for two different label strings produces different, correctly
  ordered keys.
- `xterm_key` uses `to_be_bytes()` for `SlotId` components — ensure `SlotId`
  is a `u16` (2 bytes), not `u32`, to match the 5-byte xterm key suffix.
