//! LRU eviction registry for GPU-resident blocks (PH57 · T02).
//!
//! `calyx-forge` streams from mmap: VRAM holds only the working set — the
//! current quantized embedding batch, the ANN frontier blocks, and autotune
//! scratch — never the whole corpus. When a new allocation would push Forge
//! past its soft VRAM cap, the admission path evicts the least-recently-used
//! resident block(s) until enough budget is free, or fails closed with
//! [`CALYX_FORGE_VRAM_BUDGET`](crate::ForgeError::VramBudget) when even an
//! empty registry cannot make room (some other subsystem holds the budget).
//!
//! Design (mirrors the T01 budgeter's hardware-boundary split):
//! * The **accounting and ordering run on real bytes** — a real
//!   [`VramBudgeter`] reservation ([`VramGuard`]) backs every block, and
//!   dropping a block releases that reservation (RAII). There is no mocked
//!   logic; tests drive the genuine registry.
//! * The single hardware boundary is [`BlockDeallocator`] — the physical
//!   `cudaFree`. Production injects a CUDA-backed deallocator (wired from the
//!   GPU allocation path, which owns the device context); tests inject a
//!   deterministic recorder. A deallocation failure is logged loudly and
//!   surfaced, never silently swallowed — but eviction still reclaims the
//!   budget reservation, because the block is leaving the registry regardless.
//!
//! Eviction is synchronous and deterministic: no background thread. Admission
//! control (T03) calls [`GpuBlockRegistry::evict_until`] before reserving.

use std::collections::{HashMap, VecDeque};

use crate::Result;
use crate::vram::{VramBudgeter, VramGuard, VramProbe};

/// Opaque identifier for a GPU-resident block (embedding batch, ANN frontier
/// block, or autotune scratch buffer).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct BlockId(pub u64);

/// A GPU device address. CUDA's `CUdeviceptr` is a 64-bit integer handle, so we
/// store it as an integer rather than a raw `*mut u8`: the registry stays
/// `Send`/`Sync` and free of raw-pointer aliasing UB, and the hardware boundary
/// [`BlockDeallocator`] turns it back into the device pointer for `cudaFree`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DevicePtr(pub u64);

/// Block category. Frontier blocks share the overall VRAM budget but also have
/// their own count cap so a runaway ANN search cannot starve embedding batches.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BlockKind {
    /// General resident block (embedding batch, autotune scratch).
    General,
    /// ANN frontier block, subject to `max_frontier_blocks`.
    Frontier,
}

/// The hardware boundary that physically frees a GPU allocation.
///
/// Production calls `cudaFree`; tests inject a deterministic recorder. A
/// failure MUST be reported (returned `Err`), never silently swallowed — but
/// the caller ([`GpuBlockRegistry::evict_lru`]) still completes the eviction
/// and reclaims the budget, because the mapping is gone from Forge's registry
/// regardless of what the driver reports.
pub trait BlockDeallocator: Send + Sync {
    /// Physically free `size_bytes` at device address `ptr`. `Err` on any
    /// driver failure — never a silent success.
    fn free(&self, ptr: DevicePtr, size_bytes: usize) -> Result<()>;
}

/// A GPU-resident block tracked by the registry. Owns its [`VramGuard`], so
/// dropping the block releases the budget reservation.
struct GpuBlock<'b, P: VramProbe> {
    ptr: DevicePtr,
    size_bytes: usize,
    kind: BlockKind,
    // Held solely for its RAII `Drop`: dropping the block releases this
    // budget reservation (decrementing the budgeter). Never read directly —
    // the dead-code lint does not model drop-for-effect. Its effect is proven
    // in tests via `VramBudgeter::allocated_bytes()` falling after eviction.
    #[allow(dead_code)]
    guard: VramGuard<'b, P>,
}

