mod checkpointing;
mod manifest_ops;
pub(in crate::vault) mod recovery_readback;
pub(in crate::vault) mod router_coverage;

use recovery_readback::read_manifested_batches;

use super::encode::{WriteRow, decode_write_batch, encode_write_batch};
use crate::cf::ColumnFamily;
use crate::compaction::TieringPolicy;
use crate::dedup::DedupPolicy;
use crate::manifest::{recover_vault, recover_vault_metadata};
use crate::pressure::DiskPressureGuard;
use crate::resource::ResourceCounters;
use crate::security::value_crypto::{SharedVaultContext, open_rows, seal_rows};
use crate::timetravel::RetentionHorizon;
use crate::wal::{GroupCommitBatcher, ReplayRecord, WalOptions, replay_dir};
use calyx_core::{CalyxError, Panel, Result, SystemClock, TemporalPolicy};
use calyx_ledger::CheckpointConfig;
use std::io;
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::atomic::AtomicBool;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Optional durable-recovery progress hook: `(bytes_replayed, bytes_total)`.
///
/// The vault invokes this only as WAL replay genuinely advances, so startup
/// owners can surface real intra-`vault_open` liveness instead of timer
/// heartbeats that would mask a true recovery deadlock.
#[derive(Clone)]
pub struct RecoveryProgressHook(Arc<dyn Fn(u64, u64) + Send + Sync>);

impl RecoveryProgressHook {
    pub fn new(hook: impl Fn(u64, u64) + Send + Sync + 'static) -> Self {
        Self(Arc::new(hook))
    }

    fn report(&self, bytes_replayed: u64, bytes_total: u64) {
        (self.0)(bytes_replayed, bytes_total);
    }
}

impl std::fmt::Debug for RecoveryProgressHook {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RecoveryProgressHook")
            .finish_non_exhaustive()
    }
}

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
    /// Optional authenticated value encryption context for durable bytes.
    pub value_crypto: Option<SharedVaultContext>,
    /// Restores checkpointed durable-batch/compacted SST rows plus the WAL
    /// tail into the in-memory MVCC table on open. Router memtable-flush SSTs
    /// carry flush ordinals, not commit seqs, so they are never restored; the
    /// open fails closed with `CALYX_ASTER_ROUTER_ONLY_ROWS` if any router
    /// row lacks a commit-domain durable home (issue #1132). Disable for
    /// latest-read workloads that use the CF router as the source of truth
    /// and do not request historical reads.
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
    /// Optional hook fired as WAL replay advances during open. Default `None`
    /// = zero behavior change for callers that do not observe recovery
    /// progress.
    pub recovery_progress: Option<RecoveryProgressHook>,
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
            value_crypto: None,
            restore_mvcc_rows: true,
            restore_ledger_hook: true,
            read_only: false,
            selected_cfs: None,
            recovery_progress: None,
        }
    }
}

