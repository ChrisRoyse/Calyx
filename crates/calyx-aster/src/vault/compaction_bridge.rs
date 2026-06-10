use super::AsterVault;
use crate::cf::ColumnFamily;
use crate::compaction::{
    CompactionCatalog, CompactionResult, CompactionScheduler, CompactionSchedulerOptions,
    CompactionThrottle, catalog_from_vault_tiers,
};
use crate::recurrence::{StoredRecurrenceRow, decode_recurrence_row};
use crate::sst::{SstReader, write_sst};
use calyx_core::{CalyxError, Clock, Result};
use std::fs;
use std::path::Path;
use std::sync::Arc;

#[derive(Debug)]
pub struct VaultCompactionScheduler {
    catalog: Arc<CompactionCatalog>,
    scheduler: CompactionScheduler,
}

impl VaultCompactionScheduler {
    pub fn shard_count_for_cf(&self, cf: ColumnFamily) -> usize {
        self.catalog.shard_count_for_cf(cf)
    }

    pub fn stop(self) -> std::thread::Result<()> {
        self.scheduler.stop()
    }
}

impl<C> AsterVault<C>
where
    C: Clock,
{
    pub fn compaction_catalog(&self) -> Result<Option<Arc<CompactionCatalog>>> {
        let Some(durable) = &self.durable else {
            return Ok(None);
        };
        durable.flush()?;
        self.rows.flush_all_cfs()?;
        Ok(Some(Arc::new(catalog_from_vault_tiers(
            durable.root(),
            durable.tiering_policy(),
        )?)))
    }

    pub fn compact_cf_once(&self, cf: ColumnFamily) -> Result<Option<CompactionResult>> {
        let Some(durable) = &self.durable else {
            return Ok(None);
        };
        durable.flush()?;
        self.rows.flush_all_cfs()?;
        let catalog = catalog_from_vault_tiers(durable.root(), durable.tiering_policy())?;
        let output = durable.compaction_output_path(cf, self.latest_seq());
        let mut result = catalog
            .compact_cf(cf, output, CompactionThrottle::unlimited())
            .map(Some)?;
        if let Some(CompactionResult::Compacted(report)) = &mut result
            && cf == ColumnFamily::Recurrence
        {
            report.reclaimed_input_files = reclaim_recurrence_inputs(report)?;
            prune_recurrence_tombstones(report)?;
        }
        Ok(result)
    }

    pub fn start_compaction_scheduler(
        &self,
        mut options: CompactionSchedulerOptions,
    ) -> Result<Option<VaultCompactionScheduler>> {
        if let Some(durable) = &self.durable
            && options.output_root == CompactionSchedulerOptions::default().output_root
        {
            options.output_root = durable.root().join("cf");
        }
        if let Some(durable) = &self.durable {
            options.tiering_policy = options
                .tiering_policy
                .or_else(|| durable.tiering_policy().cloned());
        }
        let Some(catalog) = self.compaction_catalog()? else {
            return Ok(None);
        };
        let scheduler = CompactionScheduler::start(catalog.clone(), options);
        Ok(Some(VaultCompactionScheduler { catalog, scheduler }))
    }
}

fn reclaim_recurrence_inputs(report: &crate::compaction::CompactionReport) -> Result<usize> {
    let output = fs::canonicalize(&report.output_path)
        .map_err(|error| CalyxError::disk_pressure(format!("stat compacted SST: {error}")))?;
    let parent = fs::canonicalize(&report.staging_parent)
        .map_err(|error| CalyxError::disk_pressure(format!("stat compaction parent: {error}")))?;
    let mut reclaimed = 0;
    for input in &report.input_paths {
        let input = match fs::canonicalize(input) {
            Ok(path) => path,
            Err(_) => continue,
        };
        if input == output {
            continue;
        }
        if input.parent() != Some(parent.as_path()) {
            continue;
        }
        if input.extension().and_then(|value| value.to_str()) != Some("sst") {
            continue;
        }
        fs::remove_file(&input).map_err(|error| {
            CalyxError::disk_pressure(format!(
                "reclaim recurrence compaction input {}: {error}",
                input.display()
            ))
        })?;
        reclaimed += 1;
    }
    Ok(reclaimed)
}

fn prune_recurrence_tombstones(report: &mut crate::compaction::CompactionReport) -> Result<()> {
    let mut retained = Vec::<(Vec<u8>, Vec<u8>)>::new();
    let mut pruned = 0_u64;
    for entry in SstReader::open(&report.output_path)?.iter()? {
        if matches!(
            decode_recurrence_row(&entry.value)?,
            StoredRecurrenceRow::Tombstone { .. }
        ) {
            pruned += 1;
            continue;
        }
        retained.push((entry.key, entry.value));
    }
    if pruned == 0 {
        return Ok(());
    }

    let seq = compacted_seq(&report.output_path)?;
    let reclaimed_path = report.staging_parent.join(format!("{seq:020}-9999.sst"));
    let entries = retained
        .iter()
        .map(|(key, value)| (key.as_slice(), value.as_slice()));
    let summary = write_sst(&reclaimed_path, entries)?;
    fs::remove_file(&report.output_path).map_err(|error| {
        CalyxError::disk_pressure(format!(
            "remove recurrence tombstone compaction file {}: {error}",
            report.output_path.display()
        ))
    })?;
    report.output_path = summary.path;
    report.output_bytes = summary.bytes;
    report.logical_bytes = retained.iter().map(|(_, value)| value.len() as u64).sum();
    report.write_amp_milli =
        report.output_bytes.saturating_mul(1_000) / report.logical_bytes.max(1);
    Ok(())
}

fn compacted_seq(path: &Path) -> Result<u64> {
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| CalyxError::aster_corrupt_shard("compacted recurrence SST has no stem"))?;
    let seq = stem.strip_prefix("compacted-").ok_or_else(|| {
        CalyxError::aster_corrupt_shard(format!("unexpected compacted SST name {stem}"))
    })?;
    seq.parse().map_err(|error| {
        CalyxError::aster_corrupt_shard(format!("parse compacted recurrence seq: {error}"))
    })
}
