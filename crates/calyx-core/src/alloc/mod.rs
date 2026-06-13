//! Bounded allocation primitives (PH56 — Stage S13).
//!
//! Every transient and hot allocation in Calyx flows through one of these
//! primitives so that *every allocation has an owner and a hard bound* (axiom
//! A26) and nothing in the system can grow the heap without limit (A16):
//!
//! - [`Arena`] — bump allocator for per-request / per-microbatch transient
//!   working sets (scoring buffers, cross-term/MI scratch). O(1) reset, no
//!   per-op `malloc`/`free` churn, fail-closed at the cap.
//! - [`SlabPool`] / [`PageAlignedSlabPool`] — fixed-size object pools for hot
//!   reused objects (vector blocks, ANN nodes, GPU staging buffers).
//!
//! All cap violations surface the single module-local code
//! [`CALYX_ALLOC_CAP_EXCEEDED`] (not a panic, not a silent realloc): the
//! allocation is denied and the caller decides how to back off. This is the
//! A26 invariant enforced at the lowest level.

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

/// Snapshot of arena allocation metrics — the Source-of-Truth read for FSV and
/// for the metrics surface (a Prometheus exporter reads these counters).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AllocStats {
    /// Peak bytes ever consumed by a single arena fill (high-water mark).
    pub arena_high_water_bytes: usize,
    /// Number of O(1) resets performed (monotonic counter).
    pub arena_resets: u64,
}