impl VaultOptions {
    /// Write-capable open profile that serves reads from the mmap-backed CF
    /// router (latest committed state) instead of materializing every MVCC row
    /// of every column family into the heap on open.
    ///
    /// This is the additive, backward-compatible substrate for latest-state
    /// read+write commands — weave-loom, kernel-build, kernel-answer — that only
    /// need the latest committed state plus write/ledger-append capability, not
    /// historical/time-travel MVCC snapshots. It is exactly the default profile
    /// with `restore_mvcc_rows: false`:
    ///
    /// * `restore_mvcc_rows: false` — open takes the `router_latest_readback`
    ///   path (see [`super::open`]), so durable SST rows stay demand-paged
    ///   through `MmapColumn` rather than heap-copied into the in-memory
    ///   [`crate::mvcc::store::VersionedCfStore`]. The working set becomes
    ///   proportional to the pages the command actually touches, not the whole
    ///   physical vault. Physical readback is preserved: latest reads
    ///   (`read_cf_at`, `scan_cf_at`, snapshots at `latest_seq`) resolve through
    ///   the router over memtable + SST + WAL-tail.
    /// * `restore_ledger_hook: true`, `read_only: false` — the handle stays
    ///   write-capable, so `write_cf_batch`, `flush`, and `append_ledger_entry`
    ///   (with verified group-commit) all function and read-after-write on the
    ///   same handle sees the just-written rows.
    /// * `selected_cfs: None` — required, because [`Self::validate_options`]
    ///   fails closed on `selected_cfs` for a non-`read_only` handle. CF
    ///   narrowing remains a read-only-handle facility.
    ///
    /// Contract preserved: the open still fails closed with
    /// `CALYX_ASTER_ROUTER_ONLY_ROWS` semantics are *not weakened* — the
    /// router-only-rows full-restore coverage check only runs for full-restore
    /// opens (it exists to catch rows visible to a restored MVCC snapshot but
    /// not the router); latest-readback reads straight from the router, so the
    /// check does not apply.
    pub fn write_latest_readback() -> Self {
        Self {
            restore_mvcc_rows: false,
            ..Self::default()
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
    value_crypto: Option<SharedVaultContext>,
    pending_checkpoint: Mutex<Vec<(u64, Vec<WriteRow>)>>,
    /// Max checkpointed seq whose batch wrote a persistent-search-input CF
    /// (issues #1100 and #1808); persisted into every manifest write as
    /// `derived_content_seq`, clamped to that manifest's `durable_seq`.
    checkpointed_derived_content_seq: AtomicU64,
    #[cfg(test)]
    fail_next_wal_append: Arc<AtomicBool>,
    #[cfg(test)]
    fail_next_mvcc_commit: Arc<AtomicBool>,
    #[cfg(test)]
    fail_next_mvcc_restore: Arc<AtomicBool>,
    #[cfg(test)]
    fail_next_checkpoint: Arc<AtomicBool>,
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
    /// The pointed manifest predates the exact persistent-search-input model;
    /// open must re-derive and prove its floor from validated relevant levels.
    pub migrate_derived_content_model: bool,
    pub torn_tail: Option<crate::wal::TornTail>,
    pub temporal_policy: Option<TemporalPolicy>,
    pub dedup_policy: Option<DedupPolicy>,
    pub retention_horizon: RetentionHorizon,
    pub router_latest_readback: bool,
    /// When `Some(floor)`, the post-floor WAL tail was NOT materialized into
    /// `batches`; the open path must stream-restore it from this floor in
    /// bounded chunks (issue #1992). `None` keeps the collecting behavior.
    pub wal_tail_stream_floor: Option<u64>,
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
        Ok(())
    }

    pub(super) fn open_after(
        root: impl AsRef<Path>,
        options: &VaultOptions,
        wal_replay_floor_seq: u64,
        derived_content_floor_seq: u64,
    ) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        Self::validate_options(options)?;
        crate::private_fs::create_private_dir_all(&root.join("cf"))?;
        if let Some(policy) = &options.tiering_policy {
            for tier_root in policy.tier_roots() {
                crate::private_fs::create_private_dir_all(&tier_root.join("cf"))?;
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
            value_crypto: options.value_crypto.clone(),
            pending_checkpoint: Mutex::new(Vec::new()),
            checkpointed_derived_content_seq: AtomicU64::new(0),
            #[cfg(test)]
            fail_next_wal_append: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            fail_next_mvcc_commit: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            fail_next_mvcc_restore: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            fail_next_checkpoint: Arc::new(AtomicBool::new(false)),
        };
        durable
            .checkpointed_derived_content_seq
            .store(derived_content_floor_seq, Ordering::Release);
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
            // Streaming-restore fast path (issue #1992): a write-capable
            // latest-readback open (wal-prune / weave / kernel) with no value
            // crypto does NOT materialize the post-floor WAL tail here. Recover
            // manifest + tip header-only and let the open path stream-restore
            // the tail in bounded chunks, so a multi-GB orphaned tail never
            // lands in the heap at open. Crypto and full-restore opens keep the
            // collecting path below (they need decrypted/whole-state batches).
            if !options.restore_mvcc_rows && !options.read_only && options.value_crypto.is_none() {
                let recovery = recover_vault_metadata(root)?;
                if let Some(policy) = &recovery.manifest.dedup_policy {
                    validate_dedup_policy(policy, options.panel.as_ref())?;
                }
                let migrate_derived_content_model =
                    !recovery.manifest.uses_persistent_search_content_model();
                return Ok(RecoveredBatches {
                    batches: Vec::new(),
                    last_recovered_seq: recovery.last_recovered_seq,
                    wal_replay_floor_seq: recovery.manifest.durable_seq,
                    derived_content_floor_seq: if migrate_derived_content_model {
                        0
                    } else {
                        recovery.manifest.effective_derived_content_seq()
                    },
                    migrate_derived_content_model,
                    torn_tail: recovery.torn_tail,
                    temporal_policy: recovery.manifest.temporal_policy,
                    dedup_policy: recovery.manifest.dedup_policy,
                    retention_horizon: recovery.manifest.retention_horizon,
                    router_latest_readback: true,
                    wal_tail_stream_floor: Some(recovery.manifest.durable_seq),
                });
            }
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
            for batch in &mut batches {
                batch.rows = open_rows(
                    options.value_crypto.as_ref(),
                    std::mem::take(&mut batch.rows),
                )?;
            }
            replay_records_into(
                &mut batches,
                recovery.wal_records,
                options.value_crypto.as_ref(),
                options.recovery_progress.as_ref(),
            )?;
            let migrate_derived_content_model =
                !recovery.manifest.uses_persistent_search_content_model();
            return Ok(RecoveredBatches {
                batches,
                last_recovered_seq: recovery.last_recovered_seq,
                wal_replay_floor_seq: recovery.manifest.durable_seq,
                derived_content_floor_seq: if migrate_derived_content_model {
                    0
                } else {
                    recovery.manifest.effective_derived_content_seq()
                },
                migrate_derived_content_model,
                torn_tail: recovery.torn_tail,
                temporal_policy: recovery.manifest.temporal_policy,
                dedup_policy: recovery.manifest.dedup_policy,
                retention_horizon: recovery.manifest.retention_horizon,
                router_latest_readback,
                wal_tail_stream_floor: None,
            });
        }

        let replay = replay_dir(root.join("wal"))?;
        let last_recovered_seq = replay.records.last().map_or(0, |record| record.seq);
        let mut batches = Vec::new();
        replay_records_into(
            &mut batches,
            replay.records,
            options.value_crypto.as_ref(),
            options.recovery_progress.as_ref(),
        )?;
        Ok(RecoveredBatches {
            batches,
            last_recovered_seq,
            wal_replay_floor_seq: 0,
            derived_content_floor_seq: 0,
            migrate_derived_content_model: false,
            torn_tail: replay.torn_tail,
            temporal_policy: options.temporal_policy,
            dedup_policy: options.dedup_policy.clone(),
            retention_horizon: options.retention_horizon.clone(),
            router_latest_readback: false,
            wal_tail_stream_floor: None,
        })
    }

