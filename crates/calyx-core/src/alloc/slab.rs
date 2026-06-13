//! Slab / fixed-size object pool (PH56 · T02).
//!
//! A [`SlabPool`] pre-allocates `cap_slots` fixed-size slots once and hands them
//! out via a free list. Hot, frequently-recycled objects — vector embedding
//! blocks, ANN graph nodes, GPU staging buffers — are *acquired* and *returned*
//! (RAII), never `malloc`/`free`'d per op, so there is no per-block
//! fragmentation and a hard slot-count cap (A26). Exhaustion fails closed with
//! [`CALYX_ALLOC_CAP_EXCEEDED`] — never a silent grow, OOM, or zeroed buffer.
//!
//! [`PageAlignedSlabPool`] is the 4 KiB-aligned variant for pinned-host CUDA
//! staging transfers.
//!
//! ## Soundness
//!
//! Slots live in `UnsafeCell`s; the free list guarantees at most one live
//! [`SlabGuard`] per slot index, so the `&mut [u8; N]` each guard hands out is
//! unique (no aliasing). The free list / slot-state vectors live behind a
//! `RefCell`, so `acquire`/`release` need only `&self` — letting every
//! outstanding guard stay alive at once (which capacity tests require).

use std::alloc::{self, Layout};
use std::cell::{RefCell, UnsafeCell};
use std::ops::{Deref, DerefMut};
use std::ptr::NonNull;

use super::alloc_cap_exceeded;
use crate::Result;

/// Page size for the page-aligned (GPU staging) variant.
pub const PAGE_SIZE: usize = 4096;

/// Embedding dimension assumed by [`VecBlockPool`]. Vector blocks elsewhere are
/// runtime-dimensioned; this fixed value sizes the convenience pool alias.
pub const DEFAULT_EMBED_DIM: usize = 768;

/// Byte size of one f32 vector block at [`DEFAULT_EMBED_DIM`].
pub const VEC_BLOCK_SIZE: usize = DEFAULT_EMBED_DIM * 4;

/// A pool of f32 embedding-vector blocks.
pub type VecBlockPool = SlabPool<VEC_BLOCK_SIZE>;

/// A fixed-size ANN graph node (id + up-to-32 neighbor ids + level).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnnNode {
    /// Node identity.
    pub id: u64,
    /// Neighbor node ids (fixed fan-out).
    pub neighbors: [u32; 32],
    /// HNSW level.
    pub level: u16,
    /// Padding to a clean size.
    pub pad: [u8; 6],
}

/// Byte size of one [`AnnNode`].
pub const ANN_NODE_SIZE: usize = std::mem::size_of::<AnnNode>();

/// A pool of fixed-size ANN graph nodes.
pub type AnnNodePool = SlabPool<ANN_NODE_SIZE>;

/// Per-slot state; the enum makes a double-release a detectable bug.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SlotStatus {
    Free,
    Held,
}

/// Free list + per-slot state, mutated through `&self` via a `RefCell`.
#[derive(Debug)]
struct Meta {
    free_list: Vec<usize>,
    status: Vec<SlotStatus>,
}

impl Meta {
    fn new(cap_slots: usize) -> Self {
        // `rev` so the first `pop()` yields slot 0, then 1, ... (readable order).
        Self {
            free_list: (0..cap_slots).rev().collect(),
            status: vec![SlotStatus::Free; cap_slots],
        }
    }

    fn acquire_index(&mut self) -> Option<usize> {
        let idx = self.free_list.pop()?;
        self.status[idx] = SlotStatus::Held;
        Some(idx)
    }

    fn release_index(&mut self, index: usize) {
        // A release of a slot that is not currently Held is a use-after-free /
        // double-release bug. Fail loudly in debug; in release, refuse to push a
        // duplicate (which would later hand the same slot to two callers).
        if self.status[index] != SlotStatus::Held {
            debug_assert!(
                false,
                "slab release of slot {index} that is not Held (double-release)"
            );
            return;
        }
        self.status[index] = SlotStatus::Free;
        self.free_list.push(index);
    }

    fn held(&self, cap_slots: usize) -> usize {
        cap_slots - self.free_list.len()
    }
}

/// A fixed-size object pool of `SLOT_SIZE`-byte slots.
#[derive(Debug)]
pub struct SlabPool<const SLOT_SIZE: usize> {
    slots: Box<[UnsafeCell<[u8; SLOT_SIZE]>]>,
    meta: RefCell<Meta>,
    cap_slots: usize,
}

