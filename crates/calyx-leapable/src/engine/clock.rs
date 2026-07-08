use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use calyx_core::{Clock, Ts};

#[derive(Clone, Debug)]
pub(super) struct EngineClock {
    ts: Arc<AtomicU64>,
}

impl EngineClock {
    pub(super) fn new(ts: Ts) -> Self {
        Self {
            ts: Arc::new(AtomicU64::new(ts)),
        }
    }

    pub(super) fn set(&self, ts: Ts) {
        self.ts.store(ts, Ordering::Release);
    }
}

impl Clock for EngineClock {
    fn now(&self) -> Ts {
        self.ts.load(Ordering::Acquire)
    }
}
