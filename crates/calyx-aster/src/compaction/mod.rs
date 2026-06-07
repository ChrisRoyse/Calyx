//! Snapshot-safe SST compaction and hot/cold tier placement.

use crate::cf::{ColumnFamily, SlotFamilyKind};
use crate::sst::{SstReader, write_sst};
use calyx_core::{CalyxError, Result, SlotId};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const DEFAULT_COMPACTION_TARGET_BYTES: u64 = 64 * 1024 * 1024;
const WRITE_AMP_SCALE: u64 = 1_000;

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
            let run_id = AtomicU64::new(0);
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
                    let output = scheduler_output_path(&options.output_root, cf, &run_id);
                    match catalog.compact_cf(cf, output, CompactionThrottle::unlimited()) {
                        Ok(CompactionResult::Compacted(report))
                            if report.write_amp_milli > options.max_write_amp_milli =>
                        {
                            interval_ms = interval_ms
                                .saturating_mul(options.backoff_factor.max(1))
                                .min(options.max_interval_ms.max(1));
                        }
                        Ok(_) => {}
                        Err(error) => eprintln!("calyx compaction scheduler error: {error}"),
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

fn scheduler_output_path(root: &Path, cf: ColumnFamily, run_id: &AtomicU64) -> PathBuf {
    let id = run_id.fetch_add(1, Ordering::AcqRel) + 1;
    root.join(cf.name()).join(format!("compacted-{id:020}.sst"))
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
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub logical_bytes: u64,
    pub write_amp_milli: u64,
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
    if inputs.is_empty() {
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
        input_bytes,
        output_bytes: summary.bytes,
        logical_bytes,
        write_amp_milli,
        debt_before,
        debt_after,
        output_path,
        staging_parent: parent,
    }))
}

/// Hot/cold physical storage tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageTier {
    Hot,
    Cold,
}

/// Resolved destination for one CF write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TierPlacement {
    pub tier: StorageTier,
    pub root: PathBuf,
    pub cf_dir: PathBuf,
}

impl TierPlacement {
    pub fn absolute_dir(&self) -> PathBuf {
        self.root.join(&self.cf_dir)
    }
}

/// PH11 tiering policy.
#[derive(Debug, Clone)]
pub struct TieringPolicy {
    hot_root: PathBuf,
    archive_root: PathBuf,
    active_slots: BTreeSet<SlotId>,
    current_panel_version: u32,
}

impl TieringPolicy {
    pub fn new(
        hot_root: impl Into<PathBuf>,
        archive_root: impl Into<PathBuf>,
        active_slots: impl IntoIterator<Item = SlotId>,
        current_panel_version: u32,
    ) -> Self {
        Self {
            hot_root: hot_root.into(),
            archive_root: archive_root.into(),
            active_slots: active_slots.into_iter().collect(),
            current_panel_version,
        }
    }

    pub fn aiwonder(
        active_slots: impl IntoIterator<Item = SlotId>,
        current_panel_version: u32,
    ) -> Self {
        Self::new(
            tier_root("/zfs/hot/calyx", "hot"),
            tier_root("/zfs/archive/calyx", "archive"),
            active_slots,
            current_panel_version,
        )
    }

    pub fn place_cf(&self, cf: ColumnFamily, panel_version: u32) -> TierPlacement {
        let cold = self.is_cold(cf, panel_version);
        let root = if cold {
            self.archive_root.clone()
        } else {
            self.hot_root.clone()
        };
        TierPlacement {
            tier: if cold {
                StorageTier::Cold
            } else {
                StorageTier::Hot
            },
            root,
            cf_dir: PathBuf::from("cf").join(cf.name()),
        }
    }

    pub fn write_tiered_sst<'a>(
        &self,
        cf: ColumnFamily,
        panel_version: u32,
        file_name: &str,
        entries: impl IntoIterator<Item = (&'a [u8], &'a [u8])>,
    ) -> Result<TierWrite> {
        let placement = self.place_cf(cf, panel_version);
        let dir = placement.absolute_dir();
        fs::create_dir_all(&dir)
            .map_err(|error| CalyxError::disk_pressure(format!("create tier dir: {error}")))?;
        let path = dir.join(file_name);
        let summary = write_sst(&path, entries)?;
        Ok(TierWrite {
            placement,
            path: summary.path,
            bytes: summary.bytes,
            staging_parent: dir,
        })
    }

    fn is_cold(&self, cf: ColumnFamily, panel_version: u32) -> bool {
        if matches!(
            cf,
            ColumnFamily::Base | ColumnFamily::Ledger | ColumnFamily::Anchors
        ) {
            return false;
        }
        if cf.is_raw_slot() {
            return true;
        }
        matches!(
            cf,
            ColumnFamily::Slot {
                slot,
                kind: SlotFamilyKind::Quantized
            } if panel_version < self.current_panel_version || !self.active_slots.contains(&slot)
        )
    }
}

fn tier_root(zfs_path: &str, fallback_dir: &str) -> PathBuf {
    let zfs = PathBuf::from(zfs_path);
    if zfs.exists() {
        return zfs;
    }
    env::var_os("CALYX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/home/croyse/calyx"))
        .join(fallback_dir)
}

/// Completed tiered SST write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TierWrite {
    pub placement: TierPlacement,
    pub path: PathBuf,
    pub bytes: u64,
    pub staging_parent: PathBuf,
}

#[cfg(test)]
mod tests;
