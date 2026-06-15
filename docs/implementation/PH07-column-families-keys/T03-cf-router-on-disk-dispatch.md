# PH07 · T03 — CF router: per-CF SstLevel + on-disk put/get dispatch

| Field | Value |
|---|---|
| **Phase** | PH07 — Column families + key encoding |
| **Stage** | S1 — Aster storage core |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/cf/router.rs` (≤500), `crates/calyx-aster/src/cf/mod.rs` (≤500) |
| **Depends on** | T01 (codecs), T02 (CxId), PH06 T05 (SstLevel) |
| **Axioms** | A16, A26 |
| **PRD** | `dbprdplans/04 §4` |

## Goal

Implement `CfRouter`: the component that maps a `ColumnFamily` to its on-disk
directory, maintains one `Memtable` per CF, dispatches writes to the correct CF
memtable, flushes to SST when the memtable hits capacity, and reads through the
`SstLevel` + active memtable for a complete per-CF KV store. This is the disk
bridge between the in-memory `VersionedCfStore` (PH08 MVCC) and the physical SST
files.

## Build (checklist of concrete, code-level steps)

- [x] Define `CfRouter` in `cf/router.rs`:
  - `vault_dir: PathBuf`
  - `memtables: HashMap<ColumnFamily, Memtable>`
  - `levels: HashMap<ColumnFamily, SstLevel>`
  - `memtable_byte_cap: usize` (default: 8 MiB).
- [x] `CfRouter::open(vault_dir, memtable_byte_cap)`: creates `vault_dir/cf/<name>/`
  for every known static CF; loads existing SST files into `SstLevel` per CF.
- [x] `CfRouter::put(&mut self, cf, key, value) -> Result<()>`: insert into the
  CF's `Memtable`; if `needs_flush()`, call `flush_cf(cf)`.
- [x] `CfRouter::flush_cf(&mut self, cf) -> Result<SstSummary>`: freeze the CF's
  memtable, write it to an SST with a monotonic filename in `vault_dir/cf/<name>/`,
  push the new SST to the CF's `SstLevel`, replace the memtable with a fresh
  empty one.
- [x] `CfRouter::get(&self, cf, key) -> Result<Option<Vec<u8>>>`: check active
  memtable first, then `SstLevel::get`.
- [x] `CfRouter::range(&self, cf, start, end) -> Result<Vec<SstEntry>>`: merge
  active memtable entries in range with `SstLevel::range`; deduplicate (memtable
  wins); return sorted.
- [x] SST file naming: `<seq:020>.sst` where seq is the next monotonic counter per
  CF (start at 1, persist nowhere — count existing files on open + 1).
- [x] Re-export `CfRouter` from `cf/mod.rs`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: open router in tempdir; `put(Base, b"k1", b"v1")`; `get(Base, b"k1")`
  returns `Some(b"v1")`; no SST file yet (below flush threshold).
- [x] unit: fill memtable to capacity; `put` triggers flush; SST file appears in
  `vault_dir/cf/base/`; `get` still returns correct values from SST.
- [x] unit: write k1 in SST and k2 only in memtable; `range(b"", b"\xff")` returns
  both in sorted order.
- [x] edge (≥3): (1) router opened on existing vault with SST files → reads them
  back; (2) two CFs written independently → each CF dir contains only its own SST;
  (3) `get` on absent key returns Ok(None).
- [x] fail-closed: corrupt SST in CF dir → `get` returns `CALYX_ASTER_CORRUPT_SHARD`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** CF directories at `/home/croyse/calyx/test-vault/cf/base/` and
  `/home/croyse/calyx/test-vault/cf/slot_00/`.
- **Readback:**
  ```
  ls /home/croyse/calyx/test-vault/cf/base/
  ls /home/croyse/calyx/test-vault/cf/slot_00/
  calyx readback --cf base --vault /home/croyse/calyx/test-vault
  ```
- **Prove:** Each CF has its own subdirectory; SST file exists after a flush;
  `calyx readback` returns the written key/value byte-exact. The `base` and
  `slot_00` SST files are independent (no cross-contamination).

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH07 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
