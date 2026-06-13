//! Arena / bump allocator (PH56 · T01).
//!
//! An [`Arena`] pre-allocates one aligned backing block of `cap` bytes and
//! satisfies every allocation by bumping a cursor — no per-op `malloc`/`free`,
//! no fragmentation. At request/microbatch end [`Arena::reset`] rewinds the
//! cursor to zero in O(1) (no `free` calls); the next request reuses the same
//! memory. Allocations fail closed with [`CALYX_ALLOC_CAP_EXCEEDED`] the instant
//! they would cross the cap — the arena never reallocs or grows (A26).
//!
//! The backing block is over-aligned to [`ARENA_BASE_ALIGN`], so the returned
//! pointers' alignment is a deterministic function of the cursor (not of where
//! the OS happened to place a `Vec`), which keeps the alignment tests exact.

use std::alloc::{self, Layout};
use std::marker::PhantomData;
use std::ptr::NonNull;

use super::{AllocStats, alloc_cap_exceeded};
use crate::Result;

/// Backing-block base alignment. Pointers requested with an alignment `<=` this
/// are aligned deterministically relative to the cursor (the block base is a
/// multiple of every power of two up to this value).
pub const ARENA_BASE_ALIGN: usize = 4096;

/// An owned, [`ARENA_BASE_ALIGN`]-aligned byte block. Frees on drop.
#[derive(Debug)]
struct AlignedBlock {
    ptr: NonNull<u8>,
    layout: Layout,
}

impl AlignedBlock {
    /// Allocates `cap` zeroed, [`ARENA_BASE_ALIGN`]-aligned bytes. `cap > 0`.
    fn new(cap: usize) -> Self {
        debug_assert!(cap > 0, "AlignedBlock::new requires cap > 0");
        let layout = Layout::from_size_align(cap, ARENA_BASE_ALIGN)
            .expect("arena cap + base align is a valid layout");
        // SAFETY: `layout` has non-zero size (cap > 0). `alloc_zeroed` returns a
        // block of `cap` bytes aligned to ARENA_BASE_ALIGN, or null on OOM.
        let raw = unsafe { alloc::alloc_zeroed(layout) };
        let ptr = NonNull::new(raw).unwrap_or_else(|| alloc::handle_alloc_error(layout));
        Self { ptr, layout }
    }
}

impl Drop for AlignedBlock {
    fn drop(&mut self) {
        // SAFETY: `ptr`/`layout` are exactly what `alloc_zeroed` returned and
        // are freed exactly once (here, on drop).
        unsafe { alloc::dealloc(self.ptr.as_ptr(), self.layout) }
    }
}

/// A bump allocator over a fixed, pre-allocated, aligned block.
///
/// Not `Sync`/`Send`-shared: an arena is owned by one request/microbatch at a
/// time. Hand allocations out via [`Arena::alloc`] (raw) or [`Arena::alloc_vec`]
/// (typed); rewind everything with [`Arena::reset`].
#[derive(Debug)]
pub struct Arena {
    block: AlignedBlock,
    cursor: usize,
    cap: usize,
    high_water: usize,
    resets: u64,
}

impl Arena {
    /// Builds an arena with a hard cap of `cap` bytes.
    ///
    /// # Errors
    /// [`CALYX_ALLOC_CAP_EXCEEDED`](super::CALYX_ALLOC_CAP_EXCEEDED) if `cap == 0`
    /// — a zero-capacity arena can never satisfy an allocation, so it is rejected
    /// at construction rather than failing every later call.
    pub fn new(cap: usize) -> Result<Self> {
        if cap == 0 {
            return Err(alloc_cap_exceeded("arena cap must be > 0"));
        }
        Ok(Self {
            block: AlignedBlock::new(cap),
            cursor: 0,
            cap,
            high_water: 0,
            resets: 0,
        })
    }

