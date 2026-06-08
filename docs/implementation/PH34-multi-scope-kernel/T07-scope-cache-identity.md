# PH34 T07 - Scope-cache identity includes anchors and corpus

| Field | Value |
|---|---|
| **Phase** | PH34 - Multi-scope kernel |
| **Stage** | S6 - Lodestar Kernel |
| **Crate** | `calyx-lodestar` |
| **Files** | `crates/calyx-lodestar/src/scope_cache.rs` (<=500), `crates/calyx-lodestar/src/multi_scope.rs` (<=500), `crates/calyx-lodestar/tests/ph34_scope_cache_identity_tests.rs` (<=500) |
| **Depends on** | T02, T03 |
| **Axioms** | A21, A26 |
| **PRD** | `dbprdplans/08 section 4b` |

## Goal

Prevent stale scoped kernels when two calls share `scope_hash` and
`panel_version` but differ by anchor kind, resolved anchor set, or backing
corpus/store shard. The cache key must cover the semantic inputs that change the
kernel produced by `build_kernel`.

## Status

Implemented in issue #328. aiwonder FSV readbacks live under
`/home/croyse/calyx/data/fsv-issue328-scope-cache-identity-20260608`.

## Build

- [x] Extend `ScopeCacheKey` with `anchor_identity` and `corpus_identity`.
- [x] Add `scope_cache_anchor_identity(anchor_kinds, anchors)` with framed,
  deterministic hashing of anchor kinds and resolved anchors.
- [x] Move `build_kernel` cache lookup after scope materialization and anchor
  resolution.
- [x] Use `params.corpus_shard_hash` as the corpus/store shard identity in the
  key.
- [x] Include the expanded identity in eviction logs.

## Tests

- [x] unit: same scope + panel + anchors + corpus hits on the second
  `build_kernel` call.
- [x] unit: same scope + panel + corpus but different anchor kind/anchor set
  misses and stores a second cache entry.
- [x] unit: same scope + panel + anchors but different corpus hash misses and
  stores a third cache entry.
- [x] FSV: ignored aiwonder test writes cache stats and per-run kernel rows.

## FSV

- **SoT:** `/home/croyse/calyx/data/fsv-issue328-scope-cache-identity-20260608/scope-cache-identity/ph34-scope-cache-identity-readback.json`
- **Readback:** `cat` the JSON on aiwonder, then `find` the evidence directory
  and run a targeted secret grep.
- **Prove:** final stats are `hits=2`, `misses=3`, `current_size=3` after the
  sequence domain miss -> domain hit -> alternate-anchor miss -> alternate-anchor
  hit -> changed-corpus miss.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) <= 500 lines
- [x] FSV evidence attached to GitHub issue #328
- [x] no stale docs still claim scope cache identity is only
      `(scope_hash, panel_version)`