impl<const SLOT_SIZE: usize> SlabPool<SLOT_SIZE> {
    /// Builds a pool of `cap_slots` zeroed slots.
    ///
    /// # Errors
    /// [`CALYX_ALLOC_CAP_EXCEEDED`](super::CALYX_ALLOC_CAP_EXCEEDED) if
    /// `cap_slots == 0` (a pool that can never hand out a slot).
    pub fn new(cap_slots: usize) -> Result<Self> {
        if cap_slots == 0 {
            return Err(alloc_cap_exceeded("slab pool cap_slots must be > 0"));
        }
        let slots = (0..cap_slots)
            .map(|_| UnsafeCell::new([0u8; SLOT_SIZE]))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Ok(Self {
            slots,
            meta: RefCell::new(Meta::new(cap_slots)),
            cap_slots,
        })
    }

    /// Acquires a free slot, returning an RAII guard that releases it on drop.
    ///
    /// # Errors
    /// [`CALYX_ALLOC_CAP_EXCEEDED`](super::CALYX_ALLOC_CAP_EXCEEDED) when the
    /// pool is exhausted — fail closed, never a silent grow.
    pub fn acquire(&self) -> Result<SlabGuard<'_, SLOT_SIZE>> {
        let index = self.meta.borrow_mut().acquire_index().ok_or_else(|| {
            alloc_cap_exceeded(format!("slab pool exhausted ({} slots)", self.cap_slots))
        })?;
        Ok(SlabGuard { pool: self, index })
    }

    /// Hard cap: total slots.
    pub fn cap_slots(&self) -> usize {
        self.cap_slots
    }

    /// Slots currently held (acquired and not yet released).
    pub fn held(&self) -> usize {
        self.meta.borrow().held(self.cap_slots)
    }

    /// Fraction of slots currently held, in `[0.0, 1.0]`.
    pub fn utilization(&self) -> f64 {
        self.held() as f64 / self.cap_slots as f64
    }

    fn release(&self, index: usize) {
        self.meta.borrow_mut().release_index(index);
    }
}

/// RAII handle to one acquired slot; derefs to its `[u8; SLOT_SIZE]` and returns
/// the slot to the pool on drop.
#[derive(Debug)]
pub struct SlabGuard<'pool, const SLOT_SIZE: usize> {
    pool: &'pool SlabPool<SLOT_SIZE>,
    index: usize,
}

impl<const SLOT_SIZE: usize> SlabGuard<'_, SLOT_SIZE> {
    /// The slot index within the pool (stable for this guard's lifetime).
    pub fn slot_index(&self) -> usize {
        self.index
    }
}

impl<const SLOT_SIZE: usize> Deref for SlabGuard<'_, SLOT_SIZE> {
    type Target = [u8; SLOT_SIZE];

    fn deref(&self) -> &Self::Target {
        // SAFETY: the free list guarantees this index has exactly one live
        // guard, so this shared borrow does not alias any other access.
        unsafe { &*self.pool.slots[self.index].get() }
    }
}

impl<const SLOT_SIZE: usize> DerefMut for SlabGuard<'_, SLOT_SIZE> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: unique live guard for this index (see `Deref`), and `&mut self`
        // makes this the only borrow of the guard — so the `&mut` is unique.
        unsafe { &mut *self.pool.slots[self.index].get() }
    }
}

impl<const SLOT_SIZE: usize> Drop for SlabGuard<'_, SLOT_SIZE> {
    fn drop(&mut self) {
        self.pool.release(self.index);
    }
}

/// A page-aligned (`4 KiB`) fixed-size slab pool for pinned-host CUDA staging.
/// Every acquired slot pointer is a multiple of [`PAGE_SIZE`].
#[derive(Debug)]
pub struct PageAlignedSlabPool {
    base: NonNull<u8>,
    layout: Layout,
    slot_size: usize,
    cap_slots: usize,
    meta: RefCell<Meta>,
}

impl PageAlignedSlabPool {
    /// Builds a pool of `cap_slots` slots of `slot_size` bytes each.
    ///
    /// # Panics
    /// If `slot_size` is not a non-zero multiple of [`PAGE_SIZE`] — a build-time
    /// configuration invariant for pinned transfers.
    ///
    /// # Errors
    /// [`CALYX_ALLOC_CAP_EXCEEDED`](super::CALYX_ALLOC_CAP_EXCEEDED) if
    /// `cap_slots == 0`.
    pub fn new(slot_size: usize, cap_slots: usize) -> Result<Self> {
        assert!(
            slot_size != 0 && slot_size.is_multiple_of(PAGE_SIZE),
            "page-aligned slot_size must be a non-zero multiple of {PAGE_SIZE}, got {slot_size}"
        );
        if cap_slots == 0 {
            return Err(alloc_cap_exceeded(
                "page-aligned pool cap_slots must be > 0",
            ));
        }
        let total = slot_size
            .checked_mul(cap_slots)
            .ok_or_else(|| alloc_cap_exceeded("page-aligned pool size overflow"))?;
        let layout = Layout::from_size_align(total, PAGE_SIZE)
            .map_err(|e| alloc_cap_exceeded(format!("invalid page-aligned layout: {e}")))?;
        // SAFETY: `total > 0` (slot_size and cap_slots are both > 0).
        let raw = unsafe { alloc::alloc_zeroed(layout) };
        let base = NonNull::new(raw).unwrap_or_else(|| alloc::handle_alloc_error(layout));
        Ok(Self {
            base,
            layout,
            slot_size,
            cap_slots,
            meta: RefCell::new(Meta::new(cap_slots)),
        })
    }

