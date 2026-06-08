# PH34 T02 - `ScopeCache`: identity-aware LRU cache

| Field | Value |
|---|---|
| **Phase** | PH34 - Multi-scope kernel |
| **Stage** | S6 - Lodestar Kernel |
| **Crate** | `calyx-lodestar` |
| **Files** | `crates/calyx-lodestar/src/scope_cache.rs` (<=500) |
| **Depends on** | T01 (`scope_hash`, `Scope`) |
| **Axioms** | A21, A26 |
| **PRD** | `dbprdplans/08 section 4b` ("caches by `(scope_hash, panel_version)` and updates incrementally"); implementation also keys anchors and corpus identity so semantic variants never reuse stale kernels |

## Goal

Implement `ScopeCache`: a bounded LRU in-memory cache mapping
`(scope_hash, panel_version, anchor_identity, corpus_identity)` to a previously
computed `Kernel`. Cache hits avoid full pipeline re-runs; misses trigger
`build_kernel_pipeline`. Hit/miss counters expose reuse behavior for FSV and
observability.

## Status

Implemented in issue #234 and hardened in #328. Base aiwonder FSV readbacks live
under `/home/croyse/calyx/data/fsv-issue234-scope-cache-20260608`; the identity
hardening readbacks live under
`/home/croyse/calyx/data/fsv-issue328-scope-cache-identity-20260608`.

## Build

- [x] `ScopeCacheKey { scope_hash, panel_version, anchor_identity, corpus_identity }`.
- [x] `ScopeCache` stores bounded `(ScopeCacheKey, Kernel)` entries with explicit
  LRU order, `max_entries`, `hits`, and `misses`.
- [x] `get(&mut self, key)` increments `hits` on hit and `misses` on miss.
- [x] `insert(key, kernel)` inserts into the LRU and evicts oldest entries over
  capacity.
- [x] `invalidate_panel_version(old_version)` removes only matching panel
  entries.
- [x] `stats()` returns `CacheStats { hits, misses, current_size, max_entries }`.
- [x] `max_entries = 0` is safe and stores nothing.
- [x] Eviction logs include scope hash, panel version, anchor identity, and
  corpus identity.
- [x] `scope_cache_anchor_identity(anchor_kinds, anchors)` deterministically
  hashes framed anchor kinds and resolved anchor IDs.

## Tests

- [x] unit: insert 3 kernels; `get` each -> hits = 3.
- [x] unit: `get` a missing key -> `None`; misses = 1.
- [x] unit: `max_entries = 2`; insert 3 entries -> first inserted is evicted.
- [x] unit: `invalidate_panel_version(v1)` removes v1 entries and leaves v2.
- [x] edge: `max_entries = 0` leaves cache empty with no panic.
- [x] edge: `panel_version = u64::MAX` still functions.
- [x] regression: same `scope_hash + panel_version` with a different anchor
  identity is a miss.
- [x] regression: same `scope_hash + panel_version + anchor_identity` with a
  different corpus identity is a miss.

## FSV

- **Base trigger:** `CALYX_FSV_ROOT=/home/croyse/calyx/data/fsv-issue234-scope-cache-20260608 cargo test -p calyx-lodestar --test ph34_scope_cache_tests -- --nocapture --test-threads=1`
- **Base readbacks:**
  - `eviction/ph34-scope-cache-eviction-readback.json`: `first_absent=true`,
    `second_present=true`, `third_present=true`, `current_size=2`.
  - `stats/ph34-scope-cache-stats-readback.json`: `hits=3`, `misses=1`,
    `current_size=3`, `max_entries=4`.
  - `invalidate/ph34-scope-cache-invalidate-readback.json`: `removed=2`,
    `v1_absent=true`, `v2_present=true`, `current_size=1`.
  - `edges/ph34-scope-cache-edges-readback.json`: `zero_capacity_size=0`,
    `max_panel_present=true`, `max_panel_version=u64::MAX`.
- **Identity follow-up (#328):**
  `/home/croyse/calyx/data/fsv-issue328-scope-cache-identity-20260608/scope-cache-identity/ph34-scope-cache-identity-readback.json`
  proves `build_kernel` produced final stats `hits=2`, `misses=3`,
  `current_size=3` after domain hit, alternate-anchor miss/hit, and
  changed-corpus miss.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) <= 500 lines
- [x] FSV evidence attached to the PH34/#328 GitHub issues
- [x] no anti-pattern: no flatten / no ungrounded trusted state / no harness-only FSV
