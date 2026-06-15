# PH57 · T02 — LRU eviction of GPU-resident blocks

| Field | Value |
|---|---|
| **Phase** | PH57 — VRAM budgeter + admission control |
| **Stage** | S13 — Resource, GC & Reliability Hardening |
| **Crate** | `calyx-forge` |
| **Files** | `crates/calyx-forge/src/vram/lru_evict.rs` (≤500) |
| **Depends on** | T01 (VramBudgeter + VramGuard) |
| **Axioms** | A26, A16 |
| **PRD** | `dbprdplans/24 §2` |

## Goal

Implement an LRU eviction registry for GPU-resident blocks (quantized embedding batches, ANN
frontier blocks, autotune scratch buffers). When a new allocation would exceed the soft VRAM
cap, evict the LRU block(s) — freeing their GPU memory back to CUDA and decrementing
`allocated_bytes` in the budgeter — until enough space is available. Eviction is synchronous
and deterministic (no background thread required here; admission control calls it). Streaming
from mmap: VRAM holds only the current batch + ANN frontier, never the corpus.

## Build (checklist of concrete, code-level steps)

- [ ] Define `struct GpuBlockRegistry { lru: IndexMap<BlockId, GpuBlock>, budgeter: Arc<VramBudgeter> }` in `lru_evict.rs`
- [ ] Define `struct GpuBlock { device_ptr: *mut u8, size_bytes: usize, guard: VramGuard<'static> }` — `guard` holds the budgeter reservation; dropping `GpuBlock` frees CUDA memory and releases the guard
- [ ] Implement `GpuBlockRegistry::insert(&mut self, id: BlockId, ptr: *mut u8, size: usize, guard: VramGuard<'static>)` — adds to LRU tail (most recently used)
- [ ] Implement `GpuBlockRegistry::touch(&mut self, id: &BlockId)` — promotes block to MRU end
- [ ] Implement `GpuBlockRegistry::evict_lru(&mut self) -> Option<usize>` — removes the LRU entry, calls `cudaFree` on `device_ptr`, drops `VramGuard`; returns freed bytes; returns `None` if empty
- [ ] Implement `GpuBlockRegistry::evict_until(&mut self, needed_bytes: usize) -> Result<(), CalyxError>` — calls `evict_lru` in a loop until `budgeter.allocated_bytes + needed_bytes <= soft_cap`; if the registry empties before enough space is free, returns `CALYX_FORGE_VRAM_BUDGET` (fail closed — even after eviction, not enough VRAM)
- [ ] Implement `GpuBlockRegistry::get(&mut self, id: &BlockId) -> Option<*const u8>` — returns pointer and calls `touch`
- [ ] ANN frontier cap: add `max_frontier_blocks: usize` config; if inserting would exceed it, evict oldest frontier block first (frontier-specific LRU budget within the overall budget)
- [ ] Add `GpuBlockStats { resident_blocks: usize, resident_bytes: usize, evictions_total: u64 }` to `VramStats`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: insert 3 blocks (A=1GiB, B=512MiB, C=256MiB) with a 2GiB soft cap; verify `resident_bytes == 1792MiB`; insert D=512MiB → `evict_until(512MiB)` evicts A (LRU); `resident_bytes == 1280MiB`; D inserted
- [ ] unit: `touch(B)` promotes B; next eviction evicts C (now LRU), not B
- [ ] unit: `evict_until` when registry has only 100MiB total and needs 200MiB → returns `CALYX_FORGE_VRAM_BUDGET`
- [ ] unit: `evict_lru` on empty registry → returns `None` (no panic)
- [ ] unit: ANN frontier cap — insert `max_frontier_blocks + 1` frontier blocks → oldest is evicted first (before general LRU)
- [ ] proptest: `forall soft_cap, ops: Vec<InsertOrGet>` — `resident_bytes` never exceeds `soft_cap` after any sequence
- [ ] edge: block with `size_bytes == 0` — insert OK; does not count against budget; eviction is a no-op on size
- [ ] fail-closed: mock `cudaFree` to return `cudaErrorInvalidValue` → `evict_lru` logs `CALYX_GPU_ERROR` but does NOT panic; budgeter `allocated_bytes` is decremented (the mapping is gone, even if CUDA is confused)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `GpuBlockStats::evictions_total` counter and `resident_bytes` from `VramBudgeter::stats()` during the concurrent TEI soak (T06)
- **Readback:** `calyx readback --metric forge_gpu_evictions_total` and `forge_vram_resident_bytes`
- **Prove:** under pressure (TEI + Forge concurrent load), `resident_bytes` stays ≤ `soft_cap_bytes`; `evictions_total > 0` (eviction is running, not stalled); no `cudaErrorMemoryAllocation` in `dmesg` or process logs.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] CPU↔GPU bit-parity ≤ 1e-3 on the golden set
- [ ] FSV evidence (readback output / screenshot) attached to the PH57 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
