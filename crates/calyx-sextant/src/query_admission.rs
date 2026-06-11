//! Bounded query admission for Sextant read/search paths.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use calyx_core::{CalyxError, Result};
use serde::{Deserialize, Serialize};

const DEFAULT_MAX_CONCURRENT_QUERIES: usize = 128;
const DEFAULT_MAX_QUEUED_QUERIES: usize = 512;
const DEFAULT_QUEUE_TIMEOUT_MILLIS: u64 = 250;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryAdmissionConfig {
    pub max_concurrent: usize,
    pub max_queued: usize,
    pub queue_timeout: Duration,
}

impl Default for QueryAdmissionConfig {
    fn default() -> Self {
        Self {
            max_concurrent: DEFAULT_MAX_CONCURRENT_QUERIES,
            max_queued: DEFAULT_MAX_QUEUED_QUERIES,
            queue_timeout: Duration::from_millis(DEFAULT_QUEUE_TIMEOUT_MILLIS),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryAdmissionStats {
    pub max_concurrent: usize,
    pub max_queued: usize,
    pub queue_timeout_millis: u64,
    pub in_flight: usize,
    pub queued: usize,
    pub admitted_total: u64,
    pub queued_total: u64,
    pub rejected_total: u64,
    pub deadline_rejected_total: u64,
    pub queue_full_rejected_total: u64,
    pub max_observed_in_flight: u64,
    pub max_observed_queued: u64,
}

#[derive(Clone, Debug)]
pub struct QueryAdmissionController {
    inner: Arc<QueryAdmissionInner>,
}

impl QueryAdmissionController {
    pub fn new(config: QueryAdmissionConfig) -> Self {
        Self {
            inner: Arc::new(QueryAdmissionInner::new(config)),
        }
    }

    pub fn acquire(&self) -> Result<QueryAdmissionPermit> {
        if self.inner.config.max_concurrent == 0 {
            self.inner
                .queue_full_rejected_total
                .fetch_add(1, Ordering::Relaxed);
            self.inner.rejected_total.fetch_add(1, Ordering::Relaxed);
            return Err(CalyxError::backpressure(
                "query admission has zero concurrent capacity",
            ));
        }
        let mut state = self.lock_state()?;
        if self.inner.try_admit(&mut state) {
            return Ok(QueryAdmissionPermit::new(Arc::clone(&self.inner)));
        }
        if state.queued >= self.inner.config.max_queued {
            self.inner
                .queue_full_rejected_total
                .fetch_add(1, Ordering::Relaxed);
            self.inner.rejected_total.fetch_add(1, Ordering::Relaxed);
            return Err(CalyxError::backpressure(format!(
                "query admission queue full: in_flight={} queued={} max_concurrent={} max_queued={}",
                state.in_flight,
                state.queued,
                self.inner.config.max_concurrent,
                self.inner.config.max_queued
            )));
        }
        state.queued += 1;
        self.inner.queued_total.fetch_add(1, Ordering::Relaxed);
        self.inner
            .max_observed_queued
            .fetch_max(state.queued as u64, Ordering::Relaxed);
        self.wait_for_permit(state)
    }

    pub fn stats(&self) -> QueryAdmissionStats {
        let state = match self.inner.state.lock() {
            Ok(guard) => *guard,
            Err(poisoned) => *poisoned.into_inner(),
        };
        QueryAdmissionStats {
            max_concurrent: self.inner.config.max_concurrent,
            max_queued: self.inner.config.max_queued,
            queue_timeout_millis: self.inner.config.queue_timeout.as_millis() as u64,
            in_flight: state.in_flight,
            queued: state.queued,
            admitted_total: self.inner.admitted_total.load(Ordering::Relaxed),
            queued_total: self.inner.queued_total.load(Ordering::Relaxed),
            rejected_total: self.inner.rejected_total.load(Ordering::Relaxed),
            deadline_rejected_total: self.inner.deadline_rejected_total.load(Ordering::Relaxed),
            queue_full_rejected_total: self.inner.queue_full_rejected_total.load(Ordering::Relaxed),
            max_observed_in_flight: self.inner.max_observed_in_flight.load(Ordering::Relaxed),
            max_observed_queued: self.inner.max_observed_queued.load(Ordering::Relaxed),
        }
    }

    pub fn metrics_text(&self) -> String {
        let stats = self.stats();
        format!(
            "calyx_query_admission_in_flight {}\n\
             calyx_query_admission_queued {}\n\
             calyx_query_admission_max_concurrent {}\n\
             calyx_query_admission_max_queued {}\n\
             calyx_query_admission_admitted_total {}\n\
             calyx_query_admission_queued_total {}\n\
             calyx_query_admission_rejected_total {}\n\
             calyx_query_admission_deadline_rejected_total {}\n\
             calyx_query_admission_queue_full_rejected_total {}\n\
             calyx_query_admission_max_observed_in_flight {}\n\
             calyx_query_admission_max_observed_queued {}\n",
            stats.in_flight,
            stats.queued,
            stats.max_concurrent,
            stats.max_queued,
            stats.admitted_total,
            stats.queued_total,
            stats.rejected_total,
            stats.deadline_rejected_total,
            stats.queue_full_rejected_total,
            stats.max_observed_in_flight,
            stats.max_observed_queued
        )
    }

    fn lock_state(&self) -> Result<std::sync::MutexGuard<'_, QueryAdmissionState>> {
        self.inner
            .state
            .lock()
            .map_err(|_| CalyxError::backpressure("query admission state lock poisoned"))
    }

    fn wait_for_permit(
        &self,
        mut state: std::sync::MutexGuard<'_, QueryAdmissionState>,
    ) -> Result<QueryAdmissionPermit> {
        let deadline = Instant::now()
            .checked_add(self.inner.config.queue_timeout)
            .unwrap_or_else(Instant::now);
        loop {
            if Instant::now() >= deadline {
                return self.reject_deadline(state);
            }
            if self.inner.try_admit(&mut state) {
                state.queued = state.queued.saturating_sub(1);
                return Ok(QueryAdmissionPermit::new(Arc::clone(&self.inner)));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            let (next, timeout) = self
                .inner
                .cvar
                .wait_timeout(state, remaining)
                .map_err(|_| CalyxError::backpressure("query admission state lock poisoned"))?;
            state = next;
            if timeout.timed_out() {
                return self.reject_deadline(state);
            }
        }
    }

    fn reject_deadline(
        &self,
        mut state: std::sync::MutexGuard<'_, QueryAdmissionState>,
    ) -> Result<QueryAdmissionPermit> {
        state.queued = state.queued.saturating_sub(1);
        self.inner
            .deadline_rejected_total
            .fetch_add(1, Ordering::Relaxed);
        self.inner.rejected_total.fetch_add(1, Ordering::Relaxed);
        Err(CalyxError::backpressure(format!(
            "query admission deadline exceeded after {} ms",
            self.inner.config.queue_timeout.as_millis()
        )))
    }
}

impl Default for QueryAdmissionController {
    fn default() -> Self {
        Self::new(QueryAdmissionConfig::default())
    }
}

#[derive(Debug)]
struct QueryAdmissionInner {
    config: QueryAdmissionConfig,
    state: Mutex<QueryAdmissionState>,
    cvar: Condvar,
    admitted_total: AtomicU64,
    queued_total: AtomicU64,
    rejected_total: AtomicU64,
    deadline_rejected_total: AtomicU64,
    queue_full_rejected_total: AtomicU64,
    max_observed_in_flight: AtomicU64,
    max_observed_queued: AtomicU64,
}

impl QueryAdmissionInner {
    fn new(config: QueryAdmissionConfig) -> Self {
        Self {
            config,
            state: Mutex::new(QueryAdmissionState::default()),
            cvar: Condvar::new(),
            admitted_total: AtomicU64::new(0),
            queued_total: AtomicU64::new(0),
            rejected_total: AtomicU64::new(0),
            deadline_rejected_total: AtomicU64::new(0),
            queue_full_rejected_total: AtomicU64::new(0),
            max_observed_in_flight: AtomicU64::new(0),
            max_observed_queued: AtomicU64::new(0),
        }
    }

    fn try_admit(&self, state: &mut QueryAdmissionState) -> bool {
        if state.in_flight >= self.config.max_concurrent {
            return false;
        }
        state.in_flight += 1;
        self.admitted_total.fetch_add(1, Ordering::Relaxed);
        self.max_observed_in_flight
            .fetch_max(state.in_flight as u64, Ordering::Relaxed);
        true
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct QueryAdmissionState {
    in_flight: usize,
    queued: usize,
}

#[derive(Debug)]
pub struct QueryAdmissionPermit {
    inner: Arc<QueryAdmissionInner>,
}

impl QueryAdmissionPermit {
    fn new(inner: Arc<QueryAdmissionInner>) -> Self {
        Self { inner }
    }
}

impl Drop for QueryAdmissionPermit {
    fn drop(&mut self) {
        let mut state = match self.inner.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        state.in_flight = state.in_flight.saturating_sub(1);
        self.inner.cvar.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn immediate_admission_is_bounded_by_concurrent_cap() {
        let controller = QueryAdmissionController::new(config(2, 0, 50));
        let first = controller.acquire().unwrap();
        let second = controller.acquire().unwrap();
        let rejected = controller.acquire().unwrap_err();

        assert_eq!(rejected.code, "CALYX_BACKPRESSURE");
        assert_eq!(controller.stats().in_flight, 2);
        assert_eq!(controller.stats().queue_full_rejected_total, 1);
        drop(first);
        drop(second);
        assert_eq!(controller.stats().in_flight, 0);
    }

    #[test]
    fn queued_waiter_admits_when_permit_releases_before_deadline() {
        let controller = QueryAdmissionController::new(config(1, 1, 1_000));
        let first = controller.acquire().unwrap();
        let waiting = controller.clone();
        let thread = thread::spawn(move || waiting.acquire().unwrap());
        wait_until(|| controller.stats().queued == 1);
        drop(first);
        let permit = thread.join().unwrap();
        assert_eq!(controller.stats().queued_total, 1);
        assert_eq!(controller.stats().max_observed_queued, 1);
        drop(permit);
        assert_eq!(controller.stats().in_flight, 0);
    }

    #[test]
    fn queued_waiter_rejects_when_deadline_expires() {
        let controller = QueryAdmissionController::new(config(1, 1, 10));
        let _first = controller.acquire().unwrap();
        let rejected = controller.acquire().unwrap_err();
        let stats = controller.stats();

        assert_eq!(rejected.code, "CALYX_BACKPRESSURE");
        assert_eq!(stats.queued, 0);
        assert_eq!(stats.queued_total, 1);
        assert_eq!(stats.deadline_rejected_total, 1);
        assert_eq!(stats.rejected_total, 1);
    }

    #[test]
    fn zero_concurrent_capacity_fails_closed_without_queue_growth() {
        let controller = QueryAdmissionController::new(config(0, 5, 50));
        let rejected = controller.acquire().unwrap_err();
        let stats = controller.stats();

        assert_eq!(rejected.code, "CALYX_BACKPRESSURE");
        assert_eq!(stats.in_flight, 0);
        assert_eq!(stats.queued, 0);
        assert_eq!(stats.rejected_total, 1);
    }

    fn config(max_concurrent: usize, max_queued: usize, timeout_ms: u64) -> QueryAdmissionConfig {
        QueryAdmissionConfig {
            max_concurrent,
            max_queued,
            queue_timeout: Duration::from_millis(timeout_ms),
        }
    }

    fn wait_until(mut predicate: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if predicate() {
                return;
            }
            thread::yield_now();
        }
        panic!("condition was not reached before timeout");
    }
}
