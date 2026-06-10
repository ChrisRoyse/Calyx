//! Aster `VaultStore` implementation over the PH08 MVCC CF table.

mod anchor_codec;
mod cf_codec;
mod commit;
mod compaction_bridge;
mod cursor;
mod dedup_commit;
mod durable;
pub mod encode;
mod ledger_hook;
pub mod ledger_stub;
mod router_bridge;
mod slot_backfill;
mod slot_column;
mod temporal_xterm;

use crate::cf::{CfRouter, ColumnFamily, anchor_key, base_key, ledger_key, slot_key};
use crate::dedup::{AnchorConflictResult, DedupPolicy, check_anchor_conflict};
use crate::mvcc::{CfRead, Freshness, ReaderLease, Snapshot, VersionedCfStore};
use crate::vault::durable::DurableVault;
use crate::vault::ledger_hook::AsterLedgerHook;
use crate::wal::TornTail;
use calyx_core::{
    Anchor, CalyxError, Clock, Constellation, CxId, Result, Seq, SlotId, SystemClock, VaultId,
    VaultStore,
};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;

pub use compaction_bridge::VaultCompactionScheduler;
pub use durable::VaultOptions;
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
    ledger_hook: Option<AsterLedgerHook>,
    recurrence_write_lock: Mutex<()>,
    recovery_report: VaultRecoveryReport,
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
        DurableVault::validate_options(&options)?;
        let recovery = DurableVault::recover_batches(vault_dir.as_ref(), &options)?;
        let ledger_hook = ledger_hook::recover_hook(&recovery, options.ledger_checkpoint.clone())?;
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
        let dedup_policy = durable_options.dedup_policy.clone().unwrap_or_default();
        let durable = DurableVault::open(vault_dir, &durable_options)?;
        Ok(Self {
            vault_id,
            vault_salt: vault_salt.into(),
            clock: SystemClock,
            rows,
            durable: Some(durable),
            dedup_policy,
            ledger_hook: Some(ledger_hook),
            recurrence_write_lock: Mutex::new(()),
            recovery_report,
        })
    }
}

