//! PH58 WAL recycler with fsync anti-storm guards.

use crate::wal::{Wal, WalRecycleReport};
use calyx_core::{Clock, SystemClock};
use std::fmt::Write as _;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

pub const DEFAULT_MAX_RECYCLE_PER_TICK: usize = 8;
pub const DEFAULT_FSYNC_BUDGET_PER_TICK: usize = 8;
pub const DEFAULT_WAL_RECYCLER_MIN_INTERVAL_MS: u64 = 1_000;
pub const DEFAULT_FSYNC_P99_ALERT_US: u64 = 10_000;

const SKIP_DISABLED: &str = "wal_recycler_disabled";
const SKIP_FSYNC_GUARD: &str = "fsync_p99_guard";
const SKIP_BACKOFF: &str = "fsync_backoff_active";
const SKIP_NO_RECYCLABLE: &str = "no_recyclable_wal_segments";

/// Bounded WAL recycle policy.
#[derive(Debug)]
pub struct WalRecycler {
    pub max_recycle_per_tick: usize,
    pub fsync_budget_per_tick: usize,
    pub min_interval_ms: u64,
    pub fsync_p99_alert_us: u64,
    fsync_p99_us: AtomicU64,
    recycled_total: AtomicU64,
    backoff_until_ms: Mutex<Option<u64>>,
}

/// One WAL recycler tick result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WalRecyclerResult {
    pub triggered: bool,
    pub rate_limited: bool,
    pub skipped_reason: Option<&'static str>,
    pub error_code: Option<&'static str>,
    pub error_message: Option<String>,
    pub newest_durable_seq: u64,
    pub wal_bytes_active_before: u64,
    pub wal_bytes_active_after: u64,
    pub recyclable_segments_before: usize,
    pub segments_recycled: usize,
    pub bytes_recycled: u64,
    pub fsync_p99_us: u64,
    pub wal_segments_recycled_total: u64,
}

impl WalRecyclerResult {
    pub fn to_metrics_text(&self, vault_label: &str) -> String {
        let vault = escape_label(vault_label);
        let mut out = String::new();
        let _ = writeln!(
            out,
            "calyx_wal_bytes_active{{vault=\"{vault}\"}} {}",
            self.wal_bytes_active_after
        );
        let _ = writeln!(
            out,
            "calyx_wal_segments_recycled_total{{vault=\"{vault}\"}} {}",
            self.wal_segments_recycled_total
        );
        let _ = writeln!(
            out,
            "calyx_fsync_p99_us{{vault=\"{vault}\"}} {}",
            self.fsync_p99_us
        );
        out
    }

    fn skipped(
        reason: &'static str,
        rate_limited: bool,
        newest_durable_seq: u64,
        bytes_active: u64,
        fsync_p99_us: u64,
        recycled_total: u64,
    ) -> Self {
        Self {
            triggered: false,
            rate_limited,
            skipped_reason: Some(reason),
            error_code: None,
            error_message: None,
            newest_durable_seq,
            wal_bytes_active_before: bytes_active,
            wal_bytes_active_after: bytes_active,
            recyclable_segments_before: 0,
            segments_recycled: 0,
            bytes_recycled: 0,
            fsync_p99_us,
            wal_segments_recycled_total: recycled_total,
        }
    }

    fn error(newest_durable_seq: u64, error: calyx_core::CalyxError) -> Self {
        Self {
            triggered: false,
            rate_limited: false,
            skipped_reason: None,
            error_code: Some(error.code),
            error_message: Some(error.message),
            newest_durable_seq,
            wal_bytes_active_before: 0,
            wal_bytes_active_after: 0,
            recyclable_segments_before: 0,
            segments_recycled: 0,
            bytes_recycled: 0,
            fsync_p99_us: 0,
            wal_segments_recycled_total: 0,
        }
    }
}

impl WalRecycler {
    pub fn new() -> Self {
        Self::with_limits(
            DEFAULT_MAX_RECYCLE_PER_TICK,
            DEFAULT_FSYNC_BUDGET_PER_TICK,
            Duration::from_millis(DEFAULT_WAL_RECYCLER_MIN_INTERVAL_MS),
        )
    }

