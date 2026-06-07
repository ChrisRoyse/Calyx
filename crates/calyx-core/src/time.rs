//! Clock injection and monotonic stamp types.

use std::time::{SystemTime, UNIX_EPOCH};

/// Monotonic store sequence number.
pub type Seq = u64;

/// Server timestamp in Unix milliseconds.
pub type Ts = u64;

/// Injected clock used by deterministic Calyx logic.
pub trait Clock: Send + Sync {
    /// Returns the current server timestamp.
    fn now(&self) -> Ts;
}

/// Real wall-clock implementation for outer runtime boundaries.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Ts {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is before Unix epoch")
            .as_millis();
        millis.try_into().unwrap_or(Ts::MAX)
    }
}

/// Deterministic fixed clock for tests and FSV.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FixedClock {
    ts: Ts,
}

impl FixedClock {
    /// Builds a fixed clock at `ts`.
    pub const fn new(ts: Ts) -> Self {
        Self { ts }
    }
}

impl Clock for FixedClock {
    fn now(&self) -> Ts {
        self.ts
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ConfidenceInterval, Signal};

    #[test]
    fn fixed_clock_makes_timestamped_operation_byte_deterministic() {
        fn stamped_signal(clock: &dyn Clock) -> Signal {
            Signal {
                bits: 0.25,
                ci: ConfidenceInterval {
                    low: 0.20,
                    high: 0.30,
                },
                n: 100,
                estimator: "fixed-clock-test".to_string(),
                ts: clock.now(),
            }
        }

        let clock = FixedClock::new(1_785_400_000);
        let first = serde_json::to_vec(&stamped_signal(&clock)).expect("serialize first signal");
        let second = serde_json::to_vec(&stamped_signal(&clock)).expect("serialize second signal");

        assert_eq!(first, second);
        assert_eq!(
            String::from_utf8(first).unwrap(),
            r#"{"bits":0.25,"ci":{"low":0.2,"high":0.3},"n":100,"estimator":"fixed-clock-test","ts":1785400000}"#
        );
    }
}
