//! PH58 ANN tombstone GC with read-safe copy-on-write swaps.

use calyx_core::{CalyxError, Clock, Result, SystemClock};
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

pub const CALYX_IO_ERROR: &str = "CALYX_IO_ERROR";
pub const DEFAULT_ANN_REBUILD_INTERVAL_MS: u64 = 10 * 60 * 1_000;
pub const DEFAULT_ANN_MAX_TOMBSTONE_RATIO: f64 = 0.25;
pub const DEFAULT_ANN_MAX_SERVING_IO_LOAD: f64 = 0.80;

const SKIP_INTERVAL: &str = "ann_rebuild_interval_not_elapsed";
const SKIP_LOW_RATIO: &str = "ann_tombstone_ratio_below_trigger";
const SKIP_HIGH_LOAD: &str = "serving_io_load_above_threshold";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnnTombstoneStats {
    pub index_id: String,
    pub total_nodes: usize,
    pub tombstoned_nodes: usize,
    pub live_nodes: usize,
}

impl AnnTombstoneStats {
    pub fn tombstone_ratio(&self) -> f64 {
        if self.total_nodes == 0 {
            0.0
        } else {
            self.tombstoned_nodes as f64 / self.total_nodes as f64
        }
    }
}

pub trait AnnIndexGraph: Clone + Send + Sync + 'static {
    fn ann_index_id(&self) -> String;
    fn ann_tombstone_stats(&self) -> AnnTombstoneStats;
    fn rebuild_without_tombstones(&self) -> Result<Self>;
}

pub trait AnnGcTarget {
    fn ann_tombstone_stats(&self, index_id: &str) -> Result<AnnTombstoneStats>;
    fn purge_ann_tombstones(&self, index_id: &str) -> Result<AnnTombstoneStats>;
}

#[derive(Debug)]
pub struct SharedAnnIndex<T> {
    current: RwLock<Arc<T>>,
}

impl<T> SharedAnnIndex<T>
where
    T: AnnIndexGraph,
{
    pub fn new(index: T) -> Self {
        Self {
            current: RwLock::new(Arc::new(index)),
        }
    }

    pub fn current(&self) -> Result<Arc<T>> {
        self.current
            .read()
            .map(|guard| Arc::clone(&guard))
            .map_err(|_| ann_io_error("ANN index read lock poisoned"))
    }
}

impl<T> AnnGcTarget for SharedAnnIndex<T>
where
    T: AnnIndexGraph,
{
    fn ann_tombstone_stats(&self, index_id: &str) -> Result<AnnTombstoneStats> {
        let current = self.current()?;
        ensure_index_id(index_id, current.ann_index_id())?;
        Ok(current.ann_tombstone_stats())
    }

    fn purge_ann_tombstones(&self, index_id: &str) -> Result<AnnTombstoneStats> {
        let old = self.current()?;
        ensure_index_id(index_id, old.ann_index_id())?;
        let rebuilt = Arc::new(old.rebuild_without_tombstones()?);
        let after = rebuilt.ann_tombstone_stats();
        let mut guard = self
            .current
            .write()
            .map_err(|_| ann_io_error("ANN index write lock poisoned"))?;
        *guard = rebuilt;
        Ok(after)
    }
}

#[derive(Debug)]
pub struct AnnGcReclaimer {
    pub rebuild_interval: Duration,
    pub max_tombstone_ratio: f64,
    pub max_serving_io_load: f64,
    rebuild_total: AtomicU64,
    last_run_at_ms: Mutex<Option<u64>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AnnGcResult {
    pub triggered: bool,
    pub rate_limited: bool,
    pub skipped_reason: Option<&'static str>,
    pub error_code: Option<&'static str>,
    pub error_message: Option<String>,
    pub index_id: String,
    pub tombstone_ratio_before: f64,
    pub tombstone_ratio_after: f64,
    pub total_nodes_before: usize,
    pub total_nodes_after: usize,
    pub tombstoned_nodes_before: usize,
    pub tombstoned_nodes_after: usize,
    pub live_nodes_after: usize,
    pub rebuild_total: u64,
}

impl AnnGcResult {
    pub fn to_metrics_text(&self, vault_label: &str) -> String {
        let vault = escape_label(vault_label);
        let index = escape_label(&self.index_id);
        let mut out = String::new();
        let _ = writeln!(
            out,
            "calyx_ann_tombstone_ratio{{vault=\"{vault}\",index=\"{index}\"}} {:.6}",
            self.tombstone_ratio_after
        );
        let _ = writeln!(
            out,
            "calyx_ann_gc_rebuild_total{{vault=\"{vault}\",index=\"{index}\"}} {}",
            self.rebuild_total
        );
        out
    }

    fn skipped(
        reason: &'static str,
        rate_limited: bool,
        before: AnnTombstoneStats,
        rebuild_total: u64,
    ) -> Self {
        let ratio = before.tombstone_ratio();
        Self {
            triggered: false,
            rate_limited,
            skipped_reason: Some(reason),
            error_code: None,
            error_message: None,
            index_id: before.index_id,
            tombstone_ratio_before: ratio,
            tombstone_ratio_after: ratio,
            total_nodes_before: before.total_nodes,
            total_nodes_after: before.total_nodes,
            tombstoned_nodes_before: before.tombstoned_nodes,
            tombstoned_nodes_after: before.tombstoned_nodes,
            live_nodes_after: before.live_nodes,
            rebuild_total,
        }
    }

