use crate::cf::{ColumnFamily, anchor_key, base_key, ledger_key, slot_key};
use crate::mvcc::CfRead;
use calyx_core::{Anchor, CalyxError, Clock, Constellation, CxId, Result, Seq, SlotId, VaultStore};
use std::collections::BTreeMap;

use super::{AsterVault, anchor_merge, encode, ledger_hook, ledger_stub};

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
        constellation.validate_schema()?;

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
                let mut merged = self.get(id, latest)?;
                let added = anchor_merge::merge_duplicate_anchors(&mut merged, &constellation)?;
                if !added.is_empty() {
                    let rows = anchor_merge::stage_anchor_merge_rows(id, &merged, &added)?;
                    self.commit_rows_locked(&rows)?;
                }
                return Ok(id);
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
            let vector = match encode::decode_slot_vector(&value) {
                Ok(vector) => vector,
                Err(_) => {
                    let raw = self
                        .rows
                        .read_at(
                            handle,
                            ColumnFamily::slot_raw(slot),
                            &slot_key(id),
                            &self.clock,
                        )?
                        .ok_or_else(|| {
                            CalyxError::aster_corrupt_shard(
                                "compressed slot CF row missing raw sidecar",
                            )
                        })?;
                    encode::decode_slot_vector(&raw)?
                }
            };
            slots.insert(slot, vector);
        }
        constellation.slots = slots;
        Ok(constellation)
    }

    fn anchor(&self, id: CxId, anchor: Anchor) -> Result<()> {
        anchor.validate_schema()?;
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
