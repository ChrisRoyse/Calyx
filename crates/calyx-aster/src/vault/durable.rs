mod manifest_ops;
mod recovery_readback;

use recovery_readback::read_manifested_batches;

use super::encode::{WriteRow, decode_write_batch, encode_write_batch};
use crate::cf::ColumnFamily;
use crate::compaction::TieringPolicy;
use crate::dedup::DedupPolicy;
use crate::manifest::recover_vault;
use crate::pressure::DiskPressureGuard;
use crate::resource::ResourceCounters;
use crate::sst::write_sst;
use crate::timetravel::RetentionHorizon;
use crate::wal::{GroupCommitBatcher, WalOptions, replay_dir};
use calyx_core::{CalyxError, Panel, Result, SystemClock, TemporalPolicy};
use calyx_ledger::CheckpointConfig;
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::atomic::AtomicBool;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
pub struct VaultOptions {
    pub wal_options: WalOptions,
    pub memtable_byte_cap: usize,
    pub tiering_policy: Option<TieringPolicy>,
    pub ledger_checkpoint: Option<CheckpointConfig>,
    pub temporal_policy: Option<TemporalPolicy>,
    pub dedup_policy: Option<DedupPolicy>,
    pub retention_horizon: RetentionHorizon,
    pub panel: Option<Panel>,
    /// Optional data-residency pin (PRD `30 §4`). When set, the vault's storage
    /// location is pinned and off-dataset writes/copies fail closed.
    pub residency: Option<crate::residency::Residency>,
    pub disk_pressure_guard: Option<DiskPressureGuard>,
    /// Restores checkpointed durable SST rows into the in-memory MVCC table on
    /// open. Disable only for latest-read workloads that can use the CF router
    /// as the checkpointed source of truth and do not request historical reads.
    pub restore_mvcc_rows: bool,
    /// Restores the full in-memory ledger hook on open. Disable only for
    /// explicitly read-only handles that verify/search latest state without
    /// appending ledger entries.
    pub restore_ledger_hook: bool,
    /// Opens the vault as a read-only handle. Any write through this handle
    /// fails before WAL append or MVCC mutation.
    pub read_only: bool,
    /// Restricts router recovery to a concrete CF set for read-only handles.
    /// This keeps analytical/search reads from enumerating unrelated large CFs.
    pub selected_cfs: Option<Vec<ColumnFamily>>,
}

impl Default for VaultOptions {
    fn default() -> Self {
        Self {
            wal_options: WalOptions::default(),
            memtable_byte_cap: 0,
            tiering_policy: None,
            ledger_checkpoint: Some(CheckpointConfig::default()),
            temporal_policy: Some(TemporalPolicy::default()),
            dedup_policy: Some(DedupPolicy::default()),
            retention_horizon: RetentionHorizon::default(),
            panel: None,
            residency: None,
            disk_pressure_guard: None,
            restore_mvcc_rows: true,
            restore_ledger_hook: true,
            read_only: false,
            selected_cfs: None,
        }
    }
}

#[derive(Debug)]
pub(super) struct DurableVault {
    root: PathBuf,
    batcher: GroupCommitBatcher,
    tiering_policy: Option<TieringPolicy>,
    ledger_checkpoint: Option<CheckpointConfig>,
    temporal_policy: Option<TemporalPolicy>,
    dedup_policy: Option<DedupPolicy>,
    retention_horizon: Mutex<RetentionHorizon>,
    panel: Option<Panel>,
    disk_pressure_guard: Option<DiskPressureGuard>,
    pending_checkpoint: Mutex<Vec<(u64, Vec<WriteRow>)>>,
    /// Max checkpointed seq whose batch wrote derived-search-content CF rows
    /// (issue #1100); persisted into every manifest write as
    /// `derived_content_seq`, clamped to that manifest's `durable_seq`.
    checkpointed_derived_content_seq: AtomicU64,
    #[cfg(test)]
    fail_next_wal_append: Arc<AtomicBool>,
}

pub(super) struct RecoveredBatch {
    pub seq: u64,
    pub rows: Vec<WriteRow>,
}

pub(super) struct RecoveredBatches {
    pub batches: Vec<RecoveredBatch>,
    pub last_recovered_seq: u64,
    pub wal_replay_floor_seq: u64,
    /// Durably recorded derived-content watermark floor for seqs at or below
    /// `wal_replay_floor_seq`; WAL replay re-derives the rest per batch.
    pub derived_content_floor_seq: u64,
    pub torn_tail: Option<crate::wal::TornTail>,
    pub temporal_policy: Option<TemporalPolicy>,
    pub dedup_policy: Option<DedupPolicy>,
    pub retention_horizon: RetentionHorizon,
    pub router_latest_readback: bool,
}

