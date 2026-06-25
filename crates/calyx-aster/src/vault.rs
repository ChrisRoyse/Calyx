//! Aster `VaultStore` implementation over the PH08 MVCC CF table.

mod anchor_codec;
mod anchor_merge;
mod batch_ingest;
mod cf_codec;
mod commit;
mod compaction_bridge;
pub mod context;
mod cursor;
mod dedup_commit;
mod durable;
pub mod encode;
mod gc_bridge;
pub mod grant;
mod htap;
mod key;
pub mod keyspace;
mod layer_commit;
mod ledger_append;
mod ledger_hook;
pub mod ledger_stub;
pub mod quota;
mod retention_horizon;
mod router_bridge;
mod seq_readback;
mod slot_backfill;
mod slot_column;
mod store;
mod temporal_xterm;

#[cfg(test)]
use crate::cf::ledger_key;
use crate::cf::{CfRouter, ColumnFamily, KeyRange, anchor_key, base_key, slot_key};
use crate::dedup::DedupPolicy;
use crate::mvcc::{Freshness, ReadBarrier, ReaderLease, Snapshot, VersionedCfStore};
use crate::resource::{ResourceStatus, VramBudgetStatus, collect_resource_status};
use crate::timetravel::RetentionHorizon;
use crate::vault::durable::DurableVault;
use crate::vault::ledger_hook::AsterLedgerHook;
use crate::wal::TornTail;
#[cfg(test)]
use calyx_core::{Anchor, SlotId, VaultStore};
use calyx_core::{CalyxError, Clock, Constellation, CxId, Result, Seq, SystemClock, VaultId};
use std::path::Path;
use std::sync::Mutex;

pub use compaction_bridge::VaultCompactionScheduler;
pub use context::VaultContext;
pub use durable::VaultOptions;
pub use grant::{AuditEvent, GrantEntry, GrantStore};
pub use htap::HtapDualRead;
pub use key::{CALYX_DECRYPTION_FAILED, CALYX_ENCRYPTION_FAILED, CALYX_VAULT_KEY_MISSING};
pub use keyspace::{
    CALYX_VAULT_KEYSPACE_MISMATCH, KeyspaceGuard, VaultWriteLock, VaultWriteLockGuard, vault_prefix,
};
pub use quota::{CALYX_QUOTA_EXCEEDED, QuotaConfig, QuotaGuard};
pub use slot_column::{
    SlotColumnManifest, SlotColumnMaterialization, SlotColumnReadback, SlotColumnRow,
    read_materialized_slot_column,
};

const DEFAULT_LEASE_MS: u64 = 5_000;

/// Single-vault Aster store with content-addressed ingest semantics.
#[derive(Debug)]
pub struct AsterVault<C = SystemClock> {
    vault_id: VaultId,
    vault_salt: Vec<u8>,
    clock: C,
    rows: VersionedCfStore,
    durable: Option<DurableVault>,
    dedup_policy: DedupPolicy,
    retention_horizon: Mutex<RetentionHorizon>,
    ledger_hook: Option<AsterLedgerHook>,
    recurrence_write_lock: Mutex<()>,
    recovery_report: VaultRecoveryReport,
    residency: Option<crate::residency::Residency>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultRecoveryReport {
    pub last_recovered_seq: Seq,
    pub torn_tail: Option<TornTail>,
}

impl AsterVault<SystemClock> {
    /// Creates a vault using the system clock.
    pub fn new(vault_id: VaultId, vault_salt: impl Into<Vec<u8>>) -> Self {
        Self::with_clock(vault_id, vault_salt, SystemClock)
    }

    pub fn new_durable(
        vault_dir: impl AsRef<Path>,
        vault_id: VaultId,
        vault_salt: impl Into<Vec<u8>>,
        options: VaultOptions,
    ) -> Result<Self> {
        Self::open(vault_dir, vault_id, vault_salt, options)
    }

