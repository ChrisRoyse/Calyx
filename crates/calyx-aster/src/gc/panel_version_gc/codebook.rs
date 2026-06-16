use super::{
    PanelVersionGcResult, PanelVersionId, PanelVersionRecord, RetentionPolicy,
    VaultPanelVersionGcTarget, VersionTier, latest_versions_to_keep,
};
use calyx_core::{Clock, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub trait CodebookVersionGcTarget {
    fn codebook_versions(&self) -> Result<Vec<PanelVersionRecord>>;
    fn move_codebook_version_to_cold(&self, id: PanelVersionId) -> Result<u64>;
    fn purge_cold_codebook_version(&self, id: PanelVersionId) -> Result<u64>;
}

#[derive(Debug)]
pub struct CodebookVersionGc {
    pub retention_policy: RetentionPolicy,
    codebook_versions_pruned_total: AtomicU64,
}

impl CodebookVersionGc {
    pub fn new(retention_policy: RetentionPolicy) -> Self {
        Self {
            retention_policy,
            codebook_versions_pruned_total: AtomicU64::new(0),
        }
    }

    pub fn find_unreferenced<T>(&self, target: &T) -> Result<Vec<PanelVersionId>>
    where
        T: CodebookVersionGcTarget + ?Sized,
    {
        let records = target.codebook_versions()?;
        let keep = latest_versions_to_keep(&records, self.retention_policy.hot_versions_to_keep);
        let mut ids = records
            .into_iter()
            .filter(|record| !record.ledger_referenced)
            .filter(|record| !keep.contains(&record.id))
            .map(|record| record.id)
            .collect::<Vec<_>>();
        ids.sort_unstable();
        ids.dedup();
        Ok(ids)
    }

    pub fn prune<T>(&self, target: &T, ids: &[PanelVersionId]) -> Result<PanelVersionGcResult>
    where
        T: CodebookVersionGcTarget + ?Sized,
    {
        let records = target
            .codebook_versions()?
            .into_iter()
            .map(|record| (record.id, record))
            .collect::<BTreeMap<_, _>>();
        let mut result = PanelVersionGcResult::default();
        for id in ids.iter().take(self.retention_policy.max_versions_per_run) {
            let Some(record) = records.get(id) else {
                continue;
            };
            if record.ledger_referenced {
                result.skipped_ledger_referenced += 1;
                continue;
            }
            if self.retention_policy.cold_tier_first && record.tier == VersionTier::Hot {
                target.move_codebook_version_to_cold(*id)?;
                result.moved_to_cold += 1;
            } else {
                result.bytes_freed = result
                    .bytes_freed
                    .saturating_add(target.purge_cold_codebook_version(*id)?);
                result.pruned += 1;
            }
        }
        result.rate_limited = ids.len() > self.retention_policy.max_versions_per_run;
        result.codebook_versions_pruned_total = self
            .codebook_versions_pruned_total
            .fetch_add(result.pruned as u64, Ordering::Relaxed)
            + result.pruned as u64;
        Ok(result)
    }
}

impl Default for CodebookVersionGc {
    fn default() -> Self {
        Self::new(RetentionPolicy::default())
    }
}

impl<C> CodebookVersionGcTarget for VaultPanelVersionGcTarget<'_, C>
where
    C: Clock,
{
    fn codebook_versions(&self) -> Result<Vec<PanelVersionRecord>> {
        let mut records = Vec::new();
        collect_codebook_records(
            &self.hot_codebook_dir,
            VersionTier::Hot,
            &self.manifest_codebook_paths,
            &mut records,
        )?;
        collect_codebook_records(
            &self.cold_codebook_dir,
            VersionTier::Cold,
            &BTreeSet::new(),
            &mut records,
        )?;
        records.sort_by_key(|record| (record.id, record.tier == VersionTier::Cold));
        Ok(records)
    }

    fn move_codebook_version_to_cold(&self, id: PanelVersionId) -> Result<u64> {
        move_versioned_file(
            &self.hot_codebook_dir,
            &self.cold_codebook_dir,
            id,
            codebook_id_from_path,
        )
    }

    fn purge_cold_codebook_version(&self, id: PanelVersionId) -> Result<u64> {
        purge_versioned_file(&self.cold_codebook_dir, id, codebook_id_from_path)
    }
}

fn collect_codebook_records(
    dir: &Path,
    tier: VersionTier,
    manifest_codebook_paths: &BTreeSet<String>,
    out: &mut Vec<PanelVersionRecord>,
) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)
        .map_err(|error| calyx_core::CalyxError::disk_pressure(error.to_string()))?
    {
        let entry =
            entry.map_err(|error| calyx_core::CalyxError::disk_pressure(error.to_string()))?;
        let path = entry.path();
        let Some(id) = codebook_id_from_path(&path) else {
            continue;
        };
        let relative = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| format!("codebooks/{name}"));
        out.push(PanelVersionRecord {
            id,
            tier,
            ledger_referenced: relative
                .as_ref()
                .is_some_and(|path| manifest_codebook_paths.contains(path)),
            bytes: entry.metadata().map(|meta| meta.len()).unwrap_or(0),
        });
    }
    Ok(())
}

fn move_versioned_file(
    hot_dir: &Path,
    cold_dir: &Path,
    id: PanelVersionId,
    parse_id: fn(&Path) -> Option<PanelVersionId>,
) -> Result<u64> {
    let Some(path) = find_versioned_file(hot_dir, id, parse_id)? else {
        return Ok(0);
    };
    fs::create_dir_all(cold_dir)
        .map_err(|error| calyx_core::CalyxError::disk_pressure(error.to_string()))?;
    let bytes = path.metadata().map(|meta| meta.len()).unwrap_or(0);
    let target =
        cold_dir
            .join(path.file_name().ok_or_else(|| {
                calyx_core::CalyxError::disk_pressure("version path has no name")
            })?);
    fs::rename(&path, target)
        .map_err(|error| calyx_core::CalyxError::disk_pressure(error.to_string()))?;
    Ok(bytes)
}

fn purge_versioned_file(
    dir: &Path,
    id: PanelVersionId,
    parse_id: fn(&Path) -> Option<PanelVersionId>,
) -> Result<u64> {
    let Some(path) = find_versioned_file(dir, id, parse_id)? else {
        return Ok(0);
    };
    let bytes = path.metadata().map(|meta| meta.len()).unwrap_or(0);
    fs::remove_file(path)
        .map_err(|error| calyx_core::CalyxError::disk_pressure(error.to_string()))?;
    Ok(bytes)
}

fn find_versioned_file(
    dir: &Path,
    id: PanelVersionId,
    parse_id: fn(&Path) -> Option<PanelVersionId>,
) -> Result<Option<PathBuf>> {
    if !dir.exists() {
        return Ok(None);
    }
    for entry in fs::read_dir(dir)
        .map_err(|error| calyx_core::CalyxError::disk_pressure(error.to_string()))?
    {
        let path = entry
            .map_err(|error| calyx_core::CalyxError::disk_pressure(error.to_string()))?
            .path();
        if parse_id(&path) == Some(id) {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

fn codebook_id_from_path(path: &Path) -> Option<PanelVersionId> {
    let name = path.file_name()?.to_str()?;
    let rest = name.strip_prefix("codebook-v")?;
    rest.get(0..8)?.parse().ok()
}