/// Point-in-time eviction-registry accounting — the in-crate Source of Truth
/// surfaced as `forge_vram_resident_bytes` / `forge_gpu_evictions_total`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GpuBlockStats {
    /// Number of blocks currently resident.
    pub resident_blocks: usize,
    /// Sum of resident block sizes in bytes.
    pub resident_bytes: usize,
    /// Cumulative count of blocks evicted over the registry's lifetime.
    pub evictions_total: u64,
}

/// LRU eviction registry for GPU-resident blocks, backed by a real
/// [`VramBudgeter`] and a [`BlockDeallocator`] hardware boundary.
///
/// The registry borrows the budgeter (`&'b`) so a block's [`VramGuard`] — which
/// also borrows the budgeter — can live exactly as long as the block. LRU order
/// is the `lru` deque: front is least-recently-used, back is most-recent.
pub struct GpuBlockRegistry<'b, P: VramProbe, D: BlockDeallocator> {
    blocks: HashMap<BlockId, GpuBlock<'b, P>>,
    /// LRU order: front = LRU (next to evict), back = MRU.
    lru: VecDeque<BlockId>,
    budgeter: &'b VramBudgeter<P>,
    dealloc: D,
    max_frontier_blocks: usize,
    evictions_total: u64,
}

impl<'b, P: VramProbe, D: BlockDeallocator> GpuBlockRegistry<'b, P, D> {
    /// Construct over a shared budgeter and a deallocator, with a cap on the
    /// number of concurrently resident frontier blocks.
    pub fn new(budgeter: &'b VramBudgeter<P>, dealloc: D, max_frontier_blocks: usize) -> Self {
        Self {
            blocks: HashMap::new(),
            lru: VecDeque::new(),
            budgeter,
            dealloc,
            max_frontier_blocks,
            evictions_total: 0,
        }
    }

    /// Register a newly allocated GPU block as most-recently-used. `guard` is
    /// the live budget reservation for `size`; the registry owns it until the
    /// block is evicted or the registry is dropped.
    ///
    /// If `kind` is [`BlockKind::Frontier`] and the registry is already at
    /// `max_frontier_blocks`, the oldest frontier block is evicted first
    /// (frontier-specific LRU within the overall budget). Re-inserting an
    /// existing `id` evicts the prior block at that id first (no silent leak of
    /// its guard or device mapping).
    pub fn insert(
        &mut self,
        id: BlockId,
        ptr: DevicePtr,
        size: usize,
        kind: BlockKind,
        guard: VramGuard<'b, P>,
    ) {
        if self.blocks.contains_key(&id) {
            self.evict_id(&id);
        }
        if kind == BlockKind::Frontier && self.frontier_count() >= self.max_frontier_blocks {
            self.evict_oldest_frontier();
        }
        self.blocks.insert(
            id,
            GpuBlock {
                ptr,
                size_bytes: size,
                kind,
                guard,
            },
        );
        self.lru.push_back(id);
    }

    /// Promote `id` to most-recently-used. No-op if absent.
    pub fn touch(&mut self, id: &BlockId) {
        if self.blocks.contains_key(id) {
            self.move_to_mru(id);
        }
    }

    /// Return the device pointer for `id` and promote it to MRU. `None` if
    /// absent.
    pub fn get(&mut self, id: &BlockId) -> Option<DevicePtr> {
        let ptr = self.blocks.get(id).map(|block| block.ptr)?;
        self.move_to_mru(id);
        Some(ptr)
    }

    /// Evict the least-recently-used block: free its GPU memory via the
    /// deallocator, drop its [`VramGuard`] (releasing the budget), and return
    /// the freed byte count. `None` if the registry is empty.
    ///
    /// A deallocator failure is logged at error level with the exact
    /// `CALYX_*` code but does NOT abort the eviction — the budget reservation
    /// is reclaimed regardless, since the block is gone from Forge's registry.
    pub fn evict_lru(&mut self) -> Option<usize> {
        let id = self.lru.front().copied()?;
        Some(self.remove_block(&id))
    }

