//! Snapshot-safe SST compaction and hot/cold tier placement.

mod rolling;
mod scan;
mod tiering;

use crate::cf::ColumnFamily;
use crate::sst::SstReader;
use crate::storage_names::{SstName, classify_sst};
use calyx_core::{CalyxError, Result};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// Default per-CF compaction target used for debt scoring (PRD 24 §8).
pub const DEFAULT_COMPACTION_TARGET_BYTES: u64 = 64 * 1024 * 1024;
const WRITE_AMP_SCALE: u64 = 1_000;
const COMPACTION_ADOPTION_FIRST_INDEX: usize = 9_000;
const COMPACTION_ADOPTION_LAST_INDEX: usize = 9_999;

pub(crate) use rolling::RollingSstWriter;
use rolling::compact_shards_with_target;
pub use scan::{catalog_from_vault_dir, catalog_from_vault_tiers};
pub use tiering::{StorageTier, TierPlacement, TierWrite, TieringPolicy};

/// One immutable SST file in the active shard set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SstShard {
    pub cf: ColumnFamily,
    pub path: PathBuf,
    pub level: u8,
    pub bytes: u64,
}

impl SstShard {
    pub fn new(cf: ColumnFamily, path: impl AsRef<Path>, level: u8) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let bytes = fs::metadata(&path)
            .map_err(|error| CalyxError::disk_pressure(format!("stat SST shard: {error}")))?
            .len();
        Ok(Self {
            cf,
            path,
            level,
            bytes,
        })
    }
}

/// Pinned view of the active shard set. Old views survive compaction swaps.
#[derive(Debug, Clone)]
pub struct CompactionSnapshot {
    shards: Arc<Vec<SstShard>>,
}

impl CompactionSnapshot {
    pub fn get(&self, cf: ColumnFamily, key: &[u8]) -> Result<Option<Vec<u8>>> {
        for shard in self.shards.iter().rev().filter(|shard| shard.cf == cf) {
            if let Some(value) = SstReader::open(&shard.path)?.get(key)? {
                return Ok(Some(value));
            }
        }
        Ok(None)
    }

    pub fn shard_count(&self) -> usize {
        self.shards.len()
    }

    pub fn shard_count_for_cf(&self, cf: ColumnFamily) -> usize {
        self.shards.iter().filter(|shard| shard.cf == cf).count()
    }
}

/// Active SST catalog with atomic snapshot swaps.
#[derive(Debug)]
pub struct CompactionCatalog {
    active: RwLock<Arc<Vec<SstShard>>>,
}

impl CompactionCatalog {
    pub fn new(shards: Vec<SstShard>) -> Self {
        Self {
            active: RwLock::new(Arc::new(shards)),
        }
    }

    pub fn pin_snapshot(&self) -> CompactionSnapshot {
        CompactionSnapshot {
            shards: self.active.read().expect("catalog lock").clone(),
        }
    }

    pub fn compact_cf(
        &self,
        cf: ColumnFamily,
        output_path: impl AsRef<Path>,
        throttle: CompactionThrottle,
    ) -> Result<CompactionResult> {
        let before = self.pin_snapshot();
        let inputs: Vec<_> = before
            .shards
            .iter()
            .filter(|shard| shard.cf == cf)
            .cloned()
            .collect();
        let CompactionResult::Compacted(report) =
            compact_shards(cf, &inputs, output_path, throttle)?
        else {
            return Ok(CompactionResult::Skipped {
                debt: CompactionDebt::measure(&inputs, DEFAULT_COMPACTION_TARGET_BYTES),
            });
        };

        let next_level = inputs.iter().map(|shard| shard.level).max().unwrap_or(0) + 1;
        let compacted = report
            .output_paths
            .iter()
            .map(|path| SstShard::new(cf, path, next_level))
            .collect::<Result<Vec<_>>>()?;
        let mut next: Vec<_> = self
            .active
            .read()
            .expect("catalog lock")
            .iter()
            .filter(|shard| shard.cf != cf)
            .cloned()
            .collect();
        next.extend(compacted);
        *self.active.write().expect("catalog lock") = Arc::new(next);
        Ok(CompactionResult::Compacted(report))
    }

    pub fn shard_count_for_cf(&self, cf: ColumnFamily) -> usize {
        self.pin_snapshot().shard_count_for_cf(cf)
    }

    pub fn shards_for_cf(&self, cf: ColumnFamily) -> Vec<SstShard> {
        self.pin_snapshot()
            .shards
            .iter()
            .filter(|shard| shard.cf == cf)
            .cloned()
            .collect()
    }

    pub fn debt_for_cf(&self, cf: ColumnFamily, target_bytes: u64) -> CompactionDebt {
        let snapshot = self.pin_snapshot();
        let inputs: Vec<_> = snapshot
            .shards
            .iter()
            .filter(|shard| shard.cf == cf)
            .cloned()
            .collect();
        CompactionDebt::measure(&inputs, target_bytes)
    }

    pub fn column_families(&self) -> Vec<ColumnFamily> {
        let snapshot = self.pin_snapshot();
        let mut cfs = Vec::new();
        for shard in snapshot.shards.iter() {
            if !cfs.contains(&shard.cf) {
                cfs.push(shard.cf);
            }
        }
        cfs
    }
}

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
}