    pub fn with_limits(
        max_recycle_per_tick: usize,
        fsync_budget_per_tick: usize,
        min_interval: Duration,
    ) -> Self {
        Self {
            max_recycle_per_tick,
            fsync_budget_per_tick,
            min_interval_ms: min_interval.as_millis().try_into().unwrap_or(u64::MAX),
            fsync_p99_alert_us: DEFAULT_FSYNC_P99_ALERT_US,
            fsync_p99_us: AtomicU64::new(0),
            recycled_total: AtomicU64::new(0),
            backoff_until_ms: Mutex::new(None),
        }
    }

    pub fn set_fsync_p99_us(&self, value: u64) {
        self.fsync_p99_us.store(value, Ordering::Relaxed);
    }

    pub fn fsync_p99_us(&self) -> u64 {
        self.fsync_p99_us.load(Ordering::Relaxed)
    }

    pub fn fsync_p99_guard(&self) -> bool {
        self.fsync_p99_us() > self.fsync_p99_alert_us
    }

    pub fn run_once(&self, wal: &mut Wal, newest_durable_seq: u64) -> WalRecyclerResult {
        let clock = SystemClock;
        self.run_once_at(wal, newest_durable_seq, clock.now())
    }

    pub fn run_once_at(
        &self,
        wal: &mut Wal,
        newest_durable_seq: u64,
        now_ms: u64,
    ) -> WalRecyclerResult {
        let bytes_active = match wal.total_segment_bytes() {
            Ok(bytes) => bytes,
            Err(error) => return WalRecyclerResult::error(newest_durable_seq, error),
        };
        let fsync_p99_us = self.fsync_p99_us();
        let recycled_total = self.recycled_total.load(Ordering::Relaxed);
        if self.max_recycle_per_tick == 0 || self.fsync_budget_per_tick == 0 {
            return WalRecyclerResult::skipped(
                SKIP_DISABLED,
                false,
                newest_durable_seq,
                bytes_active,
                fsync_p99_us,
                recycled_total,
            );
        }
        if self.backoff_active(now_ms) {
            return WalRecyclerResult::skipped(
                SKIP_BACKOFF,
                true,
                newest_durable_seq,
                bytes_active,
                fsync_p99_us,
                recycled_total,
            );
        }
        if self.fsync_p99_guard() {
            self.set_backoff(now_ms);
            return WalRecyclerResult::skipped(
                SKIP_FSYNC_GUARD,
                true,
                newest_durable_seq,
                bytes_active,
                fsync_p99_us,
                recycled_total,
            );
        }

        match wal.recycle_durable_segments(
            newest_durable_seq,
            self.max_recycle_per_tick,
            self.fsync_budget_per_tick,
        ) {
            Ok(report) => self.result_from_report(report, fsync_p99_us),
            Err(error) => WalRecyclerResult::error(newest_durable_seq, error),
        }
    }

    fn result_from_report(&self, report: WalRecycleReport, fsync_p99_us: u64) -> WalRecyclerResult {
        if report.segments_recycled == 0 {
            return WalRecyclerResult::skipped(
                SKIP_NO_RECYCLABLE,
                false,
                report.newest_durable_seq,
                report.bytes_before,
                fsync_p99_us,
                self.recycled_total.load(Ordering::Relaxed),
            );
        }
        let total = self
            .recycled_total
            .fetch_add(report.segments_recycled as u64, Ordering::Relaxed)
            + report.segments_recycled as u64;
        WalRecyclerResult {
            triggered: true,
            rate_limited: report.segments_recycled < report.recyclable_segments_before,
            skipped_reason: None,
            error_code: None,
            error_message: None,
            newest_durable_seq: report.newest_durable_seq,
            wal_bytes_active_before: report.bytes_before,
            wal_bytes_active_after: report.bytes_after,
            recyclable_segments_before: report.recyclable_segments_before,
            segments_recycled: report.segments_recycled,
            bytes_recycled: report.bytes_recycled,
            fsync_p99_us,
            wal_segments_recycled_total: total,
        }
    }

    fn backoff_active(&self, now_ms: u64) -> bool {
        self.backoff_until_ms
            .lock()
            .expect("WAL recycler backoff poisoned")
            .is_some_and(|until| now_ms < until)
    }

    fn set_backoff(&self, now_ms: u64) {
        let until = now_ms.saturating_add(self.min_interval_ms.saturating_mul(2));
        *self
            .backoff_until_ms
            .lock()
            .expect("WAL recycler backoff poisoned") = Some(until);
    }
}

impl Default for WalRecycler {
    fn default() -> Self {
        Self::new()
    }
}

fn escape_label(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests;