    pub(super) fn append_batch(&self, rows: &[WriteRow]) -> Result<u64> {
        #[cfg(test)]
        if self.fail_next_wal_append.swap(false, Ordering::SeqCst) {
            return Err(CalyxError::disk_pressure("injected WAL append failure"));
        }
        let sealed_rows = seal_rows(self.value_crypto.as_ref(), rows)?;
        let payload = encode_write_batch(&sealed_rows)?;
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

    #[cfg(test)]
    pub(super) fn fail_next_mvcc_commit(&self) {
        self.fail_next_mvcc_commit.store(true, Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(super) fn take_mvcc_commit_failure(&self) -> bool {
        self.fail_next_mvcc_commit.swap(false, Ordering::SeqCst)
    }

    #[cfg(test)]
    pub(super) fn fail_next_mvcc_restore(&self) {
        self.fail_next_mvcc_restore.store(true, Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(super) fn take_mvcc_restore_failure(&self) -> bool {
        self.fail_next_mvcc_restore.swap(false, Ordering::SeqCst)
    }

    #[cfg(test)]
    pub(super) fn fail_next_checkpoint(&self) {
        self.fail_next_checkpoint.store(true, Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(super) fn take_checkpoint_failure(&self) -> bool {
        self.fail_next_checkpoint.swap(false, Ordering::SeqCst)
    }

    fn advance_checkpointed_derived_content(&self, seq: u64, rows: &[WriteRow]) {
        if rows
            .iter()
            .any(|row| row.cf.feeds_persistent_search_index())
        {
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

    pub(super) fn flush(&self) -> Result<()> {
        self.batcher.flush_sync()?;
        self.flush_pending_checkpoints()
    }

    pub(super) fn root(&self) -> &Path {
        &self.root
    }

    pub(super) fn value_crypto_enabled(&self) -> bool {
        self.value_crypto.is_some()
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
            value_crypto: self.value_crypto.clone(),
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

    fn cf_dir(&self, cf: ColumnFamily) -> PathBuf {
        self.tiering_policy.as_ref().map_or_else(
            || self.root.join("cf").join(cf.name()),
            |policy| policy.place_current_cf(cf).absolute_dir(),
        )
    }
}

/// WAL payload bytes replayed between recovery-progress callbacks (~4 MiB).
const RECOVERY_PROGRESS_STRIDE_BYTES: u64 = 4 * 1024 * 1024;

/// Decodes replayed WAL records into `batches`, firing `progress` as real
/// bytes are consumed: once at `(0, total)`, every ~4 MiB, and once at
/// `(total, total)`. The hook is invoked only when replay actually advances,
/// so a genuine deadlock stays observable as a frozen trace.
fn replay_records_into(
    batches: &mut Vec<RecoveredBatch>,
    records: Vec<ReplayRecord>,
    value_crypto: Option<&SharedVaultContext>,
    progress: Option<&RecoveryProgressHook>,
) -> Result<()> {
    let total_bytes: u64 = records
        .iter()
        .map(|record| record.payload.len() as u64)
        .sum();
    if let Some(hook) = progress {
        hook.report(0, total_bytes);
    }
    let mut replayed_bytes: u64 = 0;
    let mut last_reported: u64 = 0;
    for record in records {
        let payload_bytes = record.payload.len() as u64;
        batches.push(RecoveredBatch {
            seq: record.seq,
            rows: open_rows(value_crypto, decode_write_batch(&record.payload)?)?,
        });
        replayed_bytes += payload_bytes;
        if let Some(hook) = progress
            && replayed_bytes.saturating_sub(last_reported) >= RECOVERY_PROGRESS_STRIDE_BYTES
        {
            hook.report(replayed_bytes, total_bytes);
            last_reported = replayed_bytes;
        }
    }
    if let Some(hook) = progress
        && replayed_bytes != last_reported
    {
        hook.report(replayed_bytes, total_bytes);
    }
    Ok(())
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
