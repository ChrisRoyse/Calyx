//! Snapshot-pin watchdog for MVCC reader leases (PRD 24 §4).

use calyx_core::{Clock, Seq, SystemClock, Ts};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Reader id used by the watchdog and MVCC lease registry.
pub type ReaderId = u64;

/// Default reader lease age, matching the PH58 FoundationDB-style discipline.
pub const DEFAULT_READER_LEASE_MS: u64 = 5_000;

/// Default maximum allowed `newest_seq - oldest_pinned_seq` gap.
pub const DEFAULT_MAX_PINNED_SEQ_GAP: u64 = 1_000_000;

/// A bounded read lease that pins one MVCC sequence until expiry.
///
/// Calyx clocks use Unix-millisecond [`Ts`] values, so the persisted shape keeps
/// milliseconds while the public registration API accepts [`Duration`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadLease {
    pub seq: Seq,
    pub created_at: Ts,
    pub lease_duration_ms: u64,
    pub reader_id: ReaderId,
}

impl ReadLease {
    pub fn new(reader_id: ReaderId, seq: Seq, created_at: Ts, lease_duration: Duration) -> Self {
        Self::from_millis(reader_id, seq, created_at, duration_millis(lease_duration))
    }

    pub const fn from_millis(
        reader_id: ReaderId,
        seq: Seq,
        created_at: Ts,
        lease_duration_ms: u64,
    ) -> Self {
        Self {
            seq,
            created_at,
            lease_duration_ms,
            reader_id,
        }
    }

    pub fn is_expired(&self, clock: &dyn Clock) -> bool {
        self.is_expired_at(clock.now())
    }

    pub fn is_expired_at(&self, now: Ts) -> bool {
        now >= self.expires_at()
    }

    pub fn expires_at(&self) -> Ts {
        self.created_at.saturating_add(self.lease_duration_ms)
    }
}

/// Alert returned when an old reader pins too wide a sequence gap.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GapAlert {
    pub gap: u64,
    pub oldest_reader_id: ReaderId,
    pub oldest_pinned_seq: Seq,
    pub newest_seq: Seq,
    pub max_gap_seqs: u64,
}

/// Watchdog metrics surfaced through resource-status Prometheus text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotPinMetrics {
    pub reader_lease_expired_total: u64,
    pub oldest_pinned_seq_gap: u64,
}

/// One background-GC tick result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotGcTick {
    pub aborted_readers: Vec<ReaderId>,
    pub gap_alert: Option<GapAlert>,
    pub metrics: SnapshotPinMetrics,
}

/// Checkpoint-backed analytics snapshot that does not pin the live frontier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundedStalenessSnapshot {
    checkpoint_seq: Seq,
}

impl BoundedStalenessSnapshot {
    pub const fn at_checkpoint(seq: Seq) -> Self {
        Self {
            checkpoint_seq: seq,
        }
    }

    pub const fn seq(self) -> Seq {
        self.checkpoint_seq
    }
}

/// Watchdog over active snapshot pins.
pub struct SnapshotPinWatchdog {
    leases: Mutex<HashMap<ReaderId, ReadLease>>,
    max_gap_seqs: u64,
    clock: Arc<dyn Clock>,
    reader_lease_expired_total: AtomicU64,
    oldest_pinned_seq_gap: AtomicU64,
}

impl fmt::Debug for SnapshotPinWatchdog {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SnapshotPinWatchdog")
            .field("leases", &self.lock().len())
            .field("max_gap_seqs", &self.max_gap_seqs)
            .field(
                "reader_lease_expired_total",
                &self.reader_lease_expired_total(),
            )
            .field(
                "oldest_pinned_seq_gap",
                &self.oldest_pinned_seq_gap.load(Ordering::Relaxed),
            )
            .finish()
    }
}

impl Default for SnapshotPinWatchdog {
    fn default() -> Self {
        Self::new(Arc::new(SystemClock))
    }
}

