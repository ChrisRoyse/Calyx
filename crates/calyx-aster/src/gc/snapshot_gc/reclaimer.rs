use super::{SnapshotPinWatchdog, duration_millis};
use calyx_core::{CalyxError, Clock, Result, Seq, SystemClock, Ts};
use serde::{Deserialize, Serialize};
use std::env;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Module-local fail-closed code for background GC scheduler failures.
pub const CALYX_GC_ERROR: &str = "CALYX_GC_ERROR";

/// Default cap for one snapshot-GC tick.
pub const DEFAULT_GC_MAX_OPS_PER_RUN: usize = 1_000;

/// Default minimum time between snapshot-GC ticks.
pub const DEFAULT_GC_MIN_INTERVAL_MS: u64 = 1_000;

const GC_MAX_OPS_ENV: &str = "CALYX_GC_MAX_OPS_PER_RUN";
const GC_MIN_INTERVAL_ENV: &str = "CALYX_GC_MIN_INTERVAL_MS";

/// Shared anti-storm rate limit for PH58 reclaimers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GcRateLimit {
    pub max_ops_per_run: usize,
    pub min_interval_ms: u64,
}

impl GcRateLimit {
    pub fn new(max_ops_per_run: usize, min_interval: Duration) -> Self {
        Self {
            max_ops_per_run,
            min_interval_ms: duration_millis(min_interval),
        }
    }

    pub const fn min_interval_ms(self) -> u64 {
        self.min_interval_ms
    }

    pub fn min_interval(self) -> Duration {
        Duration::from_millis(self.min_interval_ms)
    }

    /// Reads GC rate-limit configuration from env and fails closed on invalid values.
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            max_ops_per_run: parse_env_usize(GC_MAX_OPS_ENV, DEFAULT_GC_MAX_OPS_PER_RUN)?,
            min_interval_ms: parse_env_u64(GC_MIN_INTERVAL_ENV, DEFAULT_GC_MIN_INTERVAL_MS)?,
        })
    }
}

impl Default for GcRateLimit {
    fn default() -> Self {
        Self {
            max_ops_per_run: DEFAULT_GC_MAX_OPS_PER_RUN,
            min_interval_ms: DEFAULT_GC_MIN_INTERVAL_MS,
        }
    }
}

/// One snapshot-GC run result.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GcResult {
    pub safe_point_seq: Seq,
    pub versions_reclaimed: usize,
    pub bytes_freed: usize,
    pub compaction_debt: u64,
    pub rate_limited: bool,
}

/// Metrics emitted by resource-status Prometheus text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GcMetrics {
    pub versions_reclaimed_total: u64,
    pub bytes_freed_total: u64,
    pub soft_deletes_purged_total: u64,
    pub compaction_debt: u64,
}

/// Process-lifetime counters for snapshot-GC metrics.
#[derive(Debug, Default)]
pub struct SnapshotGcCounters {
    versions_reclaimed_total: AtomicU64,
    bytes_freed_total: AtomicU64,
    soft_deletes_purged_total: AtomicU64,
    compaction_debt: AtomicU64,
}

impl SnapshotGcCounters {
    pub fn record_result(&self, result: GcResult) {
        self.versions_reclaimed_total
            .fetch_add(result.versions_reclaimed as u64, Ordering::Relaxed);
        self.bytes_freed_total
            .fetch_add(result.bytes_freed as u64, Ordering::Relaxed);
        self.compaction_debt
            .store(result.compaction_debt, Ordering::Relaxed);
    }

    pub fn record_physical_bytes_freed(&self, bytes: usize) {
        self.bytes_freed_total
            .fetch_add(bytes as u64, Ordering::Relaxed);
    }

    pub fn record_soft_deletes_purged(&self, purged: usize) {
        self.soft_deletes_purged_total
            .fetch_add(purged as u64, Ordering::Relaxed);
    }

    pub fn metrics_with_debt(&self, compaction_debt: u64) -> GcMetrics {
        self.compaction_debt
            .store(compaction_debt, Ordering::Relaxed);
        self.metrics()
    }

    pub fn metrics(&self) -> GcMetrics {
        GcMetrics {
            versions_reclaimed_total: self.versions_reclaimed_total.load(Ordering::Relaxed),
            bytes_freed_total: self.bytes_freed_total.load(Ordering::Relaxed),
            soft_deletes_purged_total: self.soft_deletes_purged_total.load(Ordering::Relaxed),
            compaction_debt: self.compaction_debt.load(Ordering::Relaxed),
        }
    }
}

/// Backend that can reclaim snapshot-obsolete MVCC versions.
pub trait SnapshotVersionGc {
    fn reclaim_snapshot_versions(&self, safe_point: Seq, max_versions: usize) -> Result<GcResult>;

    fn snapshot_gc_debt(&self, safe_point: Seq) -> u64;
}

