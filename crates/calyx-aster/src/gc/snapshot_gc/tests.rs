use super::*;
use proptest::prelude::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

#[derive(Debug)]
struct TestClock {
    now: AtomicU64,
}

impl TestClock {
    fn new(now: Ts) -> Self {
        Self {
            now: AtomicU64::new(now),
        }
    }

    fn set(&self, now: Ts) {
        self.now.store(now, Ordering::Relaxed);
    }
}

impl Clock for TestClock {
    fn now(&self) -> Ts {
        self.now.load(Ordering::Relaxed)
    }
}

fn watchdog_at(now: Ts) -> (Arc<TestClock>, SnapshotPinWatchdog) {
    let clock = Arc::new(TestClock::new(now));
    let dyn_clock: Arc<dyn Clock> = clock.clone();
    (clock, SnapshotPinWatchdog::new(dyn_clock))
}

#[test]
fn expired_lease_is_aborted_and_counted() {
    let (clock, watchdog) = watchdog_at(1_000);
    watchdog.register(7, 42, Duration::from_millis(100));

    clock.set(1_101);
    assert_eq!(watchdog.check_and_abort_expired(), vec![7]);
    assert_eq!(watchdog.oldest_pinned_seq(), None);
    assert_eq!(watchdog.reader_lease_expired_total(), 1);
}

#[test]
fn two_readers_abort_only_the_expired_one() {
    let (clock, watchdog) = watchdog_at(1_000);
    watchdog.register(1, 100, Duration::from_millis(100));
    watchdog.register(2, 200, Duration::from_millis(500));

    clock.set(1_101);
    assert_eq!(watchdog.check_and_abort_expired(), vec![1]);
    assert_eq!(watchdog.lease_count(), 1);
    assert_eq!(watchdog.oldest_pinned_seq(), Some(200));
}

#[test]
fn oldest_pinned_seq_tracks_release() {
    let (_, watchdog) = watchdog_at(1_000);
    watchdog.register(1, 100, Duration::from_secs(60));
    watchdog.register(2, 200, Duration::from_secs(60));
    watchdog.register(3, 50, Duration::from_secs(60));

    assert_eq!(watchdog.oldest_pinned_seq(), Some(50));
    assert!(watchdog.release(3));
    assert_eq!(watchdog.oldest_pinned_seq(), Some(100));
}

#[test]
fn gap_alert_uses_hand_computed_difference() {
    let (_, watchdog) = watchdog_at(1_000);
    watchdog.register(1, 50, Duration::from_secs(60));

    let alert = watchdog.check_gap(1_100_000).expect("gap exceeds max");
    assert_eq!(alert.gap, 1_099_950);
    assert_eq!(alert.oldest_reader_id, 1);
    assert_eq!(watchdog.check_gap(1_000_000), None);
}

#[test]
fn bounded_staleness_checkpoint_does_not_register_a_pin() {
    let (_, watchdog) = watchdog_at(1_000);
    let checkpoint = BoundedStalenessSnapshot::at_checkpoint(77);

    assert_eq!(checkpoint.seq(), 77);
    assert_eq!(watchdog.oldest_pinned_seq(), None);
    assert_eq!(watchdog.lease_count(), 0);
}

#[test]
fn empty_and_exact_boundary_edges_are_fail_closed() {
    let (clock, watchdog) = watchdog_at(1_000);
    assert_eq!(watchdog.oldest_pinned_seq(), None);
    assert_eq!(watchdog.check_gap(9), None);

    watchdog.register(9, 10, Duration::from_millis(100));
    clock.set(1_100);
    assert_eq!(watchdog.check_and_abort_expired(), vec![9]);
    assert_eq!(watchdog.reader_lease_expired_total(), 1);
}

proptest! {
    #[test]
    fn aborts_exactly_expired_leases(
        durations in proptest::collection::vec(0u64..1_000, 0..32),
        advance in 0u64..1_000,
    ) {
        let start = 10_000;
        let clock = Arc::new(TestClock::new(start));
        let dyn_clock: Arc<dyn Clock> = clock.clone();
        let watchdog = SnapshotPinWatchdog::new(dyn_clock);
        for (index, duration) in durations.iter().enumerate() {
            let id = index as u64 + 1;
            watchdog.register(id, id * 10, Duration::from_millis(*duration));
        }
        clock.set(start + advance);

        let aborted = watchdog.check_and_abort_expired();
        let expected = durations
            .iter()
            .enumerate()
            .filter_map(|(index, duration)| (advance >= *duration).then_some(index as u64 + 1))
            .collect::<Vec<_>>();
        let expected_len = expected.len();

        prop_assert_eq!(aborted, expected);
        prop_assert_eq!(watchdog.lease_count(), durations.len() - expected_len);
    }
}
