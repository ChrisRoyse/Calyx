# PH56 · T01 — Arena/bump allocator — per-request and per-microbatch, O(1) reset

| Field | Value |
|---|---|
| **Phase** | PH56 — Bounded caches/queues/memtables + arenas/pools |
| **Stage** | S13 — Resource, GC & Reliability Hardening |
| **Crate** | `calyx-core` |
| **Files** | `crates/calyx-core/src/alloc/arena.rs` (≤500), `crates/calyx-core/src/alloc/mod.rs` (≤500) |
| **Depends on** | — |
| **Axioms** | A26, A16 |
| **PRD** | `dbprdplans/24 §1`, `24 §6` |

## Goal

Provide an arena/bump allocator that absorbs all per-request and per-microbatch transient
allocations (scoring buffers, cross-term working sets, MI working sets) without per-op
`malloc`/`free` churn. The arena resets in O(1) at request/batch end, eliminating fragmentation
and bounding transient heap growth. Every allocation has an owner (the arena) and a hard bound
(the arena cap) — A26 invariant at the allocation primitive level.

## Build (checklist of concrete, code-level steps)

- [ ] Define `struct Arena { buf: Vec<u8>, cursor: usize, cap: usize }` in `alloc/arena.rs`
- [ ] Implement `Arena::new(cap: usize) -> Self` — pre-allocates `cap` bytes; returns `CALYX_ALLOC_CAP_EXCEEDED` if `cap == 0`
- [ ] Implement `Arena::alloc(&mut self, size: usize, align: usize) -> Result<*mut u8, CalyxError>` — bump pointer with alignment padding; returns `CALYX_ALLOC_CAP_EXCEEDED` when `cursor + padded > cap` (fail closed, never realloc)
- [ ] Implement `Arena::reset(&mut self)` — sets `cursor = 0` in O(1); no `free` calls; safe because lifetimes of arena-allocated slices are tied to the arena's borrow
- [ ] Implement `Arena::used(&self) -> usize` and `Arena::high_water(&self) -> usize` for metrics
- [ ] Define typed wrapper `ArenaVec<T>` that borrows from the arena for the request lifetime
- [ ] Add `AllocStats { arena_high_water_bytes: usize, arena_resets: u64 }` to `alloc/mod.rs`
- [ ] Wire `AllocStats` into `calyx-core`'s metrics surface (counter increment on reset)
- [ ] `calyx-core/src/alloc/mod.rs` re-exports `Arena`, `ArenaVec`, `AllocStats`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: allocate exactly `cap` bytes in 4 calls of `cap/4` each → `used() == cap`; then `reset()` → `used() == 0`; cursor == 0 verified
- [ ] unit: allocate `cap + 1` bytes total → `CALYX_ALLOC_CAP_EXCEEDED` returned on the overflowing call; previously allocated bytes untouched
- [ ] proptest: `forall cap in 1..=1_048_576, sizes: Vec<usize>` — if `sum(sizes) <= cap` then all allocs succeed; if `sum > cap` then exactly the first call that would exceed returns `CALYX_ALLOC_CAP_EXCEEDED`
- [ ] unit: `reset()` is O(1) — measured with `std::time::Instant` on 1e6 resets; mean < 50 ns (no allocator calls)
- [ ] unit: alignment padding correct — alloc 1 byte (align 1), then alloc 8 bytes (align 8) → second pointer is 8-byte aligned; gap accounts for padding
- [ ] edge: `cap == 0` → `Arena::new` returns `CALYX_ALLOC_CAP_EXCEEDED`
- [ ] edge: `size == 0` alloc → returns dangling-but-aligned pointer, does not advance cursor (zero-size alloc is a no-op)
- [ ] edge: very large `align` (e.g., 4096, page-size) with a small arena → padding alone can exceed cap → `CALYX_ALLOC_CAP_EXCEEDED`
- [ ] fail-closed: arena at exactly `cap - 1` bytes used, alloc 2 bytes → `CALYX_ALLOC_CAP_EXCEEDED`; no partial advance of cursor

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `AllocStats::arena_high_water_bytes` metric emitted to Prometheus on aiwonder; and direct inspection of `Arena::used()` in the soak test binary
- **Readback:** `calyx readback --metric arena_high_water_bytes` or `cargo test -- --nocapture 2>&1 | grep arena_high_water`
- **Prove:** run the 1e7-op soak sub-task (T07) after T01–T06 complete; the `arena_high_water_bytes` series must plateau (no growth trend); `cursor` after each `reset()` reads as `0` in the metric — that is the byte-level proof that O(1) reset holds and no memory is leaked via the arena

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH56 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
