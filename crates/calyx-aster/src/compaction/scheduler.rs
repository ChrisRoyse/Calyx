use crate::cf::ColumnFamily;
use std::env;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use super::{
    CompactionCatalog, CompactionResult, CompactionThrottle, DEFAULT_COMPACTION_TARGET_BYTES,
    TieringPolicy, commit_domain_output_path,
};

/// Background compaction cadence and anti-storm controls.
#[derive(Debug, Clone)]
pub struct CompactionSchedulerOptions {
    pub interval_ms: u64,
    pub min_interval_ms: u64,
    pub debt_trigger_score_milli: u64,
    pub max_write_amp_milli: u64,
    pub backoff_factor: u64,
    pub debt_acceleration_factor: u64,
    pub max_interval_ms: u64,
    pub io_budget_bytes: Option<u64>,
    pub output_root: PathBuf,
    pub tiering_policy: Option<TieringPolicy>,
    pub schedule_hook: Arc<dyn CompactionScheduleHook>,
}

impl Default for CompactionSchedulerOptions {
    fn default() -> Self {
        Self {
            interval_ms: 10_000,
            min_interval_ms: 1_000,
            debt_trigger_score_milli: 1_000,
            max_write_amp_milli: 2_000,
            backoff_factor: 2,
            debt_acceleration_factor: 2,
            max_interval_ms: 60_000,
            io_budget_bytes: None,
            output_root: env::temp_dir().join("calyx-compaction-scheduler"),
            tiering_policy: None,
            schedule_hook: Arc::new(AdaptiveCompactionSchedule),
        }
    }
}

/// Scheduler state passed to the adaptive compaction cadence hook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactionScheduleState {
    pub current_interval_ms: u64,
    pub min_interval_ms: u64,
    pub max_interval_ms: u64,
    pub debt_trigger_score_milli: u64,
    pub max_debt_score_milli: u64,
    pub max_write_amp_milli: u64,
    pub observed_write_amp_milli: Option<u64>,
    pub backoff_factor: u64,
    pub debt_acceleration_factor: u64,
    pub compaction_attempts: usize,
    pub compacted_cfs: usize,
    pub io_budget_bytes: Option<u64>,
    pub io_budget_limited: bool,
}

/// Next scheduler cadence and per-run compaction byte budget.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactionScheduleDecision {
    pub next_interval_ms: u64,
    pub io_budget_bytes: Option<u64>,
}

/// Injectable source for compaction scheduler cadence decisions.
pub trait CompactionScheduleHook: std::fmt::Debug + Send + Sync {
    fn decide(&self, state: &CompactionScheduleState) -> CompactionScheduleDecision;
}

/// Background scheduler health counters exposed to vault health surfaces.
#[derive(Debug, Default)]
pub struct CompactionSchedulerHealth {
    error_count: AtomicU64,
}

impl CompactionSchedulerHealth {
    fn record_error(&self) {
        self.error_count.fetch_add(1, Ordering::AcqRel);
    }

    pub fn error_count(&self) -> u64 {
        self.error_count.load(Ordering::Acquire)
    }
}

/// Default adaptive hook: quiet periods back off, debt accelerates, and write
/// amplification or budget pressure slows the loop to avoid compaction storms.
#[derive(Debug, Clone, Copy, Default)]
pub struct AdaptiveCompactionSchedule;

impl CompactionScheduleHook for AdaptiveCompactionSchedule {
    fn decide(&self, state: &CompactionScheduleState) -> CompactionScheduleDecision {
        let min_interval = state.min_interval_ms.max(1);
        let max_interval = state.max_interval_ms.max(min_interval);
        let mut next = state.current_interval_ms.clamp(min_interval, max_interval);
        let write_amp_over_budget = state
            .observed_write_amp_milli
            .is_some_and(|value| value > state.max_write_amp_milli);

        if write_amp_over_budget || state.io_budget_limited {
            next = next
                .saturating_mul(state.backoff_factor.max(1))
                .min(max_interval);
        } else if state.max_debt_score_milli >= state.debt_trigger_score_milli {
            next = next
                .saturating_div(state.debt_acceleration_factor.max(1))
                .max(min_interval);
        } else {
            next = next
                .saturating_mul(state.backoff_factor.max(1))
                .min(max_interval);
        }

        CompactionScheduleDecision {
            next_interval_ms: next,
            io_budget_bytes: state.io_budget_bytes,
        }
    }
}