    pub fn open(
        vault_dir: impl AsRef<Path>,
        vault_id: VaultId,
        vault_salt: impl Into<Vec<u8>>,
        options: VaultOptions,
    ) -> Result<Self> {
        AsterVault::open_with_clock(vault_dir, vault_id, vault_salt, options, SystemClock)
    }
}

impl<C> AsterVault<C>
where
    C: Clock,
{
    /// Opens a durable vault with an injected clock.
    ///
    /// Production callers use [`AsterVault::open`] with [`SystemClock`]. This
    /// constructor exists for deterministic FSV: the vault remains fully
    /// durable, but group commits stamp `time_index` rows from `clock`.
    pub fn new_durable_with_clock(
        vault_dir: impl AsRef<Path>,
        vault_id: VaultId,
        vault_salt: impl Into<Vec<u8>>,
        options: VaultOptions,
        clock: C,
    ) -> Result<Self> {
        Self::open_with_clock(vault_dir, vault_id, vault_salt, options, clock)
    }

    /// Opens a durable vault with an injected clock.
    pub fn open_with_clock(
        vault_dir: impl AsRef<Path>,
        vault_id: VaultId,
        vault_salt: impl Into<Vec<u8>>,
        options: VaultOptions,
        clock: C,
    ) -> Result<Self> {
        DurableVault::validate_options(&options)?;
        let vault_root = vault_dir.as_ref().to_path_buf();
        let recovery = DurableVault::recover_batches(vault_dir.as_ref(), &options)?;
        let ledger_hook = ledger_hook::recover_hook_from_vault_dir(
            vault_dir.as_ref(),
            &recovery,
            options.ledger_checkpoint.clone(),
        )?;
        let recovery_report = VaultRecoveryReport {
            last_recovered_seq: recovery.last_recovered_seq,
            torn_tail: recovery.torn_tail.clone(),
        };
        let router = CfRouter::open_with_tiering(
            vault_dir.as_ref(),
            options.memtable_byte_cap,
            options.tiering_policy.clone(),
        )?;
        let rows = VersionedCfStore::new_with_router(recovery.last_recovered_seq, router);
        for batch in recovery.batches {
            let rows_at_seq = batch
                .rows
                .into_iter()
                .map(|row| (row.cf, row.key, row.value));
            rows.restore_batch(batch.seq, rows_at_seq)?;
        }
        rows.set_start_seq(recovery.last_recovered_seq)?;
        let mut durable_options = options.clone();
        durable_options.temporal_policy = recovery.temporal_policy;
        durable_options.dedup_policy = recovery.dedup_policy;
        durable_options.retention_horizon = recovery.retention_horizon.clone();
        let dedup_policy = durable_options.dedup_policy.clone().unwrap_or_default();
        let retention_horizon = durable_options.retention_horizon.clone();
        let durable = DurableVault::open(vault_dir, &durable_options)?;
        // Data residency (PRD 30 §4): a caller-supplied pin is enforced against
        // tiering and persisted (conflict-checked, immutable); on reopen the
        // on-disk pin is authoritative and re-enforced against tiering.
        if let Some(pin) = &options.residency {
            if let Some(tiering) = &options.tiering_policy {
                pin.enforce_tier_roots(&tiering.tier_roots())?;
            }
            pin.persist(&vault_root)?;
        }
        let residency = crate::residency::Residency::load(&vault_root)?;
        if options.residency.is_none()
            && let (Some(pin), Some(tiering)) = (&residency, &options.tiering_policy)
        {
            pin.enforce_tier_roots(&tiering.tier_roots())?;
        }
        Ok(Self {
            vault_id,
            vault_salt: vault_salt.into(),
            clock,
            rows,
            durable: Some(durable),
            dedup_policy,
            retention_horizon: Mutex::new(retention_horizon),
            ledger_hook: Some(ledger_hook),
            recurrence_write_lock: Mutex::new(()),
            recovery_report,
            residency,
        })
    }

    /// Creates a vault with an injected clock.
    pub fn with_clock(vault_id: VaultId, vault_salt: impl Into<Vec<u8>>, clock: C) -> Self {
        Self {
            vault_id,
            vault_salt: vault_salt.into(),
            clock,
            rows: VersionedCfStore::default(),
            durable: None,
            dedup_policy: DedupPolicy::default(),
            retention_horizon: Mutex::new(RetentionHorizon::default()),
            ledger_hook: None,
            recurrence_write_lock: Mutex::new(()),
            recovery_report: VaultRecoveryReport {
                last_recovered_seq: 0,
                torn_tail: None,
            },
            residency: None,
        }
    }

    /// Returns the vault's data-residency pin, if one is set (PRD `30 §4`).
    pub fn residency(&self) -> Option<&crate::residency::Residency> {
        self.residency.as_ref()
    }

    /// Authorizes an external copy/export to `target` against the residency pin.
    /// With no pin set, every target is authorized. On a violation, an
    /// `EntryKind::Admin` governance entry is written to the Ledger (the audit
    /// trail) and `CALYX_RESIDENCY_VIOLATION` is returned — fail closed, never a
    /// silent off-dataset copy.
    pub fn authorize_external_copy(&self, target: &std::path::Path) -> Result<()> {
        let Some(residency) = &self.residency else {
            return Ok(());
        };
        match residency.authorize(target) {
            Ok(()) => Ok(()),
            Err(violation) => {
                // The Ledger forbids raw paths (potential secrets), so the
                // immutable audit references paths by verifiable blake3 digest;
                // the human-readable paths travel in the returned error message.
                let payload = serde_json::to_vec(&serde_json::json!({
                    "event": "residency_violation",
                    "dataset_root_hash": residency.dataset_root_digest(),
                    "attempted_target_hash": crate::residency::Residency::path_digest(target),
                    "allow_off_dataset": residency.allow_off_dataset,
                }))
                .map_err(|error| CalyxError {
                    code: "CALYX_RESIDENCY_CORRUPT",
                    message: format!("encode residency audit payload: {error}"),
                    remediation: "report this bug; residency audit payload must be serializable",
                })?;
                self.append_ledger_entry(
                    calyx_ledger::EntryKind::Admin,
                    calyx_ledger::SubjectId::Guard(residency.audit_subject()),
                    payload,
                    calyx_ledger::ActorId::System,
                )?;
                Err(violation)
            }
        }
    }

    pub fn with_clock_and_dedup_policy(
        vault_id: VaultId,
        vault_salt: impl Into<Vec<u8>>,
        clock: C,
        dedup_policy: DedupPolicy,
    ) -> Result<Self> {
        dedup_policy.validate_manifest()?;
        let mut vault = Self::with_clock(vault_id, vault_salt, clock);
        vault.dedup_policy = dedup_policy;
        Ok(vault)
    }

    /// Computes the PRD content-addressed id for raw input bytes.
    pub fn cx_id_for_input(&self, input_bytes: &[u8], panel_version: u32) -> CxId {
        CxId::from_input(input_bytes, panel_version, &self.vault_salt)
    }

    /// Returns the latest committed vault sequence.
    pub fn latest_seq(&self) -> Seq {
        self.rows.current_seq()
    }

    pub fn recovery_report(&self) -> &VaultRecoveryReport {
        &self.recovery_report
    }

    pub fn vault_id(&self) -> VaultId {
        self.vault_id
    }

    pub(crate) fn clock_now(&self) -> u64 {
        self.clock.now()
    }

    pub fn dedup_policy(&self) -> &DedupPolicy {
        &self.dedup_policy
    }

    #[cfg(test)]
    pub(crate) fn fail_next_wal_append_for_test(&self) {
        self.durable
            .as_ref()
            .expect("test WAL failpoint requires durable vault")
            .fail_next_wal_append();
    }

    /// Reads one raw CF row at `snapshot`.
    pub fn read_cf_at(
        &self,
        snapshot: Seq,
        cf: ColumnFamily,
        key: &[u8],
    ) -> Result<Option<Vec<u8>>> {
        self.rows
            .read_at(self.snapshot_handle(snapshot), cf, key, &self.clock)
    }

    /// Writes raw CF rows through the same WAL/MVCC commit path as vault puts.
    pub fn write_cf_batch(
        &self,
        rows: impl IntoIterator<Item = (ColumnFamily, Vec<u8>, Vec<u8>)>,
    ) -> Result<Seq> {
        let rows = rows
            .into_iter()
            .map(|(cf, key, value)| encode::WriteRow { cf, key, value })
            .collect::<Vec<_>>();
        if rows.is_empty() {
            return Ok(self.latest_seq());
        }
        self.commit_rows(&rows)
    }

    /// Writes one raw CF row through the WAL-backed batch path.
    pub fn write_cf(&self, cf: ColumnFamily, key: Vec<u8>, value: Vec<u8>) -> Result<Seq> {
        self.write_cf_batch([(cf, key, value)])
    }

    /// Scans visible raw CF rows at `snapshot`.
    pub fn scan_cf_at(&self, snapshot: Seq, cf: ColumnFamily) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        self.rows
            .scan_cf_at(self.snapshot_handle(snapshot), cf, &self.clock)
    }

