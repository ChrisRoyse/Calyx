//! Snapshot-safe SST compaction and hot/cold tier placement.

use crate::cf::{ColumnFamily, SlotFamilyKind};
use crate::sst::{SstReader, write_sst};
use calyx_core::{CalyxError, Result, SlotId};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

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
            "/zfs/hot/calyx",
            "/zfs/archive/calyx",
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
        if panel_version < self.current_panel_version || cf.is_raw_slot() {
            return true;
        }
        matches!(
            cf,
            ColumnFamily::Slot {
                slot,
                kind: SlotFamilyKind::Quantized
            } if !self.active_slots.contains(&slot)
        )
    }
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