impl<T> SnapshotVersionGc for &T
where
    T: SnapshotVersionGc + ?Sized,
{
    fn reclaim_snapshot_versions(&self, safe_point: Seq, max_versions: usize) -> Result<GcResult> {
        (**self).reclaim_snapshot_versions(safe_point, max_versions)
    }

    fn snapshot_gc_debt(&self, safe_point: Seq) -> u64 {
        (**self).snapshot_gc_debt(safe_point)
    }
}

/// Rate-limited MVCC snapshot reclaimer.
#[derive(Debug)]
pub struct SnapshotGcReclaimer {
    rate_limit: Mutex<GcRateLimit>,
    last_run_at: Mutex<Option<Ts>>,
}

impl SnapshotGcReclaimer {
    pub fn new() -> Self {
        Self::with_rate_limit(GcRateLimit::default())
    }

    pub fn with_rate_limit(rate_limit: GcRateLimit) -> Self {
        Self {
            rate_limit: Mutex::new(rate_limit),
            last_run_at: Mutex::new(None),
        }
    }

    pub fn set_rate_limit(&self, rate_limit: GcRateLimit) {
        *self.rate_limit.lock().expect("snapshot GC rate poisoned") = rate_limit;
    }

    pub fn rate_limit(&self) -> GcRateLimit {
        *self.rate_limit.lock().expect("snapshot GC rate poisoned")
    }

    pub fn run_once<T>(
        &self,
        target: &T,
        watchdog: &SnapshotPinWatchdog,
        newest_seq: Seq,
    ) -> Result<GcResult>
    where
        T: SnapshotVersionGc + ?Sized,
    {
        let clock = SystemClock;
        self.run_once_at(target, &clock, watchdog, newest_seq)
    }

    pub fn run_once_at<T>(
        &self,
        target: &T,
        clock: &dyn Clock,
        watchdog: &SnapshotPinWatchdog,
        newest_seq: Seq,
    ) -> Result<GcResult>
    where
        T: SnapshotVersionGc + ?Sized,
    {
        let now = clock.now();
        let safe_point = watchdog.oldest_pinned_seq_at(now).unwrap_or(newest_seq);
        self.run_once_at_safe_point(target, clock, safe_point)
    }

    pub fn run_once_at_safe_point<T>(
        &self,
        target: &T,
        clock: &dyn Clock,
        safe_point: Seq,
    ) -> Result<GcResult>
    where
        T: SnapshotVersionGc + ?Sized,
    {
        let now = clock.now();
        let rate_limit = self.rate_limit();
        if !self.try_mark_run(now, rate_limit) {
            return Ok(GcResult {
                safe_point_seq: safe_point,
                compaction_debt: target.snapshot_gc_debt(safe_point),
                rate_limited: true,
                ..GcResult::default()
            });
        }

        let mut result =
            target.reclaim_snapshot_versions(safe_point, rate_limit.max_ops_per_run)?;
        result.safe_point_seq = safe_point;
        result.rate_limited |=
            result.compaction_debt > 0 && result.versions_reclaimed >= rate_limit.max_ops_per_run;
        Ok(result)
    }

    fn try_mark_run(&self, now: Ts, rate_limit: GcRateLimit) -> bool {
        let mut last = self
            .last_run_at
            .lock()
            .expect("snapshot GC last-run poisoned");
        if let Some(previous) = *last
            && now < previous.saturating_add(rate_limit.min_interval_ms)
        {
            return false;
        }
        *last = Some(now);
        true
    }
}

impl Default for SnapshotGcReclaimer {
    fn default() -> Self {
        Self::new()
    }
}

/// Runnable background GC task.
pub trait GcTask: Send + Sync {
    fn run_gc_tick(&self, clock: &dyn Clock) -> Result<GcResult>;
}

impl<F> GcTask for F
where
    F: Fn(&dyn Clock) -> Result<GcResult> + Send + Sync,
{
    fn run_gc_tick(&self, clock: &dyn Clock) -> Result<GcResult> {
        self(clock)
    }
}

/// One fail-closed scheduler tick.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GcSchedulerTick {
    pub result: Option<GcResult>,
    pub error_code: Option<&'static str>,
    pub errors_total: u64,
}

/// Fail-closed background scheduler wrapper for PH58 reclaimers.
#[derive(Debug)]
pub struct GcScheduler<T> {
    task: T,
    errors_total: AtomicU64,
    last_error_code: Mutex<Option<&'static str>>,
}

impl<T> GcScheduler<T> {
    pub fn new(task: T) -> Self {
        Self {
            task,
            errors_total: AtomicU64::new(0),
            last_error_code: Mutex::new(None),
        }
    }
}

