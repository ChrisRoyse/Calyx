use super::{CompactionCatalog, SstShard, TieringPolicy};
use crate::storage_names::{classify_sst, parse_cf_dir_name};
use calyx_core::{CalyxError, Result};
use std::fs;
use std::path::{Path, PathBuf};

pub fn catalog_from_vault_dir(vault_dir: impl AsRef<Path>) -> Result<CompactionCatalog> {
    catalog_from_vault_tiers(vault_dir, None)
}

pub fn catalog_from_vault_tiers(
    vault_dir: impl AsRef<Path>,
    tiering_policy: Option<&TieringPolicy>,
) -> Result<CompactionCatalog> {
    let mut shards = Vec::new();
    for cf_root in tiered_cf_roots(vault_dir.as_ref(), tiering_policy) {
        if !cf_root.exists() {
            continue;
        }
        for entry in fs::read_dir(&cf_root).map_err(|error| {
            CalyxError::disk_pressure(format!("read compaction CF root: {error}"))
        })? {
            let path = entry
                .map_err(|error| {
                    CalyxError::disk_pressure(format!("read compaction CF entry: {error}"))
                })?
                .path();
            if !path.is_dir() {
                continue;
            }
            let name = path
                .file_name()
                .map(|value| value.to_string_lossy().to_string())
                .ok_or_else(|| {
                    CalyxError::aster_corrupt_shard(format!(
                        "compaction CF directory entry {} has no name",
                        path.display()
                    ))
                })?;
            let cf = parse_cf_dir_name(&name)?;
            for sst in list_ssts(&path)? {
                shards.push(SstShard::new(cf, sst, 0)?);
            }
        }
    }
    shards.sort_by(|left, right| left.path.cmp(&right.path));
    shards.dedup_by(|left, right| left.path == right.path);
    Ok(CompactionCatalog::new(shards))
}

/// Lists SST files for compaction, failing closed on any `*.sst` name that
/// matches no canonical writer shape instead of compacting unknown bytes.
fn list_ssts(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir)
        .map_err(|error| CalyxError::disk_pressure(format!("read compaction CF dir: {error}")))?
    {
        let path = entry
            .map_err(|error| {
                CalyxError::disk_pressure(format!("read compaction SST entry: {error}"))
            })?
            .path();
        if classify_sst(&path)?.is_some() {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

fn tiered_cf_roots(root: &Path, tiering_policy: Option<&TieringPolicy>) -> Vec<PathBuf> {
    let mut roots = vec![root.join("cf")];
    if let Some(policy) = tiering_policy {
        for tier_root in policy.tier_roots() {
            let cf_root = tier_root.join("cf");
            if !roots.contains(&cf_root) {
                roots.push(cf_root);
            }
        }
    }
    roots
}