    /// Acquires a page-aligned slot.
    ///
    /// # Errors
    /// [`CALYX_ALLOC_CAP_EXCEEDED`](super::CALYX_ALLOC_CAP_EXCEEDED) when exhausted.
    pub fn acquire(&self) -> Result<PageSlabGuard<'_>> {
        let index = self.meta.borrow_mut().acquire_index().ok_or_else(|| {
            alloc_cap_exceeded(format!(
                "page-aligned pool exhausted ({} slots)",
                self.cap_slots
            ))
        })?;
        Ok(PageSlabGuard { pool: self, index })
    }

    /// Slot size in bytes (a multiple of [`PAGE_SIZE`]).
    pub fn slot_size(&self) -> usize {
        self.slot_size
    }

    /// Hard cap: total slots.
    pub fn cap_slots(&self) -> usize {
        self.cap_slots
    }

    /// Slots currently held.
    pub fn held(&self) -> usize {
        self.meta.borrow().held(self.cap_slots)
    }

    /// Fraction of slots currently held, in `[0.0, 1.0]`.
    pub fn utilization(&self) -> f64 {
        self.held() as f64 / self.cap_slots as f64
    }

    fn release(&self, index: usize) {
        self.meta.borrow_mut().release_index(index);
    }

    fn slot_ptr(&self, index: usize) -> *mut u8 {
        // SAFETY: index < cap_slots, so the offset stays within the allocation.
        unsafe { self.base.as_ptr().add(index * self.slot_size) }
    }
}

impl Drop for PageAlignedSlabPool {
    fn drop(&mut self) {
        // SAFETY: `base`/`layout` are exactly what `alloc_zeroed` returned,
        // freed exactly once here.
        unsafe { alloc::dealloc(self.base.as_ptr(), self.layout) }
    }
}

// SAFETY: the pool owns its allocation exclusively; there is no shared interior
// access without `&mut`/guard discipline. (Not `Sync`: `RefCell` is `!Sync`.)
unsafe impl Send for PageAlignedSlabPool {}

/// RAII handle to one page-aligned staging slot.
#[derive(Debug)]
pub struct PageSlabGuard<'pool> {
    pool: &'pool PageAlignedSlabPool,
    index: usize,
}

impl PageSlabGuard<'_> {
    /// The slot index within the pool.
    pub fn slot_index(&self) -> usize {
        self.index
    }

    /// A `PAGE_SIZE`-aligned mutable pointer to the slot's bytes.
    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        self.pool.slot_ptr(self.index)
    }

    /// The slot's bytes as a mutable slice.
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        // SAFETY: unique live guard for this index, pointer + len stay within
        // the one slot, which is initialized (zeroed at construction).
        unsafe {
            std::slice::from_raw_parts_mut(self.pool.slot_ptr(self.index), self.pool.slot_size)
        }
    }
}