    /// Allocates `size` bytes aligned to `align` (a power of two) by bumping the
    /// cursor past any alignment padding. Returns a pointer into the arena's
    /// block valid until the next [`reset`](Arena::reset) (or arena drop).
    ///
    /// A zero-size request is a no-op: it returns a non-null, correctly aligned
    /// dangling pointer and does **not** advance the cursor.
    ///
    /// # Errors
    /// [`CALYX_ALLOC_CAP_EXCEEDED`](super::CALYX_ALLOC_CAP_EXCEEDED) when the
    /// padded request would cross the cap. The cursor is **not** advanced on
    /// failure (no partial allocation), so the arena stays usable.
    pub fn alloc(&mut self, size: usize, align: usize) -> Result<*mut u8> {
        assert!(align.is_power_of_two(), "alignment must be a power of two");
        if size == 0 {
            // A dangling-but-aligned pointer; consumes nothing.
            return Ok(align as *mut u8);
        }
        // Padding to align the cursor. Because the block base is a multiple of
        // ARENA_BASE_ALIGN, for `align <= ARENA_BASE_ALIGN` this padding is a
        // pure function of the cursor; for larger `align` we still align the
        // real address. Compute against the absolute address to stay correct.
        let base = self.block.ptr.as_ptr() as usize;
        let current = base
            .checked_add(self.cursor)
            .ok_or_else(|| alloc_cap_exceeded("arena cursor address overflow"))?;
        let aligned = current
            .checked_next_multiple_of(align)
            .ok_or_else(|| alloc_cap_exceeded("arena alignment overflow"))?;
        let padding = aligned - current;
        let consumed = padding
            .checked_add(size)
            .ok_or_else(|| alloc_cap_exceeded("arena request size overflow"))?;
        let new_cursor = self
            .cursor
            .checked_add(consumed)
            .ok_or_else(|| alloc_cap_exceeded("arena cursor overflow"))?;
        if new_cursor > self.cap {
            return Err(alloc_cap_exceeded(format!(
                "arena alloc of {size} bytes (align {align}, +{padding} pad) would use \
                 {new_cursor} > cap {} bytes",
                self.cap
            )));
        }
        self.cursor = new_cursor;
        if self.cursor > self.high_water {
            self.high_water = self.cursor;
        }
        Ok(aligned as *mut u8)
    }

    /// Allocates room for up to `capacity` values of `T`, returning an empty
    /// [`ArenaVec`] you push into. The vector borrows the arena for its whole
    /// lifetime (so the arena cannot be reset or re-allocated from while it is
    /// alive — drop it first).
    ///
    /// # Errors
    /// [`CALYX_ALLOC_CAP_EXCEEDED`](super::CALYX_ALLOC_CAP_EXCEEDED) if the
    /// backing bytes do not fit under the cap.
    pub fn alloc_vec<T>(&mut self, capacity: usize) -> Result<ArenaVec<'_, T>> {
        let elem = std::mem::size_of::<T>();
        let align = std::mem::align_of::<T>();
        if elem == 0 || capacity == 0 {
            // ZST or zero-capacity: no backing bytes needed.
            return Ok(ArenaVec {
                ptr: NonNull::dangling(),
                len: 0,
                capacity,
                _arena: PhantomData,
            });
        }
        let bytes = elem
            .checked_mul(capacity)
            .ok_or_else(|| alloc_cap_exceeded("arena_vec byte size overflow"))?;
        let raw = self.alloc(bytes, align)?;
        let ptr = NonNull::new(raw.cast::<T>()).expect("arena alloc never returns null");
        Ok(ArenaVec {
            ptr,
            len: 0,
            capacity,
            _arena: PhantomData,
        })
    }

    /// Rewinds the cursor to zero in O(1). No `free` is performed — the backing
    /// block is retained for reuse. Any pointer handed out before the reset is
    /// invalidated (enforced at compile time for [`ArenaVec`] via its borrow).
    pub fn reset(&mut self) {
        self.cursor = 0;
        self.resets += 1;
    }

    /// Bytes currently consumed (including alignment padding) — the live cursor.
    pub fn used(&self) -> usize {
        self.cursor
    }

    /// Hard cap in bytes.
    pub fn capacity(&self) -> usize {
        self.cap
    }

    /// Peak bytes ever consumed between resets (high-water mark).
    pub fn high_water(&self) -> usize {
        self.high_water
    }

    /// Snapshot of allocation metrics for the metrics surface / FSV.
    pub fn stats(&self) -> AllocStats {
        AllocStats {
            arena_high_water_bytes: self.high_water,
            arena_resets: self.resets,
        }
    }
}

/// A typed, fixed-capacity vector backed by arena bytes for the arena's borrow.
///
/// Push initializes elements in place; only initialized elements are ever read,
/// and they are dropped when the `ArenaVec` is dropped (so `T`'s owned resources
/// do not leak even though the arena itself never frees per element).
pub struct ArenaVec<'arena, T> {
    ptr: NonNull<T>,
    len: usize,
    capacity: usize,
    _arena: PhantomData<&'arena mut Arena>,
}