impl<T> GcScheduler<T>
where
    T: GcTask,
{
    pub fn tick(&self, clock: &dyn Clock) -> GcSchedulerTick {
        match catch_unwind(AssertUnwindSafe(|| self.task.run_gc_tick(clock))) {
            Ok(Ok(result)) => {
                *self.last_error_code.lock().expect("GC error code poisoned") = None;
                GcSchedulerTick {
                    result: Some(result),
                    error_code: None,
                    errors_total: self.errors_total.load(Ordering::Relaxed),
                }
            }
            Ok(Err(error)) => self.record_error(format!("{CALYX_GC_ERROR}: {error}")),
            Err(_) => self.record_error(format!("{CALYX_GC_ERROR}: reclaimer panic")),
        }
    }

    fn record_error(&self, message: String) -> GcSchedulerTick {
        eprintln!("{message}");
        *self.last_error_code.lock().expect("GC error code poisoned") = Some(CALYX_GC_ERROR);
        let errors_total = self.errors_total.fetch_add(1, Ordering::Relaxed) + 1;
        GcSchedulerTick {
            result: None,
            error_code: Some(CALYX_GC_ERROR),
            errors_total,
        }
    }
}

pub fn gc_error(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_GC_ERROR,
        message: message.into(),
        remediation: "inspect GC evidence, fix the failing reclaimer, then retry",
    }
}

fn parse_env_usize(name: &str, default: usize) -> Result<usize> {
    let Some(value) = env::var_os(name) else {
        return Ok(default);
    };
    value
        .to_string_lossy()
        .parse()
        .map_err(|error| gc_error(format!("invalid {name}: {error}")))
}

fn parse_env_u64(name: &str, default: u64) -> Result<u64> {
    let Some(value) = env::var_os(name) else {
        return Ok(default);
    };
    value
        .to_string_lossy()
        .parse()
        .map_err(|error| gc_error(format!("invalid {name}: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

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

    #[derive(Debug)]
    struct FakeTarget {
        calls: AtomicUsize,
        debt: AtomicU64,
    }

    impl FakeTarget {
        fn new(debt: u64) -> Self {
            Self {
                calls: AtomicUsize::new(0),
                debt: AtomicU64::new(debt),
            }
        }
    }

    impl SnapshotVersionGc for FakeTarget {
        fn reclaim_snapshot_versions(
            &self,
            safe_point: Seq,
            max_versions: usize,
        ) -> Result<GcResult> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            let debt = self.debt.load(Ordering::Relaxed);
            let reclaimed = max_versions.min(debt as usize);
            let remaining = debt.saturating_sub(reclaimed as u64);
            self.debt.store(remaining, Ordering::Relaxed);
            Ok(GcResult {
                safe_point_seq: safe_point,
                versions_reclaimed: reclaimed,
                bytes_freed: reclaimed * 8,
                compaction_debt: remaining,
                rate_limited: remaining > 0,
            })
        }

        fn snapshot_gc_debt(&self, _safe_point: Seq) -> u64 {
            self.debt.load(Ordering::Relaxed)
        }
    }

    #[test]
    fn reclaimer_uses_oldest_pin_as_safe_point_and_caps_work() {
        let clock = TestClock::new(1_000);
        let watchdog = SnapshotPinWatchdog::default();
        watchdog.register(1, 50, Duration::from_secs(60));
        let target = FakeTarget::new(100);
        let reclaimer = SnapshotGcReclaimer::with_rate_limit(GcRateLimit::new(10, Duration::ZERO));

        let result = reclaimer
            .run_once_at(&target, &clock, &watchdog, 100)
            .expect("reclaim");

        assert_eq!(result.safe_point_seq, 50);
        assert_eq!(result.versions_reclaimed, 10);
        assert_eq!(result.bytes_freed, 80);
        assert_eq!(result.compaction_debt, 90);
        assert!(result.rate_limited);
    }

    #[test]
    fn min_interval_skips_second_tick_without_touching_target() {
        let clock = TestClock::new(1_000);
        let watchdog = SnapshotPinWatchdog::default();
        let target = FakeTarget::new(5);
        let reclaimer =
            SnapshotGcReclaimer::with_rate_limit(GcRateLimit::new(1, Duration::from_secs(5)));

        let first = reclaimer
            .run_once_at(&target, &clock, &watchdog, 10)
            .expect("first tick");
        assert_eq!(first.versions_reclaimed, 1);
        clock.set(1_100);
        let second = reclaimer
            .run_once_at(&target, &clock, &watchdog, 10)
            .expect("second tick");

        assert_eq!(second.versions_reclaimed, 0);
        assert_eq!(second.compaction_debt, 4);
        assert!(second.rate_limited);
        assert_eq!(target.calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn scheduler_catches_panic_and_reports_gc_error() {
        let clock = TestClock::new(1);
        let scheduler =
            GcScheduler::new(|_: &dyn Clock| -> Result<GcResult> { panic!("synthetic GC panic") });

        let tick = scheduler.tick(&clock);

        assert_eq!(tick.result, None);
        assert_eq!(tick.error_code, Some(CALYX_GC_ERROR));
        assert_eq!(tick.errors_total, 1);
    }
}