impl DurableVault {
    pub(super) fn validate_options(options: &VaultOptions) -> Result<()> {
        if let Some(policy) = &options.temporal_policy {
            policy.validate()?;
        }
        if let Some(policy) = &options.dedup_policy {
            validate_dedup_policy(policy, options.panel.as_ref())?;
        }
        options.retention_horizon.validate()?;
        if !options.restore_ledger_hook && !options.read_only {
            return Err(CalyxError {
                code: "CALYX_VAULT_OPTIONS_INVALID",
                message:
                    "restore_ledger_hook=false requires read_only=true to prevent unverified writes"
                        .to_string(),
                remediation: "open read workloads with read_only=true, or keep restore_ledger_hook=true for write-capable handles",
            });
        }
        if options.read_only && options.residency.is_some() {
            return Err(CalyxError {
                code: "CALYX_VAULT_OPTIONS_INVALID",
                message: "read_only=true cannot persist a new residency pin".to_string(),
                remediation: "persist residency with a write-capable open before opening read-only handles",
            });
        }
        if options.selected_cfs.is_some() && !options.read_only {
            return Err(CalyxError {
                code: "CALYX_VAULT_OPTIONS_INVALID",
                message: "selected_cfs requires read_only=true to prevent partial write handles"
                    .to_string(),
                remediation: "open full write-capable vault handles without selected_cfs, or mark the handle read_only=true",
            });
        }
        if options.selected_cfs.as_ref().is_some_and(Vec::is_empty) {
            return Err(CalyxError {
                code: "CALYX_VAULT_OPTIONS_INVALID",
                message: "selected_cfs cannot be empty".to_string(),
                remediation: "omit selected_cfs or include every CF required by the read workload",
            });
        }
        if options.selected_cfs.is_some() && options.tiering_policy.is_some() {
            return Err(CalyxError {
                code: "CALYX_VAULT_OPTIONS_INVALID",
                message: "selected_cfs with tiering_policy is not implemented".to_string(),
                remediation: "open a full read-only tiered handle or add selected tier-aware CF routing first",
            });
        }
        Ok(())
    }

