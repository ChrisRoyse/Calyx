# PH56 · T02 — Slab/pool allocator — vector blocks, ANN nodes, GPU staging

| Field | Value |
|---|---|
| **Phase** | PH56 — Bounded caches/queues/memtables + arenas/pools |
| **Stage** | S13 — Resource, GC & Reliability Hardening |
| **Crate** | `calyx-core` |
| **Files** | `crates/calyx-core/src/alloc/slab.rs` (≤500) |
| **Depends on** | T01 (arena/alloc module exists) |
| **Axioms** | A26, A16 |
| **PRD** | `dbprdplans/24 §1`, `24 §5` |

## Goal

Provide a slab/pool allocator for fixed-size hot objects — vector embedding blocks, ANN graph
nodes, and GPU staging buffers — that reuses memory without per-op `malloc`/`free`, eliminates
SoA per-block fragmentation, and enforces a hard slab count cap (A26). Objects are returned to
the pool, not freed; the pool owns the slab memory deterministically (RAII). GPU staging slabs
are page-aligned (4 KiB) for pinned-host CUDA transfers.

## Build (checklist of concrete, code-level steps)

- [ ] Define `struct SlabPool<const SLOT_SIZE: usize> { slots: Vec<[u8; SLOT_SIZE]>, free_list: Vec<usize>, cap_slots: usize }` in `alloc/slab.rs`
- [ ] Implement `SlabPool::new(cap_slots: usize) -> Self` — pre-allocates `cap_slots` fixed-size slots; returns `CALYX_ALLOC_CAP_EXCEEDED` if `cap_slots == 0`
- [ ] Implement `SlabPool::acquire(&mut self) -> Result<SlabHandle, CalyxError>` — pops from free_list; returns `CALYX_ALLOC_CAP_EXCEEDED` when pool exhausted (fail closed, no silent growth)
- [ ] Implement `SlabPool::release(&mut self, handle: SlabHandle)` — pushes slot index back onto free_list; RAII guard `SlabGuard` auto-releases on drop
- [ ] Define `SlabGuard<'pool, const SLOT_SIZE: usize>` wrapping a mutable byte slice; `Drop` calls `release`
- [ ] Implement page-aligned variant `PageAlignedSlabPool` (slot size must be multiple of 4096) for GPU staging — uses `std::alloc::alloc` with `Layout::from_size_align(size, 4096).unwrap()` at construction
- [ ] Add pool utilization metric: `SlabPool::utilization() -> f64` = `(cap_slots - free_list.len()) / cap_slots`
- [ ] Expose `VecBlockPool` type alias for `SlabPool<{ EMBED_DIM * 4 }>` (f32 vector blocks; `EMBED_DIM` from `calyx-core` const)
- [ ] Expose `AnnNodePool` type alias for fixed-size ANN node structs

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: acquire all `cap_slots` handles → succeeds; acquire one more → `CALYX_ALLOC_CAP_EXCEEDED`; release one → acquire again → succeeds; verify slot index is the same released slot
- [ ] unit: `SlabGuard` drop releases slot — acquire, write known bytes, drop guard; acquire same slot again, bytes are overwritable (no double-free panic)
- [ ] proptest: `forall cap in 1..=256, ops: Vec<AcquireOrRelease>` — pool never exceeds `cap` simultaneous holders; free_list.len() + held == cap at all times
- [ ] unit: page-aligned variant — slot pointer % 4096 == 0 for every acquired slot (verify with `ptr as usize % 4096 == 0`)
- [ ] unit: `utilization()` is 0.0 at init, 1.0 when all acquired, back to 0.0 after all released
- [ ] edge: `cap_slots == 0` → `CALYX_ALLOC_CAP_EXCEEDED` from `new`
- [ ] edge: release a slot that was never acquired → must panic in debug builds (double-release guard via slot-state enum)
- [ ] fail-closed: pool at capacity with `cap_slots=1`, holding one slot; `acquire` returns `CALYX_ALLOC_CAP_EXCEEDED`, not a silent null/zeroed buffer

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `SlabPool::utilization()` metric at peak load during the soak (T07); and `VecBlockPool` / `AnnNodePool` utilization in Prometheus
- **Readback:** `calyx readback --metric slab_utilization` during the 1e7-op soak
- **Prove:** utilization plateau below 1.0 (never exhausted under normal load); when exhaustion is injected, `CALYX_ALLOC_CAP_EXCEEDED` appears in the rejection log — not an OOM or panic. Metric series must show no monotonic growth (no leak of unreturned slots).

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH56 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
