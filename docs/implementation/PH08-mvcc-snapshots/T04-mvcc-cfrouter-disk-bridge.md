# PH08 · T04 — MVCC+CfRouter write bridge: disk persistence under seq

| Field | Value |
|---|---|
| **Phase** | PH08 — MVCC sequence numbers + snapshot reads |
| **Stage** | S1 — Aster storage core |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/mvcc/store.rs` (≤500), `crates/calyx-aster/src/vault.rs` (≤500) |
| **Depends on** | T02 (snapshot isolation), PH07 T03 (CfRouter) |
| **Axioms** | A15, A26 |
| **PRD** | `dbprdplans/04 §5/§6` |

## Goal

Bridge `VersionedCfStore::commit_batch` to the on-disk `CfRouter` so that every
write is persisted to the correct CF directory. After a `commit_batch`, the
written rows must be readable both from the in-memory version chain (at the
committed seq) and from the on-disk SST (after a `flush_cf`). The vault's
`seq_allocator` initial value must be settable from a recovered WAL sequence
(PH10 sets this; add the setter here).

## Build (checklist of concrete, code-level steps)

- [x] Add `VersionedCfStore::new_with_router(start_seq, cf_router: CfRouter)`:
  stores the router; in `commit_batch`, after inserting into the in-memory row
  table, also calls `cf_router.put(cf, key, value)` for each row.
- [x] Add `VersionedCfStore::set_start_seq(&self, seq: Seq)`: atomically stores
  the seq allocator's current value (for post-recovery reset by PH10). Only
  callable before any allocations in the current session.
- [x] Add `VersionedCfStore::flush_all_cfs(&mut self) -> Result<()>`: calls
  `cf_router.flush_cf(cf)` for every CF that has a non-empty memtable.
- [x] Update `AsterVault::with_clock` to accept an optional `CfRouter`; when
  provided, use `VersionedCfStore::new_with_router`.
- [x] Write integration test: `AsterVault::put(cx)` → `flush_all_cfs()` → confirm
  SST files exist in `vault_dir/cf/base/` and `vault_dir/cf/slot_00/`; open
  `CfRouter::get(Base, base_key(cx.cx_id))` independently and confirm the value
  is present.
- [x] Write test: after `flush_all_cfs()`, a new `VersionedCfStore` (cold open)
  initialized with the same `CfRouter` from the vault dir can read back the rows
  (testing that disk is the SoT, not in-memory).

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: put one constellation; flush; assert SST file exists at expected path
  and contains the expected key bytes.
- [x] unit: cold-open vault (new store, same dir): get the constellation back via
  `CfRouter::get` byte-exact.
- [x] unit: `set_start_seq(recovered_seq)` sets the allocator; next `commit_batch`
  allocates `recovered_seq + 1`.
- [x] edge (≥3): (1) put + no flush → data in memtable but not in SST; (2)
  flush twice → second SST file created; (3) cold open on empty vault dir →
  no error, empty store.
- [x] fail-closed: `set_start_seq` called after an allocation panics (or returns
  Err) — document the constraint.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `vault/cf/base/000001.sst` and `vault/cf/slot_00/000001.sst`.
- **Readback:**
  ```
  ls /home/croyse/calyx/test-vault/cf/
  calyx readback --cf base --vault /home/croyse/calyx/test-vault
  calyx readback --cf slot_00 --vault /home/croyse/calyx/test-vault
  ```
- **Prove:** After `AsterVault::put` + `flush_all_cfs`, each CF directory contains
  an SST file; `calyx readback` shows the written rows byte-exact. A cold-open
  vault on the same directory reads back the same rows without any in-memory
  state. Screenshot posted to PH08 GitHub issue.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH08 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
