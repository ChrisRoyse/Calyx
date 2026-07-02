//! Snapshot-safe SST compaction and hot/cold tier placement.

mod scan;
mod tiering;

use crate::cf::ColumnFamily;
use crate::sst::{SstReader, write_sst};
use crate::storage_names::{SstName, classify_sst};
use calyx_core::{CalyxError, Result};
use std::collections::BTreeMap;
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
        let compacted = SstShard::new(cf, &report.output_path, next_level)?;
        let mut next: Vec<_> = self
            .active
            .read()
            .expect("catalog lock")
            .iter()
            .filter(|shard| shard.cf != cf)
            .cloned()
            .collect();
        next.push(compacted);
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
    pub debt_trigger_score_milli: u64,
    pub max_write_amp_milli: u64,
    pub backoff_factor: u64,
    pub max_interval_ms: u64,
    pub output_root: PathBuf,
    pub tiering_policy: Option<TieringPolicy>,
}

impl Default for CompactionSchedulerOptions {
    fn default() -> Self {
        Self {
            interval_ms: 10_000,
            debt_trigger_score_milli: 1_000,
            max_write_amp_milli: 2_000,
            backoff_factor: 2,
            max_interval_ms: 60_000,
            output_root: env::temp_dir().join("calyx-compaction-scheduler"),
            tiering_policy: None,
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
            while !thread_stop.load(Ordering::Acquire) {
                thread::sleep(Duration::from_millis(interval_ms));
                if thread_stop.load(Ordering::Acquire) {
                    break;
                }
                // FIXME(PH46): replace fixed cadence with Anneal adaptive hook.
                for cf in catalog.column_families() {
                    let debt = catalog.debt_for_cf(cf, DEFAULT_COMPACTION_TARGET_BYTES);
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
                            eprintln!(
                                "calyx compaction scheduler error: cannot name commit-domain \
                                 output for CF {} in {}: {error}",
                                cf.name(),
                                dir.display()
                            );
                            continue;
                        }
                    };
                    match catalog.compact_cf(cf, output, CompactionThrottle::unlimited()) {
                        Ok(CompactionResult::Compacted(report))
                            if report.write_amp_milli > options.max_write_amp_milli =>
                        {
                            interval_ms = interval_ms
                                .saturating_mul(options.backoff_factor.max(1))
                                .min(options.max_interval_ms.max(1));
                        }
                        Ok(_) => {}
                        Err(error) => eprintln!(
                            "calyx compaction scheduler error: compact CF {} into {}: {error}",
                            cf.name(),
                            dir.display()
                        ),
                    }
                }
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
    const FIRST_INDEX: u16 = 9_000;
    const LAST_INDEX: u16 = 9_999;
    for index in (FIRST_INDEX..=LAST_INDEX).rev() {
        let path = dir.join(format!("{seq:020}-{index:04}.sst"));
        if !path.exists() {
            return Ok(path);
        }
    }
    Err(CalyxError {
        code: "CALYX_ASTER_COMPACTION_SLOTS_EXHAUSTED",
        message: format!(
            "no compaction adoption slot ({FIRST_INDEX}..={LAST_INDEX}) remains for commit seq \
             {seq} in {}",
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
    pub staging_parent: PathBuf,
}

pub fn compact_shards(
    cf: ColumnFamily,
    inputs: &[SstShard],
    output_path: impl AsRef<Path>,
    throttle: CompactionThrottle,
) -> Result<CompactionResult> {
    let debt_before = CompactionDebt::measure(inputs, DEFAULT_COMPACTION_TARGET_BYTES);
    if inputs.len() < 2 {
        return Ok(CompactionResult::Skipped { debt: debt_before });
    }
    if let Some(max) = throttle.max_input_bytes
        && debt_before.pending_bytes > max
    {
        return Ok(CompactionResult::Skipped { debt: debt_before });
    }

    let mut merged = BTreeMap::new();
    for shard in inputs {
        for entry in SstReader::open(&shard.path)?.iter()? {
            merged.insert(entry.key, entry.value);
        }
    }
    let entries: Vec<_> = merged
        .iter()
        .map(|(key, value)| (key.as_slice(), value.as_slice()))
        .collect();
    let logical_bytes = merged.values().map(|value| value.len() as u64).sum::<u64>();
    let output_path = output_path.as_ref().to_path_buf();
    let parent = output_path
        .parent()
        .ok_or_else(|| CalyxError::disk_pressure("compaction output has no parent"))?
        .to_path_buf();
    fs::create_dir_all(&parent).map_err(|error| {
        CalyxError::disk_pressure(format!("create compaction output dir: {error}"))
    })?;
    let summary = write_sst(&output_path, entries)?;
    let output = SstShard {
        cf,
        path: summary.path.clone(),
        level: inputs.iter().map(|shard| shard.level).max().unwrap_or(0) + 1,
        bytes: summary.bytes,
    };
    let debt_after = CompactionDebt::measure(&[output], DEFAULT_COMPACTION_TARGET_BYTES);
    let input_bytes = debt_before.pending_bytes;
    let write_amp_milli = summary.bytes.saturating_mul(WRITE_AMP_SCALE) / logical_bytes.max(1);

    Ok(CompactionResult::Compacted(CompactionReport {
        cf,
        input_files: inputs.len(),
        input_paths: inputs.iter().map(|shard| shard.path.clone()).collect(),
        input_bytes,
        output_bytes: summary.bytes,
        logical_bytes,
        write_amp_milli,
        reclaimed_input_files: 0,
        debt_before,
        debt_after,
        output_path,
        staging_parent: parent,
    }))
}

#[cfg(test)]
mod tests;
