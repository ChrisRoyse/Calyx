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

## Status — DONE ✅ (Stage 1; FSV-signed-off 2026-06-07, commit 8dcddaa)

Shipped in `calyx-aster`:
- `manifest/mod.rs` — `VaultManifest` (version/manifest_seq/durable_seq/panel_ref/codebook_refs/degraded_rebuildable), `ManifestStore::write_current`/`load_current` via atomic temp+rename+`sync_parent`, `ManifestVersion::validate` (rejects bad major), `ImmutableRef` path-traversal guards, `recover_vault` (replays WAL past `durable_seq` → `RecoveryOutcome{wal_records,torn_tail,last_recovered_seq,degraded_rebuildable}`).
- Corrupt base shard read fails closed → `CALYX_ASTER_CORRUPT_SHARD` + restic/snapshot restore guidance. CLI: `crash-drill` (3 points), `recover`, `corrupt-shard`.
- FSV-proven: SIGKILL crash drill recovered to last-acked seq + reported `CALYX_ASTER_TORN_WAL` (WAL truncated 790→774 bytes); corrupt base SST failed closed exit 2.
- Sweep residual #337 adds `AsterVault::recovery_report()` so normal cold-open callers can inspect the same torn-tail diagnostic without going through the CLI-only recovery path.

FSV evidence: GitHub issue #23 (`[CONTEXT] You are here`); Stage-1 evidence root `/home/croyse/calyx/data/fsv-stage1-exit-20260607105216`.

### Follow-ups
1. `manifest/recovery.rs` was not created — recovery logic lives in `manifest/mod.rs::recover_vault`. This is cosmetic module placement, not a behavior gap.
2. ✅ `AsterVault::open` now uses manifest-anchored recovery through `recover_vault`, restores batches at their original seqs, and calls `set_start_seq(recovery.last_recovered_seq)`.
2a. ✅ `AsterVault::open` now preserves recovery metadata in `VaultRecoveryReport`, including `last_recovered_seq` and optional `CALYX_ASTER_TORN_WAL` details.
3. `degraded_rebuildable` is a manifest field but is **never set true** on a corrupt derived CF; the self-heal/degrade path is deferred to PH44.
4. ✅ Durable writes, MVCC router flushes, compaction catalogs, and scheduler paths are unified through `vault/commit.rs`, `vault/compaction_bridge.rs`, and #295's `VaultOptions::tiering_policy` wiring.

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

> ✅ **Achieved** — byte-proven on aiwonder; evidence in GitHub issue #23 (Stage-1 FSV root `/home/croyse/calyx/data/fsv-stage1-exit-20260607105216`).

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
