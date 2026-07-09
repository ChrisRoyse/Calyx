use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Default)]
pub(crate) struct ConnectionTracker {
    active: AtomicUsize,
    lock: Mutex<()>,
    drained: Condvar,
}

impl ConnectionTracker {
    pub(crate) fn enter(&self) {
        self.active.fetch_add(1, Ordering::SeqCst);
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
