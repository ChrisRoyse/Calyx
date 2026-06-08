# PH34 · T02 — `ScopeCache`: `(scope_hash, panel_version)` LRU cache

| Field | Value |
|---|---|
| **Phase** | PH34 — Multi-scope kernel |
| **Stage** | S6 — Lodestar Kernel |
| **Crate** | `calyx-lodestar` |
| **Files** | `crates/calyx-lodestar/src/scope_cache.rs` (≤500) |
| **Depends on** | T01 (`scope_hash`, `Scope`) |
| **Axioms** | A21, A26 |
| **PRD** | `dbprdplans/08 §4b` ("caches by `(scope_hash, panel_version)` and updates incrementally") |

## Goal

Implement `ScopeCache`: an LRU in-memory cache mapping `(scope_hash, panel_version)`
to a previously computed `Kernel`. Cache hits avoid full pipeline re-runs; misses
trigger `build_kernel_pipeline`. The cache is bounded (A26: bounded memory) and
exposes hit/miss counters for observability.

## Status

Implemented in issue #234. aiwonder FSV readbacks live under
`/home/croyse/calyx/data/fsv-issue234-scope-cache-20260608`; the serial FSV log is
`ph34_t02_fsv.log`.

## Build (checklist of concrete, code-level steps)

- [x] `pub struct ScopeCacheKey { scope_hash: [u8; 32], panel_version: u64 }`.
- [x] `pub struct ScopeCache` stores bounded `(ScopeCacheKey, Kernel)` entries
  with an explicit LRU order, `max_entries`, `hits`, and `misses`.
- [x] `pub fn get(&mut self, key: &ScopeCacheKey) -> Option<&Kernel>` — LRU lookup;
  increments `hits` on hit, `misses` on miss.
- [x] `pub fn insert(&mut self, key: ScopeCacheKey, kernel: Kernel)` — inserts into LRU;
  evicts oldest entry if `len() >= max_entries`.
- [x] `pub fn invalidate_panel_version(&mut self, old_version: u64)` — removes all
  entries with `panel_version == old_version` (panel rotation).
- [x] `pub fn stats(&self) -> CacheStats { hits, misses, current_size, max_entries }`.
- [x] `max_entries` default = 128; configurable at construction.
- [x] `ScopeCache` is `Send + Sync` (wraps in `Arc<Mutex<_>>` if needed for shared use).
- [x] Eviction: when evicting, emit a structured log entry with the evicted scope hash
  (for observability); no panic on eviction.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: insert 3 kernels; `get` each → hits = 3.
- [x] unit: `get` a key not in cache → `None`; misses = 1.
- [x] unit: `max_entries = 2`; insert 3 entries → first inserted is evicted (LRU);
  `get(first_key)` = `None`.
- [x] unit: `invalidate_panel_version(v1)` with 2 entries at v1 and 1 at v2 →
  2 entries removed; `get` of v1 entries = None; v2 entry still present.
- [x] unit: `stats()` reports current size and hit/miss counters after lookups.
- [x] edge: `max_entries = 0` → every insert immediately evicts; cache always empty;
  no panic.
- [x] fail-closed: `panel_version` overflow (u64::MAX) → cache still functions;
  no arithmetic panic.

## FSV (read the bytes on aiwonder — the truth gate)

- **Trigger:** `CALYX_FSV_ROOT=/home/croyse/calyx/data/fsv-issue234-scope-cache-20260608 cargo test -p calyx-lodestar --test ph34_scope_cache_tests -- --nocapture --test-threads=1`
- **SoT readbacks:**
  - `eviction/ph34-scope-cache-eviction-readback.json`: `first_absent=true`,
    `second_present=true`, `third_present=true`, `current_size=2`.
  - `stats/ph34-scope-cache-stats-readback.json`: `hits=3`, `misses=1`,
    `current_size=3`, `max_entries=4`.
  - `invalidate/ph34-scope-cache-invalidate-readback.json`: `removed=2`,
    `v1_absent=true`, `v2_present=true`, `current_size=1`.
  - `edges/ph34-scope-cache-edges-readback.json`: `zero_capacity_size=0`,
    `max_panel_present=true`, `max_panel_version=u64::MAX`.
- **Hashes:** eviction
  `eff0266a019dede6926ef3d28a41e04f0930b29828f5f714cdfb10ceac4d8c8f`;
  stats `42b172fa5d3bc6b48eabc0664acf127c5585745790d310f7629aa0cd63276f33`;
  invalidate `f821c84910345e07e2e98e1e2a2b112ed0becee996826d30f27c5637b8a5c3d7`;
  edges `9f5a18c29289e9f84e7fe1ba9eb7c04b8b0b455619b8bb683a474fd6db66e756`;
  log `b17df830b5713384ba4d9aeffef1b14d5cc1d3da3feb67d39a6bef62a2865f40`.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH34 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