    /// Evict LRU blocks until Forge's reserved total plus `needed_bytes` fits
    /// under the soft cap. Fails closed with [`CALYX_FORGE_VRAM_BUDGET`] if the
    /// registry empties before enough budget is free (another subsystem holds
    /// the reservation that this registry cannot evict).
    ///
    /// [`crate::ForgeError::VramBudget`]: crate::ForgeError::VramBudget
    pub fn evict_until(&mut self, needed_bytes: usize) -> Result<()> {
        let soft_cap = self.budgeter.soft_cap_bytes();
        while self.budgeter.allocated_bytes().saturating_add(needed_bytes) > soft_cap {
            if self.evict_lru().is_none() {
                // Registry empty but still over budget: fail closed, do not
                // pretend space exists.
                return Err(crate::ForgeError::VramBudget {
                    detail: format!(
                        "eviction exhausted the GPU block registry but {needed} bytes still do not fit: allocated={alloc}, soft_cap={soft_cap}",
                        needed = needed_bytes,
                        alloc = self.budgeter.allocated_bytes(),
                    ),
                    remediation: crate::vram::VRAM_BUDGET_REMEDIATION.to_string(),
                });
            }
        }
        Ok(())
    }

    /// Snapshot the registry accounting (the FSV Source of Truth).
    pub fn stats(&self) -> GpuBlockStats {
        GpuBlockStats {
            resident_blocks: self.blocks.len(),
            resident_bytes: self.resident_bytes(),
            evictions_total: self.evictions_total,
        }
    }

    /// Sum of resident block sizes.
    pub fn resident_bytes(&self) -> usize {
        self.blocks.values().map(|block| block.size_bytes).sum()
    }

    /// Number of resident frontier blocks.
    pub fn frontier_count(&self) -> usize {
        self.blocks
            .values()
            .filter(|block| block.kind == BlockKind::Frontier)
            .count()
    }

    /// Move `id` to the MRU end of the LRU deque. Caller guarantees presence.
    fn move_to_mru(&mut self, id: &BlockId) {
        if let Some(pos) = self.lru.iter().position(|entry| entry == id) {
            self.lru.remove(pos);
        }
        self.lru.push_back(*id);
    }

    /// Evict the oldest (closest-to-LRU) frontier block. `None` if none exist.
    fn evict_oldest_frontier(&mut self) -> Option<usize> {
        let id = self.lru.iter().copied().find(|id| {
            self.blocks
                .get(id)
                .is_some_and(|block| block.kind == BlockKind::Frontier)
        })?;
        Some(self.remove_block(&id))
    }

    /// Evict a specific id (used when re-inserting over an existing id).
    fn evict_id(&mut self, id: &BlockId) {
        if self.blocks.contains_key(id) {
            self.remove_block(id);
        }
    }

    /// Remove `id`: free GPU memory, drop the guard, drop LRU bookkeeping,
    /// bump the eviction counter. Returns freed bytes. Caller guarantees `id`
    /// is present in `lru`/`blocks` consistently.
    fn remove_block(&mut self, id: &BlockId) -> usize {
        if let Some(pos) = self.lru.iter().position(|entry| entry == id) {
            self.lru.remove(pos);
        }
        let block = self
            .blocks
            .remove(id)
            .expect("remove_block called for a tracked id");
        let size = block.size_bytes;
        if let Err(err) = self.dealloc.free(block.ptr, size) {
            tracing::error!(
                target: "calyx_forge::vram",
                code = err.code(),
                error = %err,
                block_id = id.0,
                device_ptr = block.ptr.0,
                size_bytes = size,
                "cudaFree failed during eviction; budget reservation reclaimed regardless (mapping gone from registry)"
            );
        }
        // Dropping `block` here releases its VramGuard, decrementing the
        // budgeter's allocated_bytes by `size`.
        drop(block);
        self.evictions_total += 1;
        size
    }
}

#[cfg(test)]
#[path = "lru_evict_tests.rs"]
mod tests;
