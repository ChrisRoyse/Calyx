//! PH58 orphan slot/index reconciler.

use crate::cf::{ColumnFamily, base_key, slot_key};
use crate::mvcc::tombstone_value;
use crate::vault::AsterVault;
use crate::vault::encode::{decode_constellation_base, encode_constellation_base};
use calyx_core::{CalyxError, Clock, CxId, Result, SlotId};
use calyx_ledger::{ActorId, EntryKind, SubjectId};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

pub const CALYX_ORPHAN_RECONCILER_ERROR: &str = "CALYX_ORPHAN_RECONCILER_ERROR";
const REBUILD_PREFIX: &[u8] = b"orphan_slot_rebuild\0";
const REBUILD_METADATA_KEY: &str = "gc.orphan_reconciler";
const REBUILD_METADATA_VALUE: &str = "slot_rebuild_queued";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrphanBaseEntry {
    pub cx_id: CxId,
    pub expected_slots: Vec<SlotId>,
    pub repair_queued: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct OrphanIndexEntry {
    pub cx_id: CxId,
    pub slot: SlotId,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OrphanReport {
    pub orphan_index: Vec<CxId>,
    pub orphan_base: Vec<CxId>,
    pub orphan_index_entries: Vec<OrphanIndexEntry>,
    pub inconsistencies: usize,
}

impl OrphanReport {
    pub fn to_metrics_text(&self, vault_label: &str) -> String {
        let vault = escape_label(vault_label);
        let mut out = String::new();
        let _ = writeln!(
            out,
            "calyx_orphan_index_entries_total{{vault=\"{vault}\"}} {}",
            self.orphan_index_entries.len()
        );
        let _ = writeln!(
            out,
            "calyx_orphan_base_entries_total{{vault=\"{vault}\"}} {}",
            self.orphan_base.len()
        );
        out
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrphanRepairResult {
    pub orphan_index_repaired: usize,
    pub orphan_base_degraded: usize,
    pub repairs_total: u64,
    pub remaining_inconsistencies: usize,
    pub rate_limited: bool,
}

impl OrphanRepairResult {
    pub fn to_metrics_text(&self, vault_label: &str) -> String {
        let vault = escape_label(vault_label);
        format!(
            "calyx_orphan_repairs_total{{vault=\"{vault}\"}} {}\n",
            self.repairs_total
        )
    }
}

pub trait OrphanGcTarget {
    fn base_entries(&self) -> Result<Vec<OrphanBaseEntry>>;
    fn slot_index_entries(&self) -> Result<Vec<OrphanIndexEntry>>;
    fn purge_orphan_index(&self, cx_id: CxId, slots: &[SlotId]) -> Result<usize>;
    fn flag_orphan_base(&self, cx_id: CxId) -> Result<()>;
}

#[derive(Debug)]
pub struct OrphanReconciler {
    pub scan_interval: Duration,
    pub max_repairs_per_run: usize,
    orphan_repairs_total: AtomicU64,
}

impl OrphanReconciler {
    pub fn new(scan_interval: Duration, max_repairs_per_run: usize) -> Self {
        Self {
            scan_interval,
            max_repairs_per_run,
            orphan_repairs_total: AtomicU64::new(0),
        }
    }

    pub fn scan<T>(&self, target: &T) -> Result<OrphanReport>
    where
        T: OrphanGcTarget + ?Sized,
    {
        let base_entries = target.base_entries()?;
        let slot_entries = target.slot_index_entries()?;
        let base_by_cx = base_entries
            .into_iter()
            .map(|entry| (entry.cx_id, entry))
            .collect::<BTreeMap<_, _>>();
        let slot_by_cx = slot_entries.iter().fold(
            BTreeMap::<CxId, BTreeSet<SlotId>>::new(),
            |mut acc, entry| {
                acc.entry(entry.cx_id).or_default().insert(entry.slot);
                acc
            },
        );

        let mut orphan_index = BTreeSet::new();
        let mut orphan_index_entries = Vec::new();
        for entry in slot_entries {
            if !base_by_cx.contains_key(&entry.cx_id) {
                orphan_index.insert(entry.cx_id);
                orphan_index_entries.push(entry);
            }
        }

        let mut orphan_base = Vec::new();
        for (cx_id, base) in &base_by_cx {
            if base.expected_slots.is_empty() || base.repair_queued {
                continue;
            }
            let has_any_expected = slot_by_cx
                .get(cx_id)
                .is_some_and(|slots| base.expected_slots.iter().any(|slot| slots.contains(slot)));
            if !has_any_expected {
                orphan_base.push(*cx_id);
            }
        }

        orphan_index_entries.sort_unstable();
        let orphan_index = orphan_index.into_iter().collect::<Vec<_>>();
        let inconsistencies = orphan_index.len() + orphan_base.len();
        Ok(OrphanReport {
            orphan_index,
            orphan_base,
            orphan_index_entries,
            inconsistencies,
        })
    }

    pub fn repair<T>(&self, target: &T, report: &OrphanReport) -> Result<OrphanRepairResult>
    where
        T: OrphanGcTarget + ?Sized,
    {
        let mut remaining_budget = self.max_repairs_per_run;
        let mut repaired_index = 0;
        let mut degraded_base = 0;

        for cx_id in &report.orphan_index {
            if remaining_budget == 0 {
                break;
            }
            let slots = report
                .orphan_index_entries
                .iter()
                .filter_map(|entry| (entry.cx_id == *cx_id).then_some(entry.slot))
                .collect::<Vec<_>>();
            let purged = target.purge_orphan_index(*cx_id, &slots)?;
            if purged > 0 {
                repaired_index += 1;
                remaining_budget -= 1;
            }
        }

        for cx_id in &report.orphan_base {
            if remaining_budget == 0 {
                break;
            }
            target.flag_orphan_base(*cx_id)?;
            degraded_base += 1;
            remaining_budget -= 1;
        }

        let repaired = repaired_index + degraded_base;
        let repairs_total = self
            .orphan_repairs_total
            .fetch_add(repaired as u64, Ordering::Relaxed)
            + repaired as u64;
        let remaining_inconsistencies = report.inconsistencies.saturating_sub(repaired);
        Ok(OrphanRepairResult {
            orphan_index_repaired: repaired_index,
            orphan_base_degraded: degraded_base,
            repairs_total,
            remaining_inconsistencies,
            rate_limited: remaining_inconsistencies > 0,
        })
    }
}

impl Default for OrphanReconciler {
    fn default() -> Self {
        Self::new(Duration::from_secs(300), 1_000)
    }
}

pub struct VaultOrphanGcTarget<'a, C> {
    vault: &'a AsterVault<C>,
    slots: Vec<SlotId>,
    compact_after_tombstone: bool,
}

impl<'a, C> VaultOrphanGcTarget<'a, C> {
    pub fn new(vault: &'a AsterVault<C>, slots: impl IntoIterator<Item = SlotId>) -> Self {
        let mut slots = slots.into_iter().collect::<Vec<_>>();
        slots.sort_unstable_by_key(|slot| slot.get());
        slots.dedup();
        Self {
            vault,
            slots,
            compact_after_tombstone: true,
        }
    }

    pub fn without_auto_compaction(mut self) -> Self {
        self.compact_after_tombstone = false;
        self
    }
}

impl<C> OrphanGcTarget for VaultOrphanGcTarget<'_, C>
where
    C: Clock,
{
    fn base_entries(&self) -> Result<Vec<OrphanBaseEntry>> {
        let mut entries = Vec::new();
        for (key, bytes) in self
            .vault
            .scan_cf_at(self.vault.latest_seq(), ColumnFamily::Base)?
        {
            let cx_id = key_to_cx(&key)?;
            let cx = decode_constellation_base(&bytes)?;
            entries.push(OrphanBaseEntry {
                cx_id,
                expected_slots: cx.slots.keys().copied().collect(),
                repair_queued: cx.flags.degraded
                    && cx
                        .metadata
                        .get(REBUILD_METADATA_KEY)
                        .is_some_and(|state| state == REBUILD_METADATA_VALUE),
            });
        }
        entries.sort_by_key(|entry| entry.cx_id);
        Ok(entries)
    }

    fn slot_index_entries(&self) -> Result<Vec<OrphanIndexEntry>> {
        let mut entries = Vec::new();
        for slot in &self.slots {
            for (key, _) in self
                .vault
                .scan_cf_at(self.vault.latest_seq(), ColumnFamily::slot(*slot))?
            {
                entries.push(OrphanIndexEntry {
                    cx_id: key_to_cx(&key)?,
                    slot: *slot,
                });
            }
        }
        entries.sort_unstable();
        Ok(entries)
    }

    fn purge_orphan_index(&self, cx_id: CxId, slots: &[SlotId]) -> Result<usize> {
        let slots = if slots.is_empty() { &self.slots } else { slots };
        let mut rows = Vec::new();
        for slot in slots {
            let cf = ColumnFamily::slot(*slot);
            let key = slot_key(cx_id);
            if self
                .vault
                .read_cf_at(self.vault.latest_seq(), cf, &key)?
                .is_some()
            {
                rows.push((cf, key, tombstone_value()));
            }
        }
        if rows.is_empty() {
            return Ok(0);
        }
        let affected = rows.iter().map(|(cf, _, _)| *cf).collect::<Vec<_>>();
        self.vault.write_cf_batch(rows)?;
        if self.compact_after_tombstone {
            self.vault.purge_tombstoned_cfs(&affected)?;
        }
        self.vault.append_ledger_entry(
            EntryKind::Admin,
            SubjectId::Cx(cx_id),
            orphan_payload("orphan_index_purged", affected.len())?,
            ActorId::System,
        )?;
        Ok(affected.len())
    }

    fn flag_orphan_base(&self, cx_id: CxId) -> Result<()> {
        let key = base_key(cx_id);
        let Some(bytes) =
            self.vault
                .read_cf_at(self.vault.latest_seq(), ColumnFamily::Base, &key)?
        else {
            return Err(orphan_error("base row disappeared before repair"));
        };
        let mut cx = decode_constellation_base(&bytes)?;
        cx.flags.degraded = true;
        cx.metadata.insert(
            REBUILD_METADATA_KEY.to_string(),
            REBUILD_METADATA_VALUE.to_string(),
        );
        let mut rebuild_key = REBUILD_PREFIX.to_vec();
        rebuild_key.extend_from_slice(cx_id.as_bytes());
        let rows = vec![
            (ColumnFamily::Base, key, encode_constellation_base(&cx)?),
            (
                ColumnFamily::AnnealReplay,
                rebuild_key,
                orphan_payload("orphan_base_rebuild_requested", cx.slots.len())?,
            ),
        ];
        self.vault.write_cf_batch(rows)?;
        self.vault.append_ledger_entry(
            EntryKind::Admin,
            SubjectId::Cx(cx_id),
            orphan_payload("orphan_base_degraded", cx.slots.len())?,
            ActorId::System,
        )?;
        Ok(())
    }
}

fn key_to_cx(key: &[u8]) -> Result<CxId> {
    let bytes: [u8; 16] = key
        .try_into()
        .map_err(|_| orphan_error("slot/base key is not a 16-byte CxId"))?;
    Ok(CxId::from_bytes(bytes))
}

fn orphan_payload(event: &str, count: usize) -> Result<Vec<u8>> {
    serde_json::to_vec(&serde_json::json!({ "event": event, "count": count }))
        .map_err(|error| orphan_error(format!("encode orphan repair ledger payload: {error}")))
}

fn orphan_error(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_ORPHAN_RECONCILER_ERROR,
        message: message.into(),
        remediation: "rerun orphan scan, inspect base/slot CF bytes, and repair from WAL",
    }
}

fn escape_label(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests;