impl CompactionScheduler {
    pub fn start(catalog: Arc<CompactionCatalog>, options: CompactionSchedulerOptions) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
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
                        Err(error) => {
                            panic!(
                                "calyx compaction scheduler error: cannot name commit-domain output for CF {} in {}: {error}",
                                cf.name(),
                                dir.display()
                            );
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
                        Err(error) => panic!(
                            "calyx compaction scheduler error: compact CF {} into {}: {error}",
                            cf.name(),
                            dir.display()
                        ),
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
        Self { stop, thread }
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

/// Commit-domain output path for a compaction of `inputs` (issue #1137).
///
/// The output is named from the highest commit-domain seq present in the
/// inputs — the highest seq at which the merged state is true — so
/// full-restore readback restores the merged rows at that seq instead of
/// misreading a foreign counter as commit seq ~1 and corrupting history.
/// Legacy router flushes carry no commit-domain bound and contribute seq 0;
/// their content stays sound under the durable-coverage invariant plus the
/// `ensure_unambiguous_sst_order` gate (issue #1138). Fails closed on
/// non-canonical input names and when no adoption slot remains.
pub fn commit_domain_output_path(dir: &Path, inputs: &[SstShard]) -> Result<PathBuf> {
    let mut max_seq = 0_u64;
    for shard in inputs {
        let name = classify_sst(&shard.path)?.ok_or_else(|| {
            CalyxError::aster_corrupt_shard(format!(
                "compaction input {} is not an SST file",
                shard.path.display()
            ))
        })?;
        let seq = match name {
            SstName::RouterLegacy { .. } => 0,
            SstName::Flush { watermark, .. } => watermark,
            SstName::DurableBatch { seq, .. } | SstName::Compacted { seq } => seq,
        };
        max_seq = max_seq.max(seq);
    }
    durable_compaction_slot_path(dir, max_seq)
}

/// First free `{seq:020}-{index:04}.sst` adoption slot in the reserved
/// `9000..=9999` index range (descending), the durable-batch-shaped naming
/// full-restore readback restores at `seq`. Shared by the CLI `compact`
/// command and the background scheduler so every commit-domain compaction
/// output is named through one implementation.
pub fn durable_compaction_slot_path(dir: &Path, seq: u64) -> Result<PathBuf> {
    for index in (COMPACTION_ADOPTION_FIRST_INDEX..=COMPACTION_ADOPTION_LAST_INDEX).rev() {
        let path = dir.join(format!("{seq:020}-{index:04}.sst"));
        if !path.exists() {
            return Ok(path);
        }
    }
    Err(CalyxError {
        code: "CALYX_ASTER_COMPACTION_SLOTS_EXHAUSTED",
        message: format!(
            "no compaction adoption slot ({}..={}) remains for commit seq {seq} in {}",
            COMPACTION_ADOPTION_FIRST_INDEX,
            COMPACTION_ADOPTION_LAST_INDEX,
            dir.display()
        ),
        remediation: "advance the vault's durable seq (write and flush) so compaction outputs \
                      move to a new seq, or remove superseded adoption slots via a full \
                      compaction, then retry",
    })
}

/// Per-run throttle. `None` means no byte cap for the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactionThrottle {
    pub max_input_bytes: Option<u64>,
}

impl CompactionThrottle {
    pub const fn unlimited() -> Self {
        Self {
            max_input_bytes: None,
        }
    }

    pub const fn max_input_bytes(max_input_bytes: u64) -> Self {
        Self {
            max_input_bytes: Some(max_input_bytes),
        }
    }
}

/// Compaction debt meter for anti-storm scheduling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactionDebt {
    pub pending_bytes: u64,
    pub target_bytes: u64,
    pub score_milli: u64,
}

impl CompactionDebt {
    pub fn measure(shards: &[SstShard], target_bytes: u64) -> Self {
        let pending_bytes = shards.iter().map(|shard| shard.bytes).sum();
        let target_bytes = target_bytes.max(1);
        Self {
            pending_bytes,
            target_bytes,
            score_milli: pending_bytes.saturating_mul(WRITE_AMP_SCALE) / target_bytes,
        }
    }
}

/// Result of one compaction attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompactionResult {
    Skipped { debt: CompactionDebt },
    Compacted(CompactionReport),
}

/// Physical compaction metrics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactionReport {
    pub cf: ColumnFamily,
    pub input_files: usize,
    pub input_paths: Vec<PathBuf>,
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub logical_bytes: u64,
    pub write_amp_milli: u64,
    pub reclaimed_input_files: usize,
    pub debt_before: CompactionDebt,
    pub debt_after: CompactionDebt,
    pub output_path: PathBuf,
    pub output_paths: Vec<PathBuf>,
    pub staging_parent: PathBuf,
}

pub fn compact_shards(
    cf: ColumnFamily,
    inputs: &[SstShard],
    output_path: impl AsRef<Path>,
    throttle: CompactionThrottle,
) -> Result<CompactionResult> {
    compact_shards_with_target(
        cf,
        inputs,
        output_path,
        throttle,
        DEFAULT_COMPACTION_TARGET_BYTES,
    )
}

#[cfg(test)]
mod streaming_tests;
#[cfg(test)]
mod tests;
