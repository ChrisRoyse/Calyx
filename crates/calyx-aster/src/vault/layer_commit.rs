use super::{AsterVault, durable, encode, ledger_hook};
use crate::cf::ColumnFamily;
use calyx_core::{CalyxError, Clock, Result, Seq};
use calyx_ledger::{ActorId, EntryKind, SubjectId};

impl<C> AsterVault<C>
where
    C: Clock,
{
    pub fn write_cf_batch_with_ledger_entry(
        &self,
        rows: impl IntoIterator<Item = (ColumnFamily, Vec<u8>, Vec<u8>)>,
        kind: EntryKind,
        subject: SubjectId,
        payload: Vec<u8>,
        actor: ActorId,
    ) -> Result<Seq> {
        let mut data_rows = rows
            .into_iter()
            .map(|(cf, key, value)| encode::WriteRow { cf, key, value })
            .collect::<Vec<_>>();
        if data_rows.is_empty() {
            return Ok(self.latest_seq());
        }

        self.with_durable_commit_lock(|| {
            if let Some(hook) = &self.ledger_hook {
                let mut hook = ledger_hook::lock_hook(hook)?;
                let mut rows = Vec::with_capacity(data_rows.len() + 2);
                let staged = ledger_hook::stage_entry_payload(
                    &hook, &mut rows, kind, subject, payload, actor,
                )?;
                let ledger_ref = staged_ledger_ref(&staged)?;
                attach_ledger_ref_to_base_rows(&mut data_rows, &ledger_ref)?;
                rows.extend(data_rows);
                let seq = self.commit_rows_locked(&rows)?;
                ledger_hook::commit_staged(&mut hook, &staged)?;
                return Ok(seq);
            }

            let mut transient = self.transient_ledger_hook()?;
            let hook = transient
                .get_mut()
                .map_err(|_| CalyxError::ledger_group_commit_failed("transient hook poisoned"))?;
            let mut rows = Vec::with_capacity(data_rows.len() + 1);
            let staged =
                ledger_hook::stage_entry_payload(hook, &mut rows, kind, subject, payload, actor)?;
            let ledger_ref = staged_ledger_ref(&staged)?;
            attach_ledger_ref_to_base_rows(&mut data_rows, &ledger_ref)?;
            rows.extend(data_rows);
            let seq = self.commit_rows_locked(&rows)?;
            ledger_hook::commit_staged(hook, &staged)?;
            Ok(seq)
        })
    }

    fn transient_ledger_hook(&self) -> Result<ledger_hook::AsterLedgerHook> {
        let ledger_rows = self
            .scan_cf_at(self.latest_seq(), ColumnFamily::Ledger)?
            .into_iter()
            .map(|(key, value)| encode::WriteRow {
                cf: ColumnFamily::Ledger,
                key,
                value,
            })
            .collect::<Vec<_>>();
        let batches = if ledger_rows.is_empty() {
            Vec::new()
        } else {
            vec![durable::RecoveredBatch {
                seq: self.latest_seq(),
                rows: ledger_rows,
            }]
        };
        ledger_hook::recover_hook(
            &durable::RecoveredBatches {
                batches,
                last_recovered_seq: self.latest_seq(),
                torn_tail: None,
                temporal_policy: None,
                dedup_policy: None,
                retention_horizon: crate::timetravel::RetentionHorizon::default(),
            },
            None,
        )
    }
}

fn staged_ledger_ref(staged: &[calyx_ledger::StagedLedgerRow]) -> Result<calyx_core::LedgerRef> {
    staged
        .first()
        .map(calyx_ledger::StagedLedgerRow::ledger_ref)
        .ok_or_else(|| CalyxError::ledger_group_commit_failed("no staged ledger rows"))
}

fn attach_ledger_ref_to_base_rows(
    rows: &mut [encode::WriteRow],
    ledger_ref: &calyx_core::LedgerRef,
) -> Result<()> {
    for row in rows.iter_mut().filter(|row| row.cf == ColumnFamily::Base) {
        let mut constellation = encode::decode_constellation_base(&row.value)?;
        constellation.provenance = ledger_ref.clone();
        row.value = encode::encode_constellation_base(&constellation)?;
    }
    Ok(())
}
