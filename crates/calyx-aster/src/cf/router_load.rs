//! Cold-open scan of a CF router's SST files: fail-closed name
//! classification, seq-domain-safe ordering (issue #1138), and the per-CF
//! flush-ordinal counter.

use super::ColumnFamily;
use super::router::CfRouter;
use crate::sst::level::SstLevel;
use crate::storage_names::{
    SstName, classify_sst, ensure_unambiguous_sst_order, parse_cf_dir_name, sst_order_key,
};
use calyx_core::{CalyxError, Result};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

impl CfRouter {
    pub(super) fn load_existing(&mut self) -> Result<()> {
        let mut by_cf = HashMap::<ColumnFamily, Vec<PathBuf>>::new();
        for cf_root in self.cf_roots() {
            if !cf_root.exists() {
                continue;
            }
            for entry in fs::read_dir(cf_root)
                .map_err(|error| CalyxError::disk_pressure(format!("read CF root: {error}")))?
            {
                let path = entry
                    .map_err(|error| CalyxError::disk_pressure(format!("read CF entry: {error}")))?
                    .path();
                if !path.is_dir() {
                    continue;
                }
                let name = path
                    .file_name()
                    .map(|value| value.to_string_lossy().to_string())
                    .ok_or_else(|| {
                        CalyxError::aster_corrupt_shard(format!(
                            "CF directory entry {} has no name",
                            path.display()
                        ))
                    })?;
                let cf = parse_cf_dir_name(&name)?;
                by_cf.entry(cf).or_default().extend(list_sst_files(&path)?);
            }
        }
        for (cf, files) in by_cf {
            self.load_cf_level(cf, files)?;
        }
        Ok(())
    }

    pub(super) fn load_existing_cfs(&mut self, cfs: &[ColumnFamily]) -> Result<()> {
        let mut by_cf = HashMap::<ColumnFamily, Vec<PathBuf>>::new();
        for cf_root in self.cf_roots() {
            for cf in cfs {
                let cf_dir = cf_root.join(cf.name());
                if cf_dir.exists() {
                    by_cf
                        .entry(*cf)
                        .or_default()
                        .extend(list_sst_files(&cf_dir)?);
                }
            }
        }
        for cf in cfs {
            let files = by_cf.remove(cf).unwrap_or_default();
            self.load_cf_level(*cf, files)?;
        }
        Ok(())
    }

    fn load_cf_level(&mut self, cf: ColumnFamily, mut files: Vec<PathBuf>) -> Result<()> {
        sort_ssts_by_sequence(&mut files)?;
        files.dedup();
        // Only router-flushed SSTs (legacy and commit-anchored shapes)
        // participate in the next-file ordinal counter; durable batches and
        // compaction outputs use disjoint name shapes.
        let next = files
            .iter()
            .filter_map(|file| match classify_sst(file) {
                Ok(Some(SstName::RouterLegacy { ordinal })) => Some(ordinal),
                Ok(Some(SstName::Flush { ordinal, .. })) => Some(ordinal as u64),
                _ => None,
            })
            .max()
            .unwrap_or(0)
            + 1;
        self.ensure_cf(cf)?;
        self.levels.insert(cf, load_level_for_cf(cf, files)?);
        self.next_file.insert(cf, next);
        Ok(())
    }
}

fn load_level_for_cf(cf: ColumnFamily, files: Vec<PathBuf>) -> Result<SstLevel> {
    if eager_lookup_on_open(cf) {
        SstLevel::from_oldest_first_with_lookup(files)
    } else {
        Ok(SstLevel::from_oldest_first(files))
    }
}

fn eager_lookup_on_open(cf: ColumnFamily) -> bool {
    matches!(cf, ColumnFamily::Base)
}

/// Lists SST files in a CF directory, failing closed on any `*.sst` file
/// whose name matches no canonical writer shape (such files were previously
/// loaded into levels while being invisible to the next-file counter).
pub(super) fn list_sst_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir)
        .map_err(|error| CalyxError::disk_pressure(format!("read CF dir: {error}")))?
    {
        let path = entry
            .map_err(|error| CalyxError::disk_pressure(format!("read CF file: {error}")))?
            .path();
        if sst_order_key(&path)?.is_some() {
            files.push(path);
        }
    }
    Ok(files)
}

/// Sorts one CF's SST files into newest-wins read order, failing closed when
/// legacy flush ordinals and commit-domain seqs overlap (issue #1138) —
/// serving reads over an ambiguous order would silently return stale rows.
pub(super) fn sort_ssts_by_sequence(files: &mut [PathBuf]) -> Result<()> {
    ensure_unambiguous_sst_order(files.iter().map(PathBuf::as_path))?;
    let mut keyed = files
        .iter()
        .map(|path| {
            Ok((
                sst_order_key(path)?.ok_or_else(|| {
                    CalyxError::aster_corrupt_shard(format!(
                        "non-SST path {} in CF level",
                        path.display()
                    ))
                })?,
                path.clone(),
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    keyed.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    for (slot, (_, path)) in files.iter_mut().zip(keyed) {
        *slot = path;
    }
    Ok(())
}
