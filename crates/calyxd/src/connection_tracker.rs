use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

pub(crate) const DEFAULT_MAX_CONNECTIONS: usize = 128;
pub(crate) const MAX_CONNECTION_LIMIT_CEILING: usize = 4096;

pub(crate) struct ConnectionTracker {
    active: AtomicUsize,
    max: usize,
    lock: Mutex<()>,
    drained: Condvar,
}

impl ConnectionTracker {
    pub(crate) fn new(max: usize) -> Result<Self, String> {
        if max == 0 || max > MAX_CONNECTION_LIMIT_CEILING {
            return Err(format!(
                "connection limit {max} out of range (must be 1..={MAX_CONNECTION_LIMIT_CEILING})"
            ));
        }
        Ok(Self {
            active: AtomicUsize::new(0),
            max,
            lock: Mutex::new(()),
            drained: Condvar::new(),
        })
    }

    pub(crate) fn try_enter(&self) -> bool {
        let mut current = self.active.load(Ordering::SeqCst);
        loop {
            if current >= self.max {
                return false;
            }
            match self.active.compare_exchange_weak(
                current,
                current + 1,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => return true,
                Err(next) => current = next,
            }
        }
    }

    pub(crate) fn exit(&self) {
        let _guard = self.lock.lock().expect("connection tracker lock poisoned");
        let previous = self.active.fetch_sub(1, Ordering::SeqCst);
        if previous == 1 {
            self.drained.notify_all();
        }
    }

    pub(crate) fn active(&self) -> usize {
        self.active.load(Ordering::SeqCst)
    }

    pub(crate) fn max(&self) -> usize {
        self.max
    }

    pub(crate) fn wait_for_drain(&self, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        let mut guard = self.lock.lock().expect("connection tracker lock poisoned");
        while self.active() > 0 {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            let remaining = deadline.saturating_duration_since(now);
            let (next_guard, _) = self
                .drained
                .wait_timeout(guard, remaining)
                .expect("connection tracker wait poisoned");
            guard = next_guard;
        }
    }
}

impl Default for ConnectionTracker {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_CONNECTIONS).expect("default connection limit is valid")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn try_enter_enforces_limit_and_releases_slots() {
        let tracker = ConnectionTracker::new(1).unwrap();
        assert!(tracker.try_enter());
        assert!(!tracker.try_enter());
        assert_eq!(tracker.active(), 1);
        tracker.exit();
        assert_eq!(tracker.active(), 0);
        assert!(tracker.try_enter());
        tracker.exit();
    }

    #[test]
    fn invalid_limits_fail_closed() {
        assert!(ConnectionTracker::new(0).is_err());
        assert!(ConnectionTracker::new(MAX_CONNECTION_LIMIT_CEILING + 1).is_err());
    }
}
