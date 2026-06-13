//! Token-bucket backpressure guard for the streaming ingest pipeline (A26).
//!
//! Bounded-by-construction: the bucket holds at most `capacity` tokens, each
//! [`StreamIngester::send`](super::StreamIngester::send) spends exactly one, and
//! a refill never pushes the count above `capacity`. When the budget is exhausted
//! the call fails with `CALYX_STREAM_BACKPRESSURE` instead of letting the channel
//! grow without bound.
//!
//! The read-modify-write on the token counter is a single `compare_exchange`
//! loop on one atomic. This is the *correct* lock-free shape — a single counter
//! mutated by an atomic RMW — and is deliberately not the multi-counter,
//! independent-store anti-pattern that races and silently over-admits (see the
//! fixed-window limiter failure documented in issue #703). Refill is driven by an
//! explicit elapsed-time argument so the bucket is fully deterministic under test
//! and FSV (the standard testable token-bucket pattern: inject the clock).

use std::sync::atomic::{AtomicUsize, Ordering};

use calyx_core::CalyxError;

/// Module-local error code: stream ingest backpressure tripped (A26).
///
/// Not a PRD 18 catalog entry; built directly per the closed-catalog doctrine.
pub const CALYX_STREAM_BACKPRESSURE: &str = "CALYX_STREAM_BACKPRESSURE";

const STREAM_BACKPRESSURE_REMEDIATION: &str = "retry after token refill or reduce the stream send rate; the bucket is bounded by capacity (A26)";

/// Builds a `CALYX_STREAM_BACKPRESSURE` error.
pub(crate) fn stream_backpressure_error(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_STREAM_BACKPRESSURE,
        message: message.into(),
        remediation: STREAM_BACKPRESSURE_REMEDIATION,
    }
}

/// Bounded token bucket enforcing A26 on the streaming ingest path.
#[derive(Debug)]
pub struct BackpressureGuard {
    tokens: AtomicUsize,
    capacity: usize,
    refill_rate: usize,
}

impl BackpressureGuard {
    /// Creates a guard with a full bucket of `capacity` tokens.
    ///
    /// `refill_rate` is the number of tokens replenished per millisecond when
    /// [`BackpressureGuard::refill`] is driven; `0` means a non-refilling bucket
    /// (each token is single-use until the guard is rebuilt).
    pub fn new(capacity: usize, refill_rate: usize) -> Self {
        Self {
            tokens: AtomicUsize::new(capacity),
            capacity,
            refill_rate,
        }
    }

    /// Maximum number of tokens the bucket can ever hold.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Tokens replenished per millisecond when refill is driven.
    pub fn refill_rate(&self) -> usize {
        self.refill_rate
    }

    /// Currently available tokens (observation only — racy by nature).
    pub fn available(&self) -> usize {
        self.tokens.load(Ordering::Acquire)
    }

    /// Atomically spends `n` tokens, or fails closed with
    /// `CALYX_STREAM_BACKPRESSURE`.
    ///
    /// A request for more tokens than `capacity` can ever satisfy fails
    /// immediately (it could never succeed even with a full bucket). Otherwise a
    /// single `compare_exchange` RMW loop guarantees no two concurrent callers
    /// double-spend the same token.
    pub fn acquire(&self, n: usize) -> Result<(), CalyxError> {
        if n > self.capacity {
            return Err(stream_backpressure_error(format!(
                "requested {n} tokens exceeds bucket capacity {}",
                self.capacity
            )));
        }
        if n == 0 {
            return Ok(());
        }
        let mut current = self.tokens.load(Ordering::Acquire);
        loop {
            if current < n {
                return Err(stream_backpressure_error(format!(
                    "only {current} tokens available, need {n}"
                )));
            }
            match self.tokens.compare_exchange_weak(
                current,
                current - n,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Ok(()),
                Err(observed) => current = observed,
            }
        }
    }

    /// Replenishes tokens for `elapsed_ms` of wall time, saturating at
    /// `capacity`. Never lets the bucket exceed `capacity` (A26).
    pub fn refill(&self, elapsed_ms: u64) {
        if self.refill_rate == 0 || elapsed_ms == 0 {
            return;
        }
        let add_u128 = (self.refill_rate as u128).saturating_mul(elapsed_ms as u128);
        let add = usize::try_from(add_u128).unwrap_or(self.capacity);
        let mut current = self.tokens.load(Ordering::Acquire);
        loop {
            let next = current.saturating_add(add).min(self.capacity);
            if next == current {
                return;
            }
            match self.tokens.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return,
                Err(observed) => current = observed,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn full_bucket_admits_exactly_capacity_then_fails_closed() {
        let guard = BackpressureGuard::new(5, 0);
        assert_eq!(guard.available(), 5);
        for i in 0..5 {
            guard
                .acquire(1)
                .unwrap_or_else(|_| panic!("token {i} should be available"));
        }
        assert_eq!(guard.available(), 0);
        let err = guard.acquire(1).expect_err("6th acquire must fail closed");
        assert_eq!(err.code, CALYX_STREAM_BACKPRESSURE);
    }

    #[test]
    fn acquire_more_than_capacity_fails_immediately() {
        let guard = BackpressureGuard::new(5, 0);
        let err = guard
            .acquire(6)
            .expect_err("over-capacity request must fail");
        assert_eq!(err.code, CALYX_STREAM_BACKPRESSURE);
        // A failed over-capacity acquire spends nothing.
        assert_eq!(guard.available(), 5);
    }

    #[test]
    fn refill_never_exceeds_capacity() {
        let guard = BackpressureGuard::new(10, 3);
        guard.acquire(10).expect("drain bucket");
        assert_eq!(guard.available(), 0);
        // 1000 ms * 3 tokens/ms = 3000 desired, capped at capacity 10.
        guard.refill(1000);
        assert_eq!(guard.available(), 10);
    }

    #[test]
    fn refill_zero_rate_is_a_noop() {
        let guard = BackpressureGuard::new(4, 0);
        guard.acquire(4).expect("drain");
        guard.refill(1_000_000);
        assert_eq!(guard.available(), 0);
    }

    #[test]
    fn concurrent_acquire_never_over_admits() {
        // 8 threads racing for a 100-token bucket must hand out exactly 100
        // successes — the multi-counter over-admit bug (#703) would hand out a
        // nondeterministic number > 100.
        let guard = Arc::new(BackpressureGuard::new(100, 0));
        let mut handles = Vec::new();
        for _ in 0..8 {
            let guard = Arc::clone(&guard);
            handles.push(thread::spawn(move || {
                let mut admitted = 0usize;
                for _ in 0..50 {
                    if guard.acquire(1).is_ok() {
                        admitted += 1;
                    }
                }
                admitted
            }));
        }
        let total: usize = handles.into_iter().map(|h| h.join().expect("join")).sum();
        assert_eq!(total, 100, "exactly capacity admitted across all threads");
        assert_eq!(guard.available(), 0);
    }
}