impl<C> AsterVault<C>
where
    C: Clock,
{
    /// Creates a vault with an injected clock.
    pub fn with_clock(vault_id: VaultId, vault_salt: impl Into<Vec<u8>>, clock: C) -> Self {
        Self {
            vault_id,
            vault_salt: vault_salt.into(),
            clock,
            rows: VersionedCfStore::default(),
            durable: None,
            dedup_policy: DedupPolicy::default(),
            ledger_hook: None,
            recurrence_write_lock: Mutex::new(()),
            recovery_report: VaultRecoveryReport {
                last_recovered_seq: 0,
                torn_tail: None,
            },
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

    /// Scans visible raw CF rows at `snapshot`.
    pub fn scan_cf_at(&self, snapshot: Seq, cf: ColumnFamily) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        self.rows
            .scan_cf_at(self.snapshot_handle(snapshot), cf, &self.clock)
    }

    pub(super) fn stage_constellation_rows(
        &self,
        rows: &mut Vec<encode::WriteRow>,
        constellation: &Constellation,
    ) -> Result<()> {
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
}

impl<C> VaultStore for AsterVault<C>
where
    C: Clock,
{
    fn put(&self, constellation: Constellation) -> Result<CxId> {
        if constellation.vault_id != self.vault_id {
            return Err(CalyxError::vault_access_denied(
                "constellation belongs to another vault",
            ));
        }

        self.with_durable_commit_lock(|| {
            let mut constellation = constellation;
            let id = constellation.cx_id;
            let base_key = base_key(id);
            let latest = self.snapshot();
            if let Some(existing) = self.rows.read_at(
                self.snapshot_handle(latest),
                ColumnFamily::Base,
                &base_key,
                &self.clock,
            )? {
                let base_bytes = encode::encode_constellation_base(&constellation)?;
                if existing == base_bytes {
                    return Ok(id);
                }
                if encode::same_constellation_identity(&existing, &base_bytes)? {
                    let existing_cx = encode::decode_constellation_base(&existing)?;
                    let incoming_cx = encode::decode_constellation_base(&base_bytes)?;
                    if let AnchorConflictResult::Conflicting {
                        anchor_type,
                        reason,
                    } = check_anchor_conflict(&incoming_cx, &existing_cx)
                    {
                        return Err(CalyxError::aster_corrupt_shard(format!(
                            "CxId duplicate has conflicting {anchor_type:?} anchor: {reason:?}"
                        )));
                    }
                    return Ok(id);
                }
                return Err(CalyxError::aster_corrupt_shard(
                    "CxId collision or non-idempotent duplicate constellation",
                ));
            }

            let mut rows = Vec::new();
            let mut hook_guard = match &self.ledger_hook {
                Some(hook) => Some(ledger_hook::lock_hook(hook)?),
                None => None,
            };
            let staged_ledger = if let Some(hook) = hook_guard.as_deref() {
                let staged = ledger_hook::stage_ingest(hook, &mut rows, &constellation)?;
                constellation.provenance = staged
                    .first()
                    .ok_or_else(|| CalyxError::ledger_group_commit_failed("no staged ledger rows"))?
                    .ledger_ref();
                Some(staged)
            } else {
                rows.push(encode::WriteRow {
                    cf: ColumnFamily::Ledger,
                    key: ledger_key(constellation.provenance.seq),
                    value: ledger_stub::encode(constellation.provenance.seq),
                });
                None
            };
            let base_bytes = encode::encode_constellation_base(&constellation)?;
            rows.push(encode::WriteRow {
                cf: ColumnFamily::Base,
                key: base_key,
                value: base_bytes,
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
            self.commit_rows_locked(&rows)?;
            if let (Some(hook), Some(staged)) = (hook_guard.as_deref_mut(), staged_ledger.as_ref())
            {
                ledger_hook::commit_staged(hook, staged)?;
            }
            Ok(id)
        })
    }

    fn get(&self, id: CxId, snapshot: Seq) -> Result<Constellation> {
        let handle = self.snapshot_handle(snapshot);
        let base = self
            .rows
            .read_at(handle, ColumnFamily::Base, &base_key(id), &self.clock)?
            .ok_or_else(|| CalyxError::stale_derived("constellation missing at snapshot"))?;
        let mut constellation = encode::decode_constellation_base(&base)?;
        let slot_ids: Vec<SlotId> = constellation.slots.keys().copied().collect();
        let reads: Vec<_> = slot_ids
            .iter()
            .map(|slot| CfRead::new(ColumnFamily::slot(*slot), slot_key(id)))
            .collect();
        let values = self.rows.read_batch(handle, &reads, &self.clock)?;
        let mut slots = BTreeMap::new();
        for (slot, value) in slot_ids.into_iter().zip(values) {
            let value =
                value.ok_or_else(|| CalyxError::aster_corrupt_shard("slot CF row missing"))?;
            slots.insert(slot, encode::decode_slot_vector(&value)?);
        }
        constellation.slots = slots;
        Ok(constellation)
    }

    fn anchor(&self, id: CxId, anchor: Anchor) -> Result<()> {
        self.with_recurrence_write_lock(|| {
            let latest = self.snapshot();
            let mut constellation = self.get(id, latest)?;
            constellation.anchors.push(anchor.clone());
            let rows = [
                (
                    ColumnFamily::Base,
                    base_key(id),
                    encode::encode_constellation_base(&constellation)?,
                ),
                (
                    ColumnFamily::Anchors,
                    anchor_key(id, &anchor.kind),
                    encode::encode_anchor(&anchor)?,
                ),
            ];
            let rows = rows
                .into_iter()
                .map(|(cf, key, value)| encode::WriteRow { cf, key, value })
                .collect::<Vec<_>>();
            self.commit_rows(&rows)?;
            Ok(())
        })
    }

    fn snapshot(&self) -> Seq {
        self.latest_seq()
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
mod tests;