impl Drop for PageSlabGuard<'_> {
    fn drop(&mut self) {
        self.pool.release(self.index);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alloc::CALYX_ALLOC_CAP_EXCEEDED;

    #[test]
    fn acquire_to_cap_then_exhausted_then_reuse_same_slot() {
        let pool = SlabPool::<64>::new(3).expect("pool");
        let g0 = pool.acquire().expect("0");
        let g1 = pool.acquire().expect("1");
        let g2 = pool.acquire().expect("2");
        println!("held at cap = {} / {}", pool.held(), pool.cap_slots());
        assert_eq!(pool.held(), 3);
        let err = pool.acquire().expect_err("one past cap");
        assert_eq!(err.code, CALYX_ALLOC_CAP_EXCEEDED);
        let freed = g1.slot_index();
        drop(g1);
        let g1b = pool.acquire().expect("reuse");
        assert_eq!(g1b.slot_index(), freed, "released slot is the one reused");
        drop((g0, g2, g1b));
        assert_eq!(pool.held(), 0);
    }

    #[test]
    fn guard_drop_releases_and_slot_is_overwritable() {
        let pool = SlabPool::<8>::new(1).expect("pool");
        {
            let mut g = pool.acquire().expect("acquire");
            g.copy_from_slice(&[0xAB; 8]);
            assert_eq!(*g, [0xAB; 8]);
        } // drop releases
        let mut g2 = pool.acquire().expect("reacquire same slot");
        // Overwritable, no double-free panic.
        g2.copy_from_slice(&[0xCD; 8]);
        assert_eq!(*g2, [0xCD; 8]);
    }

    #[test]
    fn utilization_tracks_holders() {
        let pool = SlabPool::<16>::new(4).expect("pool");
        assert_eq!(pool.utilization(), 0.0);
        let g = pool.acquire().unwrap();
        let g2 = pool.acquire().unwrap();
        let g3 = pool.acquire().unwrap();
        let g4 = pool.acquire().unwrap();
        assert_eq!(pool.utilization(), 1.0);
        drop((g, g2, g3, g4));
        assert_eq!(pool.utilization(), 0.0);
    }

    #[test]
    fn zero_cap_rejected() {
        let err = SlabPool::<8>::new(0).expect_err("zero cap");
        assert_eq!(err.code, CALYX_ALLOC_CAP_EXCEEDED);
    }

    #[test]
    #[should_panic(expected = "double-release")]
    fn release_of_unacquired_slot_panics_in_debug() {
        let pool = SlabPool::<8>::new(2).expect("pool");
        // Slot 0 was never acquired -> releasing it is a bug.
        pool.release(0);
    }

    #[test]
    fn page_aligned_slots_are_page_aligned() {
        let pool = PageAlignedSlabPool::new(PAGE_SIZE, 4).expect("pool");
        let mut guards = Vec::new();
        for _ in 0..4 {
            let mut g = pool.acquire().expect("acquire");
            let p = g.as_mut_ptr();
            println!(
                "slot {} ptr {p:p} % {PAGE_SIZE} = {}",
                g.slot_index(),
                p as usize % PAGE_SIZE
            );
            assert_eq!(p as usize % PAGE_SIZE, 0, "slot pointer is page-aligned");
            guards.push(g);
        }
        assert_eq!(pool.utilization(), 1.0);
        let err = pool.acquire().expect_err("exhausted");
        assert_eq!(err.code, CALYX_ALLOC_CAP_EXCEEDED);
    }

    #[test]
    fn page_aligned_slot_is_writable_and_isolated() {
        let pool = PageAlignedSlabPool::new(PAGE_SIZE * 2, 2).expect("pool");
        let mut a = pool.acquire().unwrap();
        let mut b = pool.acquire().unwrap();
        a.as_mut_slice().fill(0x11);
        b.as_mut_slice().fill(0x22);
        // Distinct slots: writing one does not bleed into the other.
        assert!(a.as_mut_slice().iter().all(|&x| x == 0x11));
        assert!(b.as_mut_slice().iter().all(|&x| x == 0x22));
    }

    #[test]
    #[should_panic(expected = "multiple")]
    fn page_aligned_rejects_non_page_slot_size() {
        let _ = PageAlignedSlabPool::new(100, 1);
    }

    #[test]
    fn vec_block_and_ann_node_pools_construct() {
        let vblocks = VecBlockPool::new(2).expect("vec block pool");
        assert_eq!(vblocks.cap_slots(), 2);
        let _g = vblocks.acquire().unwrap();
        assert_eq!(
            std::mem::size_of::<[u8; VEC_BLOCK_SIZE]>(),
            DEFAULT_EMBED_DIM * 4
        );

        let nodes = AnnNodePool::new(2).expect("ann node pool");
        assert_eq!(nodes.cap_slots(), 2);
        assert_eq!(ANN_NODE_SIZE, std::mem::size_of::<AnnNode>());
    }

    proptest::proptest! {
        #[test]
        fn never_exceeds_cap_and_accounting_balances(
            cap in 1usize..=256,
            ops in proptest::collection::vec(proptest::bool::ANY, 0..1024),
        ) {
            let pool = SlabPool::<8>::new(cap).expect("pool");
            let mut held: Vec<SlabGuard<8>> = Vec::new();
            for acquire in ops {
                if acquire {
                    if let Ok(g) = pool.acquire() {
                        held.push(g);
                    } else {
                        // Exhaustion only when truly at cap.
                        proptest::prop_assert_eq!(held.len(), cap);
                    }
                } else {
                    held.pop(); // drop releases
                }
                // Invariant: held + free == cap, and held never exceeds cap.
                proptest::prop_assert!(held.len() <= cap);
                proptest::prop_assert_eq!(pool.held(), held.len());
            }
        }
    }
}
