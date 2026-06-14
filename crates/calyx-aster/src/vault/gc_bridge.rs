//! Vault-facing bridge for snapshot GC scheduler ticks.

use crate::cf::ColumnFamily;
use crate::compaction::{
    CompactionResult, CompactionThrottle, catalog_from_vault_tiers, compact_shards,
};
use crate::gc::{GcRateLimit, GcResult, SnapshotGcTick};
use crate::mvcc::Snapshot;
use crate::storage_names::{classify_sst, sst_order_key};
use crate::vault::AsterVault;
use calyx_core::{CalyxError, Clock, Result};
use std::fs;

impl<C> AsterVault<C>
where
    C: Clock,
{
    /// Runs one snapshot-pin watchdog tick.
    ///
    /// The background GC scheduler should call this at its 1 s cadence once the
    /// scheduler exists. Until then, resource-status and tests use the same
    /// underlying store hook to abort expired reader pins fail-closed.
    pub fn snapshot_gc_tick(&self, max_gap_seqs: u64) -> SnapshotGcTick {
        self.rows.snapshot_gc_tick(&self.clock, max_gap_seqs)
    }

    /// Runs one MVCC snapshot-version GC tick and physically compacts obsolete SSTs.
    pub fn snapshot_version_gc_once(&self, rate_limit: GcRateLimit) -> Result<GcResult> {
        self.rows.set_snapshot_gc_rate_limit(rate_limit);
        let mut result = self.rows.snapshot_version_gc_tick(&self.clock)?;
        if result.versions_reclaimed == 0 && result.rate_limited {
            return Ok(result);
        }
        let physical =
            self.reclaim_snapshot_ssts(result.safe_point_seq, rate_limit.max_ops_per_run)?;
        if physical.bytes_freed > 0 {
            self.rows
                .record_snapshot_gc_physical_bytes_freed(physical.bytes_freed);
            result.bytes_freed = result.bytes_freed.saturating_add(physical.bytes_freed);
        }
        result.rate_limited |= physical.rate_limited;
        Ok(result)
    }

    /// Runs snapshot GC using env-configured anti-storm limits.
    pub fn snapshot_version_gc_once_from_env(&self) -> Result<GcResult> {
        self.snapshot_version_gc_once(GcRateLimit::from_env()?)
    }

    /// Reads one CF row through an explicit tracked reader snapshot.
    pub fn read_pinned_cf(
        &self,
        snapshot: Snapshot,
        cf: ColumnFamily,
        key: &[u8],
    ) -> Result<Option<Vec<u8>>> {
        self.rows.read_at(snapshot, cf, key, &self.clock)
    }

    fn reclaim_snapshot_ssts(&self, safe_point: u64, max_input_files: usize) -> Result<GcResult> {
        let Some(durable) = &self.durable else {
            return Ok(GcResult {
                safe_point_seq: safe_point,
                ..GcResult::default()
            });
        };
        if max_input_files == 0 || safe_point == 0 {
            return Ok(GcResult {
                safe_point_seq: safe_point,
                rate_limited: max_input_files == 0,
                ..GcResult::default()
            });
        }

        durable.flush()?;
        self.rows.flush_all_cfs()?;
        let catalog = catalog_from_vault_tiers(durable.root(), durable.tiering_policy())?;
        let mut bytes_freed = 0usize;
        let mut files_seen = 0usize;
        for cf in catalog.column_families() {
            if cf == ColumnFamily::Ledger || files_seen >= max_input_files {
                continue;
            }
            let mut inputs = Vec::new();
            for shard in catalog.shards_for_cf(cf) {
                let Some(order) = sst_order_key(&shard.path)? else {
                    continue;
                };
                if order.seq < safe_point {
                    inputs.push(shard);
                    files_seen += 1;
                    if files_seen == max_input_files {
                        break;
                    }
                }
            }
            if inputs.len() < 2 {
                continue;
            }
            let output_seq = safe_point.saturating_sub(1);
            let output = durable.compaction_output_path(cf, output_seq);
            if let CompactionResult::Compacted(report) =
                compact_shards(cf, &inputs, output, CompactionThrottle::unlimited())?
            {
                let net = report.input_bytes.saturating_sub(report.output_bytes) as usize;
                reclaim_snapshot_inputs(&report)?;
                bytes_freed = bytes_freed.saturating_add(net);
            }
        }
        Ok(GcResult {
            safe_point_seq: safe_point,
            bytes_freed,
            rate_limited: files_seen >= max_input_files,
            ..GcResult::default()
        })
    }
}

fn reclaim_snapshot_inputs(report: &crate::compaction::CompactionReport) -> Result<usize> {
    let output = fs::canonicalize(&report.output_path)
        .map_err(|error| CalyxError::disk_pressure(format!("stat snapshot GC output: {error}")))?;
    let mut reclaimed = 0;
    for input in &report.input_paths {
        let input = match fs::canonicalize(input) {
            Ok(path) => path,
            Err(_) => continue,
        };
        if input == output {
            continue;
        }
        if classify_sst(&input)?.is_none() {
            continue;
        }
        fs::remove_file(&input).map_err(|error| {
            CalyxError::disk_pressure(format!(
                "reclaim snapshot GC input {}: {error}",
                input.display()
            ))
        })?;
        reclaimed += 1;
    }
    Ok(reclaimed)
}