impl SnapshotPinWatchdog {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self::with_max_gap(clock, DEFAULT_MAX_PINNED_SEQ_GAP)
    }

    pub fn with_max_gap(clock: Arc<dyn Clock>, max_gap_seqs: u64) -> Self {
        Self {
            leases: Mutex::new(HashMap::new()),
            max_gap_seqs,
            clock,
            reader_lease_expired_total: AtomicU64::new(0),
            oldest_pinned_seq_gap: AtomicU64::new(0),
        }
    }

    pub fn register(&self, reader_id: ReaderId, seq: Seq, duration: Duration) {
        let lease = ReadLease::new(reader_id, seq, self.clock.now(), duration);
        self.register_lease(lease);
    }

    pub fn register_lease(&self, lease: ReadLease) {
        self.lock().insert(lease.reader_id, lease);
    }

    pub fn release(&self, reader_id: ReaderId) -> bool {
        self.lock().remove(&reader_id).is_some()
    }

    pub fn abort_reader(&self, reader_id: ReaderId) -> Option<ReadLease> {
        self.lock().remove(&reader_id)
    }

    pub fn abort_if_expired_at(&self, reader_id: ReaderId, now: Ts) -> bool {
        let mut leases = self.lock();
        let expired = leases
            .get(&reader_id)
            .is_some_and(|lease| lease.is_expired_at(now));
        if expired {
            leases.remove(&reader_id);
            self.reader_lease_expired_total
                .fetch_add(1, Ordering::Relaxed);
        }
        expired
    }

    pub fn check_and_abort_expired(&self) -> Vec<ReaderId> {
        self.check_and_abort_expired_at(self.clock.now())
    }

    pub fn check_and_abort_expired_at(&self, now: Ts) -> Vec<ReaderId> {
        let mut leases = self.lock();
        let mut expired = leases
            .iter()
            .filter_map(|(id, lease)| lease.is_expired_at(now).then_some(*id))
            .collect::<Vec<_>>();
        expired.sort_unstable();
        for id in &expired {
            leases.remove(id);
        }
        if !expired.is_empty() {
            self.reader_lease_expired_total
                .fetch_add(expired.len() as u64, Ordering::Relaxed);
        }
        expired
    }

    pub fn oldest_pinned_seq(&self) -> Option<Seq> {
        self.oldest_pinned_seq_at(self.clock.now())
    }

    pub fn oldest_pinned_seq_at(&self, now: Ts) -> Option<Seq> {
        self.check_and_abort_expired_at(now);
        self.lock().values().map(|lease| lease.seq).min()
    }

    pub fn lease_count(&self) -> usize {
        self.lock().len()
    }

    pub fn check_gap(&self, newest_seq: Seq) -> Option<GapAlert> {
        self.check_gap_at(newest_seq, self.clock.now())
    }

    pub fn check_gap_at(&self, newest_seq: Seq, now: Ts) -> Option<GapAlert> {
        self.check_gap_at_with_max(newest_seq, now, self.max_gap_seqs)
    }

    pub fn check_gap_at_with_max(
        &self,
        newest_seq: Seq,
        now: Ts,
        max_gap_seqs: u64,
    ) -> Option<GapAlert> {
        self.check_and_abort_expired_at(now);
        let oldest = self
            .lock()
            .values()
            .min_by_key(|lease| (lease.seq, lease.reader_id))
            .copied();
        let Some(oldest) = oldest else {
            self.oldest_pinned_seq_gap.store(0, Ordering::Relaxed);
            return None;
        };
        let gap = newest_seq.saturating_sub(oldest.seq);
        self.oldest_pinned_seq_gap.store(gap, Ordering::Relaxed);
        (gap > max_gap_seqs).then_some(GapAlert {
            gap,
            oldest_reader_id: oldest.reader_id,
            oldest_pinned_seq: oldest.seq,
            newest_seq,
            max_gap_seqs,
        })
    }

    pub fn metrics_at(&self, newest_seq: Seq, now: Ts) -> SnapshotPinMetrics {
        let _ = self.check_gap_at(newest_seq, now);
        SnapshotPinMetrics {
            reader_lease_expired_total: self.reader_lease_expired_total(),
            oldest_pinned_seq_gap: self.oldest_pinned_seq_gap.load(Ordering::Relaxed),
        }
    }

    pub fn reader_lease_expired_total(&self) -> u64 {
        self.reader_lease_expired_total.load(Ordering::Relaxed)
    }

    pub const fn max_gap_seqs(&self) -> u64 {
        self.max_gap_seqs
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<ReaderId, ReadLease>> {
        self.leases.lock().expect("snapshot watchdog poisoned")
    }
}

fn duration_millis(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

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
}
