# PH06 · T05 — Multi-SST level: newest-first point lookup + range merge

| Field | Value |
|---|---|
| **Phase** | PH06 — Memtable + LSM SSTable writer/reader |
| **Stage** | S1 — Aster storage core |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/sst/level.rs` (≤500) |
| **Depends on** | T02 (SstReader), T03 (bloom) |
| **Axioms** | A26 |
| **PRD** | `dbprdplans/04 §2/§8` |

## Goal

Implement `SstLevel`: an ordered collection of SST files for one column family
(newest first) that supports (a) point lookup — returns the first match scanning
newest-to-oldest, using bloom filters to skip files; (b) range merge — returns a
deduplicated sorted merge of all matching entries across files (newest version
wins). This is the per-CF read layer used by PH07 and PH09.

## Build (checklist of concrete, code-level steps)

- [x] Define `SstLevel { files: Vec<PathBuf> }` where `files[0]` is the newest
  (most recently flushed) SST.
- [x] `SstLevel::push(&mut self, path: PathBuf)` appends the newest SST at the
  front (`files.insert(0, path)`).
- [x] `SstLevel::get(&self, key: &[u8]) -> Result<Option<Vec<u8>>>`: iterate
  `files` newest-to-oldest; open reader and check bloom; on bloom hit do index
  lookup; return first value found. If no file returns Some, return Ok(None).
- [x] `SstLevel::range(&self, start: &[u8], end: &[u8]) -> Result<Vec<SstEntry>>`:
  collect all matching entries from all files, deduplicate by key keeping newest,
  return sorted ascending by key.
- [x] `SstLevel::file_count(&self) -> usize`.
- [x] Ensure `SstLevel::get` opens each `SstReader` lazily per call (no persistent
  mmap handles in `SstLevel`) — the compaction catalog (`CompactionCatalog`) holds
  the production lifetime; `SstLevel` is the per-CF thin query layer for PH07.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: 2 SSTs, key `b"k1"` exists in both; `get(b"k1")` returns the value
  from the newer SST (file 0).
- [x] unit: 3 SSTs with ranges `[k1,k3]`, `[k2,k4]`, `[k5,k6]`; `range(k1,k7)`
  returns k1..k6 deduplicated in ascending order with newest version for k2,k3,k4.
- [x] proptest: for any non-empty set of `(key,value)` pairs split across 1..=4
  SSTs with deterministic seed: `SstLevel::get` returns the latest value for
  every key; `range(min, max)` returns all keys sorted.
- [x] edge (≥3): (1) empty level → `get` returns Ok(None), `range` returns empty
  vec; (2) single SST with 1 entry → `get` round-trips; (3) key present only in
  oldest SST (bloom miss in newer ones) → `get` still finds it.
- [x] fail-closed: corrupt SST in level → `get` returns `CALYX_ASTER_CORRUPT_SHARD`
  (not silently skips).

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** CF directory at `/home/croyse/calyx/test-vault/cf/base/` with ≥2 SST
  files.
- **Readback:**
  ```
  ls -la /home/croyse/calyx/test-vault/cf/base/*.sst
  calyx readback --cf base --level /home/croyse/calyx/test-vault/cf/base/
  ```
- **Prove:** `calyx readback` shows each key once (newest version), in ascending
  order. The returned value for any key matches the value in the newest SST
  containing that key (verified by separately running `calyx readback --sst <path>`
  on each file).

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH06 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