    pub(super) fn open_after(
        root: impl AsRef<Path>,
        options: &VaultOptions,
        wal_replay_floor_seq: u64,
    ) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        Self::validate_options(options)?;
        fs::create_dir_all(root.join("cf"))
            .map_err(|error| storage_error("create durable CF root", error))?;
        if let Some(policy) = &options.tiering_policy {
            for tier_root in policy.tier_roots() {
                fs::create_dir_all(tier_root.join("cf"))
                    .map_err(|error| storage_error("create tiered durable CF root", error))?;
            }
        }
        let wal = crate::wal::Wal::open_after(
            root.join("wal"),
            options.wal_options,
            wal_replay_floor_seq,
        )?;
        let batcher = GroupCommitBatcher::new(
            wal,
            options.wal_options.group_commit_window,
            Arc::new(SystemClock),
        )?;
        let durable = Self {
            root,
            batcher,
            tiering_policy: options.tiering_policy.clone(),
            ledger_checkpoint: options.ledger_checkpoint.clone(),
            temporal_policy: options.temporal_policy,
            dedup_policy: options.dedup_policy.clone(),
            retention_horizon: Mutex::new(options.retention_horizon.clone()),
            panel: options.panel.clone(),
            disk_pressure_guard: options.disk_pressure_guard.clone(),
            pending_checkpoint: Mutex::new(Vec::new()),
            checkpointed_derived_content_seq: AtomicU64::new(0),
            #[cfg(test)]
            fail_next_wal_append: Arc::new(AtomicBool::new(false)),
        };
        if durable.root.join("CURRENT").exists() {
            let manifest = crate::manifest::ManifestStore::open(&durable.root).load_current()?;
            durable
                .checkpointed_derived_content_seq
                .store(manifest.effective_derived_content_seq(), Ordering::Release);
        }
        if durable.panel.is_some() && !durable.root.join("CURRENT").exists() {
            durable.write_manifest_with_seq(1, 0)?;
        }
        Ok(durable)
    }

    pub(super) fn recover_batches(
        root: impl AsRef<Path>,
        options: &VaultOptions,
    ) -> Result<RecoveredBatches> {
        Self::validate_options(options)?;
        let root = root.as_ref();
        if root.join("CURRENT").exists() {
            let recovery = recover_vault(root)?;
            if let Some(policy) = &recovery.manifest.dedup_policy {
                validate_dedup_policy(policy, options.panel.as_ref())?;
            }
            let router_latest_readback = !options.restore_mvcc_rows;
            let mut batches = if options.restore_mvcc_rows {
                read_manifested_batches(
                    root,
                    options.tiering_policy.as_ref(),
                    recovery.manifest.durable_seq,
                )?
            } else {
                Vec::new()
            };
            for record in recovery.wal_records {
                batches.push(RecoveredBatch {
                    seq: record.seq,
                    rows: decode_write_batch(&record.payload)?,
                });
            }
            return Ok(RecoveredBatches {
                batches,
                last_recovered_seq: recovery.last_recovered_seq,
                wal_replay_floor_seq: recovery.manifest.durable_seq,
                derived_content_floor_seq: recovery.manifest.effective_derived_content_seq(),
                torn_tail: recovery.torn_tail,
                temporal_policy: recovery.manifest.temporal_policy,
                dedup_policy: recovery.manifest.dedup_policy,
                retention_horizon: recovery.manifest.retention_horizon,
                router_latest_readback,
            });
        }

        let replay = replay_dir(root.join("wal"))?;
        let last_recovered_seq = replay.records.last().map_or(0, |record| record.seq);
        let batches = replay
            .records
            .iter()
            .map(|record| {
                Ok(RecoveredBatch {
                    seq: record.seq,
                    rows: decode_write_batch(&record.payload)?,
                })
            })
            .collect::<Result<_>>()?;
        Ok(RecoveredBatches {
            batches,
            last_recovered_seq,
            wal_replay_floor_seq: 0,
            derived_content_floor_seq: 0,
            torn_tail: replay.torn_tail,
            temporal_policy: options.temporal_policy,
            dedup_policy: options.dedup_policy.clone(),
            retention_horizon: options.retention_horizon.clone(),
            router_latest_readback: false,
        })
    }

    pub(super) fn append_batch(&self, rows: &[WriteRow]) -> Result<u64> {
        #[cfg(test)]
        if self.fail_next_wal_append.swap(false, Ordering::SeqCst) {
            return Err(CalyxError::disk_pressure("injected WAL append failure"));
        }
        let payload = encode_write_batch(rows)?;
        let ack = self.batcher.submit(payload)?;
        Ok(ack.seq)
    }

    pub(super) fn ensure_disk_write_allowed(&self, counters: &ResourceCounters) -> Result<()> {
        let Some(guard) = &self.disk_pressure_guard else {
            return Ok(());
        };
        match guard.check() {
            Ok(_) => Ok(()),
            Err(error) if error.code == "CALYX_DISK_PRESSURE" => {
                counters.record_disk_pressure();
                guard.request_spill();
                Err(error)
            }
            Err(error) => Err(error),
        }
    }

    pub(super) fn durable_tip_seq(&self) -> Result<u64> {
        self.batcher.tip_seq()
    }

    #[cfg(test)]
    pub(super) fn fail_next_wal_append(&self) {
        self.fail_next_wal_append.store(true, Ordering::SeqCst);
    }

    pub(super) fn checkpoint_batch(&self, seq: u64, rows: &[WriteRow]) -> Result<()> {
        self.write_rows(seq, rows)?;
        self.advance_checkpointed_derived_content(seq, rows);
        self.write_manifest(seq)
    }

    fn advance_checkpointed_derived_content(&self, seq: u64, rows: &[WriteRow]) {
        if rows.iter().any(|row| row.cf.feeds_derived_search_content()) {
            self.checkpointed_derived_content_seq
                .fetch_max(seq, Ordering::AcqRel);
        }
    }

    /// Watermark value a manifest written at `durable_seq` may vouch for.
    pub(super) fn derived_content_seq_for_manifest(&self, durable_seq: u64) -> u64 {
        self.checkpointed_derived_content_seq
            .load(Ordering::Acquire)
            .min(durable_seq)
    }

    /// Adopts a foreign writer's checkpointed watermark (picked up from the
    /// on-disk manifest under the commit lock) so later manifest writes from
    /// this handle vouch for content it did not checkpoint itself (#1100).
    pub(in crate::vault) fn advance_derived_content_watermark_to_at_least(&self, seq: u64) {
        self.checkpointed_derived_content_seq
            .fetch_max(seq, Ordering::AcqRel);
    }

    pub(super) fn stage_checkpoint_batch(&self, seq: u64, rows: &[WriteRow]) -> Result<()> {
        self.pending_checkpoint
            .lock()
            .map_err(|_| CalyxError::disk_pressure("checkpoint staging lock poisoned"))?
            .push((seq, rows.to_vec()));
        Ok(())
    }

    pub(super) fn flush(&self) -> Result<()> {
        self.batcher.flush_sync()?;
        self.flush_pending_checkpoints()
    }

    pub(super) fn root(&self) -> &Path {
        &self.root
    }

    pub(super) fn recurrence_lock_path(&self) -> PathBuf {
        self.root.join("locks").join("recurrence.write.lock")
    }

    pub(super) fn commit_lock_path(&self) -> PathBuf {
        self.root.join("locks").join("durable.commit.lock")
    }

    pub(super) fn recover_current_batches(&self) -> Result<RecoveredBatches> {
        let options = VaultOptions {
            tiering_policy: self.tiering_policy.clone(),
            ledger_checkpoint: self.ledger_checkpoint.clone(),
            temporal_policy: self.temporal_policy,
            dedup_policy: self.dedup_policy.clone(),
            retention_horizon: self.retention_horizon(),
            panel: self.panel.clone(),
            disk_pressure_guard: self.disk_pressure_guard.clone(),
            restore_mvcc_rows: true,
            ..VaultOptions::default()
        };
        Self::recover_batches(&self.root, &options)
    }

    pub(super) fn ledger_checkpoint(&self) -> Option<CheckpointConfig> {
        self.ledger_checkpoint.clone()
    }

    pub(super) fn tiering_policy(&self) -> Option<&TieringPolicy> {
        self.tiering_policy.as_ref()
    }

    pub(super) fn compaction_output_path(&self, cf: ColumnFamily, seq: u64) -> PathBuf {
        self.cf_dir(cf).join(format!("compacted-{seq:020}.sst"))
    }

    fn write_rows(&self, seq: u64, rows: &[WriteRow]) -> Result<()> {
        let mut by_cf = Vec::<(ColumnFamily, Vec<(usize, &WriteRow)>)>::new();
        for (index, row) in rows.iter().enumerate() {
            if let Some((_, group)) = by_cf.iter_mut().find(|(cf, _)| *cf == row.cf) {
                group.push((index, row));
            } else {
                by_cf.push((row.cf, vec![(index, row)]));
            }
        }
        by_cf.sort_by_key(|(cf, _)| cf.name());
        for (cf, rows) in by_cf {
            let rows = latest_rows_by_key(rows);
            let first_index = rows.first().map_or(0, |(index, _)| *index);
            let dir = self.cf_dir(cf);
            fs::create_dir_all(&dir).map_err(|error| storage_error("create CF dir", error))?;
            let path = dir.join(format!("{seq:020}-{first_index:04}.sst"));
            let entries = rows
                .iter()
                .map(|(_, row)| (row.key.as_slice(), row.value.as_slice()));
            write_sst(&path, entries)?;
        }
        Ok(())
    }

    fn flush_pending_checkpoints(&self) -> Result<()> {
        let batches = self
            .pending_checkpoint
            .lock()
            .map_err(|_| CalyxError::disk_pressure("checkpoint staging lock poisoned"))?
            .clone();
        if batches.is_empty() {
            return Ok(());
        }
        for (seq, rows) in &batches {
            self.write_rows(*seq, rows)?;
            self.advance_checkpointed_derived_content(*seq, rows);
        }
        let last_seq = batches.last().map_or(0, |(seq, _)| *seq);
        self.write_manifest(last_seq)?;
        let mut pending = self
            .pending_checkpoint
            .lock()
            .map_err(|_| CalyxError::disk_pressure("checkpoint staging lock poisoned"))?;
        pending.retain(|(seq, _)| *seq > last_seq);
        Ok(())
    }

    fn cf_dir(&self, cf: ColumnFamily) -> PathBuf {
        self.tiering_policy.as_ref().map_or_else(
            || self.root.join("cf").join(cf.name()),
            |policy| policy.place_current_cf(cf).absolute_dir(),
        )
    }
}

fn latest_rows_by_key<'a>(rows: Vec<(usize, &'a WriteRow)>) -> Vec<(usize, &'a WriteRow)> {
    let mut latest = BTreeMap::<Vec<u8>, (usize, &'a WriteRow)>::new();
    for (index, row) in rows {
        latest.insert(row.key.clone(), (index, row));
    }
    latest.into_values().collect()
}

fn validate_dedup_policy(policy: &DedupPolicy, panel: Option<&Panel>) -> Result<()> {
    if let Some(panel) = panel {
        policy.validate(panel)
    } else {
        policy.validate_manifest()
    }
}

fn storage_error(context: &str, error: io::Error) -> CalyxError {
    CalyxError::disk_pressure(format!("{context}: {error}"))
}
