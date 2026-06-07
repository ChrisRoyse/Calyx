# PH10 — Manifest + atomic swap + crash recovery

**Stage:** S1 — Aster storage core  ·  **Crate:** `calyx-aster`  ·
**PRD roadmap:** P0  ·  **Axioms:** A15, A16

## Objective

Deliver an atomic manifest pointer (`CURRENT` → `manifest-NNNN.json` via
`rename()`), vault recovery that replays WAL past the last durable manifest seq
to the last fsync'd record, and fail-closed corrupt-base detection
(`CALYX_ASTER_CORRUPT_SHARD`). After PH10, the vault round-trips `kill -9` at
any point in the write path and recovers byte-exact to the last acked record.

## Dependencies

- **Phases:** PH09 (vault write path, WAL, CF persistence), PH05 (WAL replay),
  PH07 (CF keys), PH04 (CalyxError)
- **Provides for:** PH11 (compaction uses manifest to track SST files),
  PH35 (Ledger recovery via manifest), PH58 (GC uses manifest seq watermark)

## Current state (build off what exists)

`manifest/mod.rs` is already written with:
- `VaultManifest`: `version`, `manifest_seq`, `durable_seq`, `panel_ref`,
  `codebook_refs`, `degraded_rebuildable`.
- `ManifestStore::write_current` / `load_current`: atomic write via `write_atomic`
  (temp + `rename()`).
- `recover_vault`: loads MANIFEST, replays WAL past `durable_seq`, returns
  `RecoveryOutcome`.
- `read_base_shard`: fail-closed SST read with `CALYX_ASTER_CORRUPT_SHARD`.
- `manifest/tests.rs` exists.

**What remains:**
- `recover_vault` currently loads MANIFEST + replays WAL but does NOT apply the
  WAL records to a `VersionedCfStore` or `CfRouter`. The recovery must reconstruct
  the in-memory MVCC state from the WAL records (re-apply each write batch) so
  the vault is ready to serve reads immediately after recovery.
- The `AsterVault::open` constructor (for on-disk vaults) must call `recover_vault`,
  set `SeqAllocator::set_start_seq(last_recovered_seq)`, and re-apply WAL writes.
- A crash drill that tests `kill -9` at three specific points: (1) before WAL
  fsync, (2) after WAL fsync but before `commit_batch`, (3) after `commit_batch`
  but before manifest write.
- Corrupt-base test: flip a byte in a base CF SST → read fails with
  `CALYX_ASTER_CORRUPT_SHARD`.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `src/manifest/mod.rs` | `VaultManifest`, `ManifestStore`, `recover_vault`, `read_base_shard` |
| `src/manifest/recovery.rs` | WAL-replay-to-MVCC reconstruction; `AsterVault::open` |
| `src/manifest/tests.rs` | Crash drill tests; corrupt-shard test; atomic swap test |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | Manifest atomic swap + version guard | — |
| T02 | WAL-replay recovery: reconstruct MVCC from WAL records | T01, PH09 T02 |
| T03 | AsterVault::open — recovery constructor | T02 |
| T04 | kill -9 crash drill (3 points) + corrupt-shard FSV | T03 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

Run the crash drill: `kill -9` at each of 3 points → `calyx recover` →
`calyx readback`. Prove byte-exact recovery to last acked record.

```
calyx crash-drill --vault /home/croyse/calyx/test-vault --point before-wal-fsync
calyx recover --vault /home/croyse/calyx/test-vault
calyx readback --cf base --vault /home/croyse/calyx/test-vault
xxd /home/croyse/calyx/test-vault/CURRENT
```

Also: flip one byte in `vault/cf/base/000001.sst` → `calyx readback --cf base`
returns `CALYX_ASTER_CORRUPT_SHARD`, not silently empty. Evidence posted to PH10
GitHub issue.

## Risks / landmines

- `rename()` on Linux is atomic only within the same filesystem. Staging the temp
  file inside the vault directory (same ZFS dataset as CURRENT) avoids `EXDEV`.
  The existing `write_atomic` already does this.
- `sync_parent` (fsync of the vault dir after rename) is called in `write_atomic`;
  this is correct on Linux. On ZFS, ensure the pool does not have `sync=disabled`.
- Recovery re-applying WAL records: a WAL record may contain a write for a key
  that already exists in the SST (from before the last manifest). The re-apply
  must not corrupt existing rows — use the MVCC `commit_batch` which handles
  overwrites correctly.
- `degraded_rebuildable` flag: if a derived CF (ANN, xterm) is corrupt, set the
  flag in the manifest and allow reads; do not block reads of base/slot CFs.
