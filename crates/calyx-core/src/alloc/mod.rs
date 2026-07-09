//! Bounded allocation primitives (PH56 — Stage S13).
//!
//! These are the bounded building blocks for axiom A26: when a caller chooses
//! one of these primitives, the allocation has an owner, a hard cap, and a
//! fail-closed error path instead of a silent grow. They are currently wired
//! into PH56 soak/hazard surfaces and selected crate tests; production ingest,
//! query, kernel, and index paths must opt in explicitly before their heap use
//! is covered by these counters.
//!
//! - [`Arena`] — bump allocator for per-request / per-microbatch transient
//!   working sets (scoring buffers, cross-term/MI scratch). O(1) reset, no
//!   per-op `malloc`/`free` churn, fail-closed at the cap.
//! - [`SlabPool`] / [`PageAlignedSlabPool`] — fixed-size object pools for hot
//!   reused objects (vector blocks, ANN nodes, page-aligned staging buffers).
//!
//! All cap violations surface the single module-local code
//! [`CALYX_ALLOC_CAP_EXCEEDED`] (not a panic, not a silent realloc): the
//! allocation is denied and the caller decides how to back off. This is the
//! A26 invariant enforced for callers that route allocations through this
//! module.

pub mod arena;
pub mod slab;

pub use arena::{Arena, ArenaVec};
pub use slab::{
    AnnNode, AnnNodePool, DEFAULT_EMBED_DIM, PageAlignedSlabPool, PageSlabGuard, SlabGuard,
    SlabPool, VecBlockPool,
};

use crate::CalyxError;

/// An allocation would exceed its owner's hard cap. The allocation is denied
/// (fail closed) — never satisfied by a silent realloc or by exceeding the cap.
pub const CALYX_ALLOC_CAP_EXCEEDED: &str = "CALYX_ALLOC_CAP_EXCEEDED";

/// Builds the [`CALYX_ALLOC_CAP_EXCEEDED`] error with a concrete message.
pub(crate) fn alloc_cap_exceeded(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_ALLOC_CAP_EXCEEDED,
        message: message.into(),
        remediation: "raise the cap or shrink the working set; allocations fail closed (A26)",
    }
}

/// Snapshot of arena allocation metrics for arenas that callers explicitly
/// instantiate. This is not a process-wide heap census.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AllocStats {
    /// Peak bytes ever consumed by a single arena fill (high-water mark).
    pub arena_high_water_bytes: usize,
    /// Number of O(1) resets performed (monotonic counter).
    pub arena_resets: u64,
}
