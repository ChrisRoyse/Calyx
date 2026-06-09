# PH16 · T01 — `AutotuneCache` key type + CRUD + persist/load

| Field | Value |
|---|---|
| **Phase** | PH16 — Autotune Config Cache |
| **Stage** | S2 — Forge Math Runtime |
| **Crate** | `calyx-forge` |
| **Files** | `crates/calyx-forge/src/autotune.rs` (≤500) |
| **Depends on** | PH12 T01 (BestConfig, BackendKind) |
| **Axioms** | A14, A16 |
| **PRD** | `dbprdplans/12 §4`, `dbprdplans/13 §7` |

## Goal

Define the `AutotuneKey` (the 5-tuple `(op, shape, dtype, device, recall_tgt)`)
and the `AutotuneCache` struct with CRUD operations and atomic persist/load
to/from a JSON file. The cache is the stable read path that Anneal (PH43+) calls
on every Forge dispatch; writes only happen during exploration (T03).

## Build (checklist of concrete, code-level steps)

- [x] `pub struct AutotuneKey { pub op: String, pub shape: Vec<usize>, pub dtype: String, pub device: String, pub recall_tgt: f32 }`
  — `serde::{Serialize, Deserialize}`, `Clone`, `Debug`, `PartialEq`, `Eq`, `Hash`;
  `recall_tgt` is quantized to nearest 0.01 for hashing
  (`impl Hash`: hash `(op, shape, dtype, device, (recall_tgt * 100.0) as u32)`)
- [x] `pub struct AutotuneCache { entries: HashMap<AutotuneKey, BestConfig>, path: PathBuf }`
- [x] `pub fn load(path: &Path) -> Result<AutotuneCache, ForgeError>`
  — if file not found → return empty cache (not an error);
  if file malformed → `ForgeError::QuantError { op: "autotune_load", detail: "json parse error: {e}" }`
  (reusing the quant error variant is wrong — add `ForgeError::CacheError { ... }` → `CALYX_FORGE_CACHE_ERROR`)
- [x] `pub fn get(&self, key: &AutotuneKey) -> Option<&BestConfig>`
- [x] `pub fn insert(&mut self, key: AutotuneKey, config: BestConfig)`
- [x] `pub fn persist(&self) -> Result<(), ForgeError>`
  — write to `<path>.tmp` then `fs::rename` atomically; if rename fails on same-fs
  → `ForgeError::CacheError`; on ZFS, ensure tmp file is in the same dataset
  (avoid `EXDEV` — stage temp file in same dir as `path`)
- [x] `pub fn rollback(&mut self, key: &AutotuneKey, previous: BestConfig)`
  — replaces current config with `previous`; used by T04 reversibility
- [x] `AutotuneKey::default_for(op: &str, shape: &[usize], dtype: &str, device: &str) -> AutotuneKey`
  — `recall_tgt=0.95` default

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `insert` + `get` round-trip for a known key → returns the inserted `BestConfig`
- [x] unit: `persist` writes a file then `load` from that path returns the same entry
- [x] unit: `load` from non-existent path → empty cache, no error
- [x] proptest: `AutotuneKey` with `recall_tgt=0.951` and `recall_tgt=0.954` hash to
  the same value (quantized to 0.01); `recall_tgt=0.95` and `recall_tgt=0.96` hash differently
- [x] edge (≥3): (1) empty cache persist → valid JSON file; (2) malformed JSON load → `CacheError`;
  (3) `rollback` on key not in cache → inserts the previous config (no error)
- [x] fail-closed: `persist` to a read-only path → `ForgeError::CacheError` with path in detail

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `autotune_tests::persist_load_roundtrip` + `rollback_restores_previous` on aiwonder
- **Readback:**
  ```bash
  cargo test -p calyx-forge autotune::tests::persist -- --nocapture 2>&1 \
    | grep -E "PASSED|FAILED|CALYX_FORGE_CACHE"
  ls -la /tmp/calyx_autotune_test_*.json 2>/dev/null
  ```
- **Prove:** `persist_load_roundtrip` PASSED; the `.json` file exists and is valid JSON
  (grep for `"op"` key); absent: any `CALYX_FORGE_CACHE_ERROR` in the success path

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (test output + JSON file listing) attached to PH16 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