    fn error(index_id: &str, before: Option<AnnTombstoneStats>, error: CalyxError) -> Self {
        let stats = before.unwrap_or_else(|| AnnTombstoneStats {
            index_id: index_id.to_string(),
            total_nodes: 0,
            tombstoned_nodes: 0,
            live_nodes: 0,
        });
        let ratio = stats.tombstone_ratio();
        Self {
            triggered: false,
            rate_limited: false,
            skipped_reason: None,
            error_code: Some(error.code),
            error_message: Some(error.message),
            index_id: stats.index_id,
            tombstone_ratio_before: ratio,
            tombstone_ratio_after: ratio,
            total_nodes_before: stats.total_nodes,
            total_nodes_after: stats.total_nodes,
            tombstoned_nodes_before: stats.tombstoned_nodes,
            tombstoned_nodes_after: stats.tombstoned_nodes,
            live_nodes_after: stats.live_nodes,
            rebuild_total: 0,
        }
    }
}

impl AnnGcReclaimer {
    pub fn new() -> Self {
        Self::with_limits(
            Duration::from_millis(DEFAULT_ANN_REBUILD_INTERVAL_MS),
            DEFAULT_ANN_MAX_TOMBSTONE_RATIO,
            DEFAULT_ANN_MAX_SERVING_IO_LOAD,
        )
    }

    pub fn with_limits(
        rebuild_interval: Duration,
        max_tombstone_ratio: f64,
        max_serving_io_load: f64,
    ) -> Self {
        Self {
            rebuild_interval,
            max_tombstone_ratio,
            max_serving_io_load,
            rebuild_total: AtomicU64::new(0),
            last_run_at_ms: Mutex::new(None),
        }
    }

    pub fn tombstone_ratio<T>(&self, target: &T, index_id: &str) -> Result<f64>
    where
        T: AnnGcTarget + ?Sized,
    {
        Ok(target.ann_tombstone_stats(index_id)?.tombstone_ratio())
    }

    pub fn run_once<T>(&self, target: &T, index_id: &str, serving_io_load: f64) -> AnnGcResult
    where
        T: AnnGcTarget + ?Sized,
    {
        let clock = SystemClock;
        self.run_once_at(target, index_id, serving_io_load, clock.now())
    }

    pub fn run_once_at<T>(
        &self,
        target: &T,
        index_id: &str,
        serving_io_load: f64,
        now_ms: u64,
    ) -> AnnGcResult
    where
        T: AnnGcTarget + ?Sized,
    {
        let before = match target.ann_tombstone_stats(index_id) {
            Ok(stats) => stats,
            Err(error) => return AnnGcResult::error(index_id, None, error),
        };
        let rebuild_total = self.rebuild_total.load(Ordering::Relaxed);
        if !self.interval_elapsed(now_ms) {
            return AnnGcResult::skipped(SKIP_INTERVAL, true, before, rebuild_total);
        }
        if before.tombstone_ratio() <= self.max_tombstone_ratio {
            return AnnGcResult::skipped(SKIP_LOW_RATIO, false, before, rebuild_total);
        }
        if serving_io_load > self.max_serving_io_load {
            return AnnGcResult::skipped(SKIP_HIGH_LOAD, true, before, rebuild_total);
        }
        self.mark_run(now_ms);
        match target.purge_ann_tombstones(index_id) {
            Ok(after) => {
                let total = self.rebuild_total.fetch_add(1, Ordering::Relaxed) + 1;
                AnnGcResult {
                    triggered: true,
                    rate_limited: false,
                    skipped_reason: None,
                    error_code: None,
                    error_message: None,
                    index_id: after.index_id.clone(),
                    tombstone_ratio_before: before.tombstone_ratio(),
                    tombstone_ratio_after: after.tombstone_ratio(),
                    total_nodes_before: before.total_nodes,
                    total_nodes_after: after.total_nodes,
                    tombstoned_nodes_before: before.tombstoned_nodes,
                    tombstoned_nodes_after: after.tombstoned_nodes,
                    live_nodes_after: after.live_nodes,
                    rebuild_total: total,
                }
            }
            Err(error) => AnnGcResult::error(index_id, Some(before), error),
        }
    }

    fn interval_elapsed(&self, now_ms: u64) -> bool {
        let last = self
            .last_run_at_ms
            .lock()
            .expect("ANN GC last-run poisoned");
        last.is_none_or(|previous| now_ms >= previous.saturating_add(self.rebuild_interval_ms()))
    }

    fn mark_run(&self, now_ms: u64) {
        *self
            .last_run_at_ms
            .lock()
            .expect("ANN GC last-run poisoned") = Some(now_ms);
    }

    fn rebuild_interval_ms(&self) -> u64 {
        self.rebuild_interval
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX)
    }
}

impl Default for AnnGcReclaimer {
    fn default() -> Self {
        Self::new()
    }
}

pub fn ann_io_error(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_IO_ERROR,
        message: message.into(),
        remediation: "retain the live ANN graph, inspect rebuild I/O, and retry below serving load",
    }
}

fn ensure_index_id(requested: &str, actual: String) -> Result<()> {
    if requested == actual {
        Ok(())
    } else {
        Err(ann_io_error(format!(
            "ANN index id mismatch: requested {requested}, current {actual}"
        )))
    }
}

fn escape_label(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests;