/// Background thread that compacts CFs whose debt crosses the configured trigger.
#[derive(Debug)]
pub struct CompactionScheduler {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
    health: Arc<CompactionSchedulerHealth>,
}

impl CompactionScheduler {
    pub fn start(catalog: Arc<CompactionCatalog>, options: CompactionSchedulerOptions) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let health = Arc::new(CompactionSchedulerHealth::default());
        let thread_health = health.clone();
        let thread = thread::spawn(move || {
            let mut interval_ms = options.interval_ms.max(1);
            let mut io_budget_bytes = options.io_budget_bytes;
            while !thread_stop.load(Ordering::Acquire) {
                thread::sleep(Duration::from_millis(interval_ms));
                if thread_stop.load(Ordering::Acquire) {
                    break;
                }
                let mut max_debt_score_milli = 0_u64;
                let mut observed_write_amp_milli = None::<u64>;
                let mut compaction_attempts = 0_usize;
                let mut compacted_cfs = 0_usize;
                let mut io_budget_limited = false;
                for cf in catalog.column_families() {
                    let debt = catalog.debt_for_cf(cf, DEFAULT_COMPACTION_TARGET_BYTES);
                    max_debt_score_milli = max_debt_score_milli.max(debt.score_milli);
                    if debt.score_milli < options.debt_trigger_score_milli {
                        continue;
                    }
                    // The scheduler itself is the only writer of this catalog,
                    // so this input set matches the one compact_cf pins below.
                    let inputs = catalog.shards_for_cf(cf);
                    let dir = scheduler_output_dir(
                        &options.output_root,
                        options.tiering_policy.as_ref(),
                        cf,
                    );
                    let output = match commit_domain_output_path(&dir, &inputs) {
                        Ok(output) => output,
                        Err(_) => {
                            thread_health.record_error();
                            continue;
                        }
                    };
                    let input_bytes = inputs.iter().map(|shard| shard.bytes).sum::<u64>();
                    let throttle = match io_budget_bytes {
                        Some(max_input_bytes) => {
                            if input_bytes > max_input_bytes {
                                io_budget_limited = true;
                            }
                            CompactionThrottle::max_input_bytes(max_input_bytes)
                        }
                        None => CompactionThrottle::unlimited(),
                    };
                    compaction_attempts += 1;
                    match catalog.compact_cf(cf, output, throttle) {
                        Ok(CompactionResult::Compacted(report)) => {
                            compacted_cfs += 1;
                            observed_write_amp_milli = Some(
                                observed_write_amp_milli
                                    .unwrap_or(0)
                                    .max(report.write_amp_milli),
                            );
                        }
                        Ok(CompactionResult::Skipped { .. }) => {}
                        Err(_) => {
                            thread_health.record_error();
                        }
                    }
                }
                let state = CompactionScheduleState {
                    current_interval_ms: interval_ms,
                    min_interval_ms: options.min_interval_ms,
                    max_interval_ms: options.max_interval_ms,
                    debt_trigger_score_milli: options.debt_trigger_score_milli,
                    max_debt_score_milli,
                    max_write_amp_milli: options.max_write_amp_milli,
                    observed_write_amp_milli,
                    backoff_factor: options.backoff_factor,
                    debt_acceleration_factor: options.debt_acceleration_factor,
                    compaction_attempts,
                    compacted_cfs,
                    io_budget_bytes,
                    io_budget_limited,
                };
                let decision = options.schedule_hook.decide(&state);
                interval_ms = decision.next_interval_ms.max(1);
                io_budget_bytes = decision.io_budget_bytes;
            }
        });
        Self {
            stop,
            thread,
            health,
        }
    }

    pub fn error_count(&self) -> u64 {
        self.health.error_count()
    }

    pub fn stop(self) -> thread::Result<()> {
        self.stop.store(true, Ordering::Release);
        self.thread.join()
    }
}

fn scheduler_output_dir(
    root: &Path,
    tiering_policy: Option<&TieringPolicy>,
    cf: ColumnFamily,
) -> PathBuf {
    if let Some(policy) = tiering_policy {
        policy.place_current_cf(cf).absolute_dir()
    } else {
        root.join(cf.name())
    }
}
