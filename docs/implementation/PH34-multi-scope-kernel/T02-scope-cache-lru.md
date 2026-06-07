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

## Build (checklist of concrete, code-level steps)

- [ ] `pub struct ScopeCacheKey { scope_hash: [u8; 32], panel_version: u64 }`.
- [ ] `pub struct ScopeCache { entries: LruCache<ScopeCacheKey, Kernel>, max_entries: usize, hits: u64, misses: u64 }`.
- [ ] `pub fn get(&mut self, key: &ScopeCacheKey) -> Option<&Kernel>` — LRU lookup;
  increments `hits` on hit, `misses` on miss.
- [ ] `pub fn insert(&mut self, key: ScopeCacheKey, kernel: Kernel)` — inserts into LRU;
  evicts oldest entry if `len() >= max_entries`.
- [ ] `pub fn invalidate_panel_version(&mut self, old_version: u64)` — removes all
  entries with `panel_version == old_version` (panel rotation).
- [ ] `pub fn stats(&self) -> CacheStats { hits, misses, current_size, max_entries }`.
- [ ] `max_entries` default = 128; configurable at construction.
- [ ] `ScopeCache` is `Send + Sync` (wraps in `Arc<Mutex<_>>` if needed for shared use).
- [ ] Eviction: when evicting, emit a structured log entry with the evicted scope hash
  (for observability); no panic on eviction.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: insert 3 kernels; `get` each → hits = 3, misses = 0.
- [ ] unit: `get` a key not in cache → `None`; misses = 1.
- [ ] unit: `max_entries = 2`; insert 3 entries → first inserted is evicted (LRU);
  `get(first_key)` = `None`.
- [ ] unit: `invalidate_panel_version(v1)` with 2 entries at v1 and 1 at v2 →
  2 entries removed; `get` of v1 entries = None; v2 entry still present.
- [ ] unit: `stats()` returns `current_size == 1` after inserting 1 and evicting 0.
- [ ] edge: `max_entries = 0` → every insert immediately evicts; cache always empty;
  no panic.
- [ ] fail-closed: `panel_version` overflow (u64::MAX) → cache still functions;
  no arithmetic panic.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `cargo test -p calyx-lodestar scope_cache -- --nocapture` stdout.
- **Readback:** `cargo test -p calyx-lodestar scope_cache 2>&1 | tee /tmp/ph34_t02_fsv.txt && cat /tmp/ph34_t02_fsv.txt`.
- **Prove:** eviction test prints that first-inserted key is absent after 3 inserts
  into a capacity-2 cache; `stats()` prints correct hit/miss counts; all tests pass;
  output attached to PH34 GitHub issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH34 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