impl<T> ArenaVec<'_, T> {
    /// Appends `value`.
    ///
    /// # Errors
    /// [`CALYX_ALLOC_CAP_EXCEEDED`](super::CALYX_ALLOC_CAP_EXCEEDED) if the
    /// vector is already at its reserved capacity (fail closed; never grows).
    pub fn push(&mut self, value: T) -> Result<()> {
        if self.len == self.capacity {
            return Err(alloc_cap_exceeded(format!(
                "arena_vec at capacity {} — cannot push",
                self.capacity
            )));
        }
        // SAFETY: `len < capacity`, so `ptr + len` is within the reserved,
        // correctly aligned region and currently uninitialized; we write once.
        unsafe { self.ptr.as_ptr().add(self.len).write(value) };
        self.len += 1;
        Ok(())
    }

    /// Number of initialized elements.
    pub fn len(&self) -> usize {
        self.len
    }

    /// True when no elements have been pushed.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Reserved element capacity.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// The initialized elements as a slice.
    pub fn as_slice(&self) -> &[T] {
        // SAFETY: elements `0..len` are initialized and the region is aligned.
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }

    /// The initialized elements as a mutable slice.
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        // SAFETY: elements `0..len` are initialized and uniquely borrowed.
        unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.len) }
    }
}

impl<T> Drop for ArenaVec<'_, T> {
    fn drop(&mut self) {
        // SAFETY: drop the `len` initialized elements in place so `T`'s owned
        // resources are released; the backing bytes belong to the arena.
        unsafe {
            std::ptr::drop_in_place(std::ptr::slice_from_raw_parts_mut(
                self.ptr.as_ptr(),
                self.len,
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alloc::CALYX_ALLOC_CAP_EXCEEDED;

    #[test]
    fn fills_exactly_cap_in_quarters_then_resets() {
        let cap = 4096;
        let mut a = Arena::new(cap).expect("arena");
        for i in 0..4 {
            let p = a.alloc(cap / 4, 1).expect("quarter alloc");
            assert!(!p.is_null());
            println!("after quarter {i}: used = {}", a.used());
        }
        assert_eq!(a.used(), cap, "four cap/4 align-1 allocs fill exactly cap");
        assert_eq!(a.high_water(), cap);
        a.reset();
        assert_eq!(a.used(), 0, "reset rewinds cursor to 0");
        assert_eq!(a.stats().arena_resets, 1);
        // High-water survives reset (it is the peak ever seen).
        assert_eq!(a.high_water(), cap);
    }

    #[test]
    fn cap_plus_one_fails_closed_without_advancing() {
        let cap = 1024;
        let mut a = Arena::new(cap).expect("arena");
        let _ = a.alloc(cap, 1).expect("fill to cap");
        let before = a.used();
        let err = a.alloc(1, 1).expect_err("one past cap must fail");
        println!("alloc past cap -> {} ; used still {}", err.code, a.used());
        assert_eq!(err.code, CALYX_ALLOC_CAP_EXCEEDED);
        assert_eq!(a.used(), before, "failed alloc does not advance the cursor");
    }

    #[test]
    fn alignment_padding_is_applied() {
        let mut a = Arena::new(256).expect("arena");
        let p1 = a.alloc(1, 1).expect("byte");
        assert!(!p1.is_null());
        let used_after_first = a.used();
        assert_eq!(used_after_first, 1);
        let p2 = a.alloc(8, 8).expect("aligned 8");
        assert_eq!(p2 as usize % 8, 0, "second pointer is 8-byte aligned");
        // 7 bytes of padding were inserted (cursor 1 -> 8) then 8 bytes used.
        assert_eq!(a.used(), 16, "1 + 7 pad + 8 = 16");
        println!("p1={p1:p} p2={p2:p} used={}", a.used());
    }

    #[test]
    fn zero_cap_rejected_at_construction() {
        let err = Arena::new(0).expect_err("zero cap rejected");
        assert_eq!(err.code, CALYX_ALLOC_CAP_EXCEEDED);
    }

    #[test]
    fn zero_size_alloc_is_noop_aligned() {
        let mut a = Arena::new(64).expect("arena");
        let p = a.alloc(0, 16).expect("zero-size alloc");
        assert_eq!(p as usize % 16, 0, "zero-size pointer is aligned");
        assert_eq!(a.used(), 0, "zero-size alloc consumes nothing");
    }

    #[test]
    fn padding_alone_can_exceed_cap() {
        // Tiny arena; first byte pushes the cursor to 1, then a 4096-aligned
        // request needs 4095 bytes of padding alone -> exceeds the 16-byte cap.
        let mut a = Arena::new(16).expect("arena");
        let _ = a.alloc(1, 1).expect("one byte");
        let err = a
            .alloc(1, ARENA_BASE_ALIGN)
            .expect_err("padding alone exceeds cap");
        println!("padding-exceeds-cap -> {}", err.code);
        assert_eq!(err.code, CALYX_ALLOC_CAP_EXCEEDED);
    }

    #[test]
    fn fail_closed_at_cap_minus_one() {
        let mut a = Arena::new(16).expect("arena");
        let _ = a.alloc(15, 1).expect("15 of 16");
        let err = a.alloc(2, 1).expect_err("2 more would exceed");
        assert_eq!(err.code, CALYX_ALLOC_CAP_EXCEEDED);
        assert_eq!(a.used(), 15, "no partial advance");
    }

    #[test]
    fn reset_is_o1_no_allocator_calls() {
        let mut a = Arena::new(4096).expect("arena");
        let iters = 1_000_000u32;
        let start = std::time::Instant::now();
        for _ in 0..iters {
            let _ = a.alloc(64, 8);
            a.reset();
        }
        let mean_ns = start.elapsed().as_nanos() / u128::from(iters);
        println!("mean reset+alloc = {mean_ns} ns over {iters} iters");
        assert_eq!(a.stats().arena_resets, u64::from(iters));
        // Generous ceiling: O(1) reset must not be doing per-op heap work.
        assert!(mean_ns < 1000, "reset path far from O(1): {mean_ns} ns");
    }

    #[test]
    fn arena_vec_typed_push_and_drop() {
        let mut a = Arena::new(4096).expect("arena");
        {
            let mut v = a.alloc_vec::<u64>(4).expect("alloc_vec");
            for i in 0..4u64 {
                v.push(i * 10).expect("push");
            }
            assert_eq!(v.as_slice(), &[0, 10, 20, 30]);
            let err = v.push(99).expect_err("past capacity");
            assert_eq!(err.code, CALYX_ALLOC_CAP_EXCEEDED);
            assert_eq!(v.len(), 4);
            println!("arena_vec = {:?}", v.as_slice());
        }
        // After the borrow ends the arena is usable again.
        a.reset();
        assert_eq!(a.used(), 0);
    }

    #[test]
    fn arena_vec_drops_owned_elements() {
        use std::rc::Rc;
        let mut a = Arena::new(4096).expect("arena");
        let shared = Rc::new(());
        {
            let mut v = a.alloc_vec::<Rc<()>>(3).expect("alloc_vec");
            v.push(Rc::clone(&shared)).unwrap();
            v.push(Rc::clone(&shared)).unwrap();
            assert_eq!(Rc::strong_count(&shared), 3);
        }
        // ArenaVec::drop must have dropped the two clones.
        assert_eq!(
            Rc::strong_count(&shared),
            1,
            "ArenaVec dropped its owned elements (no leak)"
        );
    }

    proptest::proptest! {
        #[test]
        fn sum_within_cap_all_succeed_else_first_overflow_fails(
            cap in 1usize..=1_048_576,
            sizes in proptest::collection::vec(0usize..4096, 0..64),
        ) {
            let mut a = Arena::new(cap).expect("arena");
            let mut running = 0usize; // align-1 so consumed == size
            let mut hit_cap = false;
            for s in sizes {
                let r = a.alloc(s, 1);
                if hit_cap {
                    // Once we've failed, smaller allocs may still fit; only
                    // assert the invariant used() <= cap always holds.
                    proptest::prop_assert!(a.used() <= cap);
                    continue;
                }
                if running + s <= cap {
                    proptest::prop_assert!(r.is_ok(), "alloc within cap must succeed");
                    running += s;
                    proptest::prop_assert_eq!(a.used(), running);
                } else {
                    proptest::prop_assert!(r.is_err(), "alloc crossing cap must fail");
                    proptest::prop_assert_eq!(r.unwrap_err().code, CALYX_ALLOC_CAP_EXCEEDED);
                    proptest::prop_assert_eq!(a.used(), running, "no advance on failure");
                    hit_cap = true;
                }
            }
            proptest::prop_assert!(a.used() <= cap);
        }
    }
}
