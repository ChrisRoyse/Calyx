//! Sequence allocation, freshness, and reader lease handles.

use calyx_core::{CalyxError, Clock, Result, Seq, Ts};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Vault-wide monotonic sequence allocator.
#[derive(Debug)]
pub struct SeqAllocator {
    current: AtomicU64,
    allocated: AtomicBool,
}

impl SeqAllocator {
    /// Creates an allocator whose next committed write is `start + 1`.
    pub const fn new(start: Seq) -> Self {
        Self {
            current: AtomicU64::new(start),
            allocated: AtomicBool::new(false),
        }
    }

    /// Allocates the next write sequence.
    pub fn allocate(&self) -> Seq {
        self.allocated.store(true, Ordering::Release);
        self.current.fetch_add(1, Ordering::AcqRel) + 1
    }

    /// Returns the latest committed sequence.
    pub fn current(&self) -> Seq {
        self.current.load(Ordering::Acquire)
    }

    /// Resets the recovered start sequence before any allocation in this process.
    pub fn set_start_seq(&self, seq: Seq) -> Result<()> {
        if self.allocated.load(Ordering::Acquire) {
            return Err(CalyxError::backpressure(
                "cannot reset MVCC start seq after allocation",
            ));
        }
        self.current.store(seq, Ordering::Release);
        Ok(())
    }

    /// Advances a live allocator after reading externally committed durable rows.
    pub fn advance_to_at_least(&self, seq: Seq) {
        let mut current = self.current();
        while current < seq {
            match self
                .current
                .compare_exchange(current, seq, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => return,
                Err(next_current) => current = next_current,
            }
        }
    }
}

impl Default for SeqAllocator {
    fn default() -> Self {
        Self::new(0)
    }
}

/// Derived-index freshness policy for reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Freshness {
    /// Derived structures must be built at or after the pinned base sequence.
    FreshDerived,
    /// Derived structures may lag the pinned base sequence by at most `max_lag`.
    StaleOk { max_lag: Seq },
}

impl Freshness {
    /// Verifies a derived structure is acceptable for the pinned snapshot.
    pub fn ensure(self, pinned_seq: Seq, derived_seq: Seq) -> Result<()> {
        if derived_seq >= pinned_seq {
            return Ok(());
        }
        let lag = pinned_seq - derived_seq;
        match self {
            Self::FreshDerived => Err(CalyxError::stale_derived(format!(
                "derived seq {derived_seq} is behind pinned seq {pinned_seq}"
            ))),
            Self::StaleOk { max_lag } if lag <= max_lag => Ok(()),
            Self::StaleOk { max_lag } => Err(CalyxError::stale_derived(format!(
                "derived lag {lag} exceeds allowed lag {max_lag}"
            ))),
        }
    }
}

/// A bounded reader lease that pins one MVCC sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReaderLease {
    id: u64,
    pinned_seq: Seq,
    issued_at: Ts,
    max_age_ms: u64,
}

impl ReaderLease {
    pub const fn new(id: u64, pinned_seq: Seq, issued_at: Ts, max_age_ms: u64) -> Self {
        Self {
            id,
            pinned_seq,
            issued_at,
            max_age_ms,
        }
    }

    pub const fn id(self) -> u64 {
        self.id
    }

    pub const fn pinned_seq(self) -> Seq {
        self.pinned_seq
    }

    pub const fn issued_at(self) -> Ts {
        self.issued_at
    }

    pub const fn max_age_ms(self) -> u64 {
        self.max_age_ms
    }

    pub fn expires_at(self) -> Ts {
        self.issued_at.saturating_add(self.max_age_ms)
    }

    pub fn is_expired_at(self, now: Ts) -> bool {
        now >= self.expires_at()
    }

    pub fn is_expired(self, clock: &dyn Clock) -> bool {
        self.is_expired_at(clock.now())
    }

    pub fn ensure_live_at(self, now: Ts) -> Result<()> {
        if self.is_expired_at(now) {
            return Err(CalyxError::reader_lease_expired(format!(
                "reader lease {} for seq {} expired at {}",
                self.id,
                self.pinned_seq,
                self.expires_at()
            )));
        }
        Ok(())
    }

    pub fn ensure_live(self, clock: &dyn Clock) -> Result<()> {
        self.ensure_live_at(clock.now())
    }
}

/// Snapshot handle pinned to one sequence and guarded by a lease.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Snapshot {
    seq: Seq,
    freshness: Freshness,
    lease: ReaderLease,
}

impl Snapshot {
    pub const fn new(seq: Seq, freshness: Freshness, lease: ReaderLease) -> Self {
        Self {
            seq,
            freshness,
            lease,
        }
    }

    pub const fn seq(self) -> Seq {
        self.seq
    }

    pub const fn freshness(self) -> Freshness {
        self.freshness
    }

    pub const fn lease(self) -> ReaderLease {
        self.lease
    }

    pub fn ensure_live(self, clock: &dyn Clock) -> Result<()> {
        self.lease.ensure_live(clock)
    }
}