    /// Scans visible raw CF rows in a key range at `snapshot`.
    pub fn scan_cf_range_at(
        &self,
        snapshot: Seq,
        cf: ColumnFamily,
        range: &KeyRange,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        self.rows
            .scan_cf_range_at(self.snapshot_handle(snapshot), cf, range, &self.clock)
    }

    /// Scans at most `limit` visible raw CF rows in key order after `after_key`.
    pub fn scan_cf_range_page_at(
        &self,
        snapshot: Seq,
        cf: ColumnFamily,
        range: &KeyRange,
        after_key: Option<&[u8]>,
        limit: usize,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        self.rows.scan_cf_range_page_at(
            self.snapshot_handle(snapshot),
            cf,
            range,
            after_key,
            limit,
            &self.clock,
        )
    }

    pub(super) fn stage_constellation_rows(
        &self,
        rows: &mut Vec<encode::WriteRow>,
        constellation: &Constellation,
    ) -> Result<()> {
        constellation.validate_schema()?;
        let id = constellation.cx_id;
        rows.push(encode::WriteRow {
            cf: ColumnFamily::Base,
            key: base_key(id),
            value: encode::encode_constellation_base(constellation)?,
        });
        for (slot, vector) in &constellation.slots {
            rows.push(encode::WriteRow {
                cf: ColumnFamily::slot(*slot),
                key: slot_key(id),
                value: encode::encode_slot_vector(vector)?,
            });
        }
        for anchor in &constellation.anchors {
            rows.push(encode::WriteRow {
                cf: ColumnFamily::Anchors,
                key: anchor_key(id, &anchor.kind),
                value: encode::encode_anchor(anchor)?,
            });
        }
        Ok(())
    }

