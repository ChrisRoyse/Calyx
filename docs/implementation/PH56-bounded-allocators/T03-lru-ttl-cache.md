# PH56 · T03 — LRU+TTL byte-capped cache — every cache an LRU with byte cap

| Field | Value |
|---|---|
| **Phase** | PH56 — Bounded caches/queues/memtables + arenas/pools |
| **Stage** | S13 — Resource, GC & Reliability Hardening |
| **Crate** | `calyx-core` |
| **Files** | `crates/calyx-core/src/cache/lru_ttl.rs` (≤500) |
| **Depends on** | T01 (alloc module; `Clock` trait for injected TTL time) |
| **Axioms** | A26, A16 |
| **PRD** | `dbprdplans/24 §1`, `24 §6` |

## Goal

Provide a single generic `LruTtlCache<K, V>` that every cached artifact in Calyx uses — lazy
cross-terms, query plans, autotune configs, kernel results, and any future cache. The cache has
a hard byte cap, an LRU eviction policy, and a per-entry TTL. No cache in the system is
unbounded. Uses the injected `Clock` trait (never `SystemTime::now()` in logic) so tests are
deterministic. Returns `CALYX_CACHE_EVICTED` to the metric surface on eviction.

## Build (checklist of concrete, code-level steps)

- [ ] Define `struct LruTtlCache<K, V> { map: IndexMap<K, CacheEntry<V>>, byte_cap: usize, used_bytes: usize, clock: Arc<dyn Clock> }` where `CacheEntry<V>` stores `value: V, inserted_at: Instant, size_bytes: usize`
- [ ] Implement `LruTtlCache::new(byte_cap: usize, ttl: Duration, clock: Arc<dyn Clock>) -> Self`; `byte_cap == 0` returns `CALYX_ALLOC_CAP_EXCEEDED`
- [ ] Implement `fn get(&mut self, key: &K) -> Option<&V>` — checks TTL first (expired → evict + return None); then promotes to MRU end; returns reference
- [ ] Implement `fn insert(&mut self, key: K, value: V, size_bytes: usize) -> InsertResult` — if `size_bytes > byte_cap` return `CALYX_ALLOC_CAP_EXCEEDED` (single entry too large); evict LRU entries until `used_bytes + size_bytes <= byte_cap`; insert; return `InsertResult { evicted: usize }` with eviction count for metrics
- [ ] Implement `fn evict_expired(&mut self) -> usize` — sweep for TTL-expired entries, remove, decrement `used_bytes`; call on every `insert` and `get`
- [ ] Implement `fn len(&self) -> usize`, `fn used_bytes(&self) -> usize`, `fn hit_rate(&self) -> f64` (hits / (hits + misses))
- [ ] Emit `CALYX_CACHE_EVICTED` structured log event (not a panic) on each eviction with `key` type name + `size_bytes`
- [ ] TTL jitter: add optional `jitter: Duration` to `new`; randomize each entry's TTL by `±jitter/2` using seeded RNG (prevents cache stampede thundering-herd — hazard 15)

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: insert 10 entries of 100 bytes each into a 500-byte cap cache → first 5 inserted; 6th insert evicts LRU (entry 0) to make room; `used_bytes == 500`
- [ ] unit: TTL expiry — insert entry at `clock.now()`, advance mock clock by `ttl + 1ms`, `get` returns None; `used_bytes` decremented
- [ ] unit: LRU order — insert A, B, C; `get(A)` promotes A; insert D that evicts LRU → B is evicted (not A); verify via `len()`
- [ ] proptest: `forall byte_cap, entries: Vec<(key, size)>` — `used_bytes` never exceeds `byte_cap` after any sequence of inserts
- [ ] unit: `hit_rate()` — 10 inserts, 10 matching gets → `hit_rate == 1.0`; 10 misses → `hit_rate == 0.0`
- [ ] edge: single entry exactly `byte_cap` bytes → inserted, `used_bytes == byte_cap`; insert another → first evicted
- [ ] edge: entry `size_bytes > byte_cap` → `CALYX_ALLOC_CAP_EXCEEDED` immediately, cache unmodified
- [ ] edge: `byte_cap == 0` → `new` returns `CALYX_ALLOC_CAP_EXCEEDED`
- [ ] fail-closed: insert into full cache (all entries have TTL not yet expired) → LRU eviction happens deterministically, no panic, `CALYX_CACHE_EVICTED` event emitted

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** Prometheus metrics `calyx_cache_used_bytes` and `calyx_cache_evictions_total` on aiwonder, and the cache's `used_bytes()` read during the 1e7-op soak
- **Readback:** `calyx readback --metric cache_used_bytes` — must remain ≤ `byte_cap` throughout the soak; `calyx readback --metric cache_evictions_total` — must be non-zero (eviction is running) and stable (not monotonically growing unbounded)
- **Prove:** before/after the soak, `used_bytes` is bounded — not growing without bound. Inject a cache-flood (insert 10× cap worth of entries) → `used_bytes` stays at `byte_cap`, eviction log shows entries dropped.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH56 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
