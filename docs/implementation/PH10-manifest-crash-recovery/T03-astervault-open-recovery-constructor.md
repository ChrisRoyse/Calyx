# PH10 · T03 — AsterVault::open — recovery constructor

| Field | Value |
|---|---|
| **Phase** | PH10 — Manifest + atomic swap + crash recovery |
| **Stage** | S1 — Aster storage core |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/vault.rs` (≤500), `crates/calyx-aster/src/manifest/recovery.rs` (≤500) |
| **Depends on** | T01 (manifest), T02 (WAL replay), PH08 T04 (seq setter) |
| **Axioms** | A15, A16 |
| **PRD** | `dbprdplans/04 §7` |

## Goal

Implement `AsterVault::open(vault_dir, vault_salt, options) -> Result<Self>`:
the full cold-open constructor that (1) reads MANIFEST + CURRENT, (2) replays WAL
past `durable_seq`, (3) reconstructs in-memory MVCC state, (4) sets
`SeqAllocator` to `last_recovered_seq`, and (5) returns a fully operational vault.
If MANIFEST is absent (first open), initialise an empty vault. This makes the
vault usable across process restarts.

## Build (checklist of concrete, code-level steps)

- [ ] In `AsterVault`, add `open(vault_dir: PathBuf, vault_salt: Vec<u8>,
  options: VaultOptions) -> Result<Self>`:
  - Create `CfRouter::open(vault_dir.clone(), options.memtable_byte_cap)`.
  - If `CURRENT` exists: `recover_vault(&vault_dir)?` → `reconstruct_from_recovery
    (outcome, &mut cf_router)?` → `start_seq = last_recovered_seq`.
  - If `CURRENT` absent: `start_seq = 0`.
  - Build `VersionedCfStore::new_with_router(start_seq, cf_router)`.
  - Wire `GroupCommitBatcher` for WAL writes.
  - Return the vault.
- [ ] Add `VaultOptions`: `memtable_byte_cap`, `wal_options`, `clock` (injectable).
- [ ] After `open`, the vault's `snapshot()` returns `last_recovered_seq` (not 0).
- [ ] Write test: put N constellations; flush; call `AsterVault::open` on the same
  vault dir; `snapshot()` == N; `get` all N constellations byte-exact.
- [ ] Write test: `open` on a directory with a manifest but no WAL records after
  `durable_seq` → vault starts at `durable_seq`, reads SST data correctly.
- [ ] Write test: `open` on an empty directory (first launch) → no error;
  `snapshot() == 0`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: put+flush → open → get: byte-exact.
- [ ] unit: open on empty dir → no error, snapshot=0.
- [ ] unit: open with WAL records after durable_seq → snapshot = last_recovered_seq.
- [ ] edge (≥3): (1) open twice on same dir sequentially (no crash) → second open
  recovers cleanly; (2) open with torn WAL tail → recovers to last acked; (3)
  corrupt MANIFEST → `CALYX_ASTER_CORRUPT_SHARD`.
- [ ] fail-closed: corrupt MANIFEST → `CALYX_ASTER_CORRUPT_SHARD` on `open`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `vault/cf/base/000001.sst` and `vault/wal/` after `AsterVault::open`.
- **Readback:**
  ```
  calyx recover --vault /home/croyse/calyx/test-vault
  calyx readback --cf base --vault /home/croyse/calyx/test-vault
  cat /home/croyse/calyx/test-vault/CURRENT
  ```
- **Prove:** After `open`, `snapshot()` equals the value of `last_recovered_seq`
  (printed by `calyx recover`); `calyx readback` shows all constellations from
  before the process restart, byte-exact. CURRENT points at the correct manifest
  file. Screenshot posted to PH10 GitHub issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH10 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