    fn snapshot_handle(&self, seq: Seq) -> Snapshot {
        let lease = ReaderLease::new(0, seq, self.clock.now(), DEFAULT_LEASE_MS);
        Snapshot::new(seq, Freshness::FreshDerived, lease)
    }

    pub fn flush(&self) -> Result<()> {
        if let Some(durable) = &self.durable {
            durable.flush()?;
        }
        self.rows.flush_all_cfs()?;
        Ok(())
    }

    /// Pins an explicit reader lease tracked for oldest-pinned-seq accounting.
    ///
    /// Unlike vault-internal snapshot handles, leases pinned here register in
    /// the store lease registry and hold the oldest-pinned-seq gap open until
    /// [`Self::release_reader`] or lease expiry.
    pub fn pin_reader(&self, freshness: Freshness, max_age_ms: u64) -> Snapshot {
        self.rows.pin_snapshot(freshness, &self.clock, max_age_ms)
    }

    /// Releases an explicit reader lease; returns whether it was still live.
    pub fn release_reader(&self, lease_id: u64) -> bool {
        self.rows.release_lease(lease_id)
    }

    /// Pins a reader lease at a historical `seq` (time-travel) and returns its
    /// lease id, which the caller must release with [`Self::release_reader`].
    pub fn pin_reader_at(&self, seq: Seq, max_age_ms: u64) -> u64 {
        self.rows
            .pin_snapshot_at(seq, Freshness::FreshDerived, &self.clock, max_age_ms)
            .lease()
            .id()
    }

    /// Opens a time-travel snapshot as of wall-clock `t_millis` (PRD `17 §8`).
    pub fn as_of(&self, t_millis: u64) -> Result<crate::timetravel::TimeTravelSnapshot<'_, C>> {
        crate::timetravel::TimeTravelSnapshot::open(self, t_millis)
    }

    #[cfg(test)]
    pub(crate) fn clock_ref(&self) -> &C {
        &self.clock
    }

    /// Collects the aggregate resource status for this vault (PRD 18 §4).
    ///
    /// `vault_dir` is the durable root this vault was opened from; `vram` is
    /// the VRAM budget section sourced from the vault Anneal budget config.
    pub fn resource_status(
        &self,
        vault_dir: &Path,
        vram: VramBudgetStatus,
    ) -> Result<ResourceStatus> {
        collect_resource_status(vault_dir, vram, &self.rows, self.clock.now())
    }

    pub fn install_read_barrier(&self, barrier: ReadBarrier) {
        self.rows.install_read_barrier(barrier);
    }

    pub fn remove_read_barrier(&self, id: &str) -> bool {
        self.rows.remove_read_barrier(id)
    }

    pub fn read_barriers(&self) -> Vec<ReadBarrier> {
        self.rows.read_barriers()
    }
}

#[cfg(test)]
mod compaction_tests;

#[cfg(test)]
mod recovery_tests;

#[cfg(test)]
mod ledger_timestamp_tests;

#[cfg(test)]
mod ledger_integration_tests;

#[cfg(test)]
mod ledger_atomicity_tests;

#[cfg(test)]
mod ledger_checkpoint_tests;

#[cfg(test)]
mod anchor_merge_tests;

#[cfg(test)]
mod tests;
