# PH09 · T02 — WAL-integrated vault write path

| Field | Value |
|---|---|
| **Phase** | PH09 — Constellation CRUD + CxId + idempotent ingest |
| **Stage** | S1 — Aster storage core |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/vault.rs` (≤500) |
| **Depends on** | T01 (binary encoding), PH05 T03 (GroupCommitBatcher), PH08 T04 (MVCC+CfRouter bridge) |
| **Axioms** | A15, A16 |
| **PRD** | `dbprdplans/04 §5` |

## Goal

Wire the vault `put` path so the write sequence is:
1. `cx_id = blake3(input ‖ panel_version ‖ salt)[0..16]`; dedup check on disk.
2. Encode CF rows with binary codecs.
3. `GroupCommitBatcher::submit(wal_payload)` → fsync ack (WAL is durable).
4. `VersionedCfStore::commit_batch(rows)` → MVCC seq advanced + CfRouter write.
5. Ack to caller with `Ok(cx_id)`.

Fail closed at any step: if the WAL submit fails, the CF rows are never committed.
If `commit_batch` fails, the WAL record is orphaned (recovered by PH10 replay).

## Build (checklist of concrete, code-level steps)

- [x] Add `AsterVault::new_durable(vault_dir, vault_salt, wal_options)` that
  opens a `GroupCommitBatcher`-backed WAL in `vault_dir/wal/`, a `CfRouter`
  rooted at `vault_dir/`, and wires them into `VersionedCfStore::new_with_router`.
- [x] In `AsterVault::put`: build `wal_payload = encode_write_batch(cx)` (a
  binary blob listing all CF rows); call `batcher.submit(wal_payload)` (blocks
  until fsync ack); only then call `self.rows.commit_batch(rows)`.
- [x] Define `encode_write_batch` / `decode_write_batch` in `vault.rs` or
  `vault/encode.rs`: a binary format listing `n_rows (u32 BE) | [(cf_tag (u8),
  key_len (u32 BE), key, value_len (u32 BE), value), ...]`.
- [x] Ensure `put` returns `Err` if `batcher.submit` fails; the in-memory MVCC
  table is NOT mutated if the WAL fails.
- [x] Add `AsterVault::flush(&self) -> Result<()>` that calls
  `rows.flush_all_cfs()` to persist all memtable data to SST.
- [x] Update `AsterVault::get` to read from `CfRouter` (disk) when in-memory
  lookup misses (for cold-open vaults with data only on disk).

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `put(cx)` with durable vault → WAL segment file exists with ≥1 record;
  `get(cx.cx_id, snapshot)` returns the constellation byte-exact.
- [x] unit: vault process cold-open (new `AsterVault::new_durable` on same dir
  after flush): `get` returns the constellation from disk.
- [x] unit: WAL failure (write to a read-only WAL dir) → `put` returns Err; MVCC
  seq unchanged; no CF rows written.
- [x] edge (≥3): (1) empty vault cold-open → no error, `get` returns Err (not
  found); (2) two puts, one flush → both readable; (3) put of constellation with
  15 slots → all 15 slot CFs written.
- [x] fail-closed: WAL error → `CALYX_DISK_PRESSURE`; no partial state.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** WAL segment at `vault/wal/00000000000000000000.wal` and SST at
  `vault/cf/base/000001.sst`.
- **Readback:**
  ```
  xxd /home/croyse/calyx/test-vault/wal/00000000000000000000.wal | head -4
  calyx readback --cf base --vault /home/croyse/calyx/test-vault
  ```
- **Prove:** WAL segment exists and contains ≥1 complete record (magic `CXW1`
  at offset 0); `calyx readback` returns the ingested constellation key/value
  byte-exact. After a cold-open vault, the same result is returned. Screenshot
  posted to PH09 GitHub issue.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH09 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
