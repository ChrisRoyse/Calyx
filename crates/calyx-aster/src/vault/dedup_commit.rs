use super::{AsterVault, encode, ledger_hook, ledger_stub};
use crate::cf::{ColumnFamily, base_key, ledger_key};
use calyx_core::{CalyxError, Clock, Constellation, CxId, Result, Seq};

impl<C> AsterVault<C>
where
    C: Clock,
{
    pub(crate) fn commit_recurrence_batch(
        &self,
        recurrence_rows: Vec<(Vec<u8>, Vec<u8>)>,
        updated_base: Option<Constellation>,
    ) -> Result<Seq> {
        let mut rows = Vec::new();
        if let Some(cx) = updated_base.as_ref() {
            if cx.vault_id != self.vault_id {
                return Err(CalyxError::vault_access_denied(
                    "recurrence base update belongs to another vault",
                ));
            }
            rows.push(encode::WriteRow {
                cf: ColumnFamily::Base,
                key: base_key(cx.cx_id),
                value: encode::encode_constellation_base(cx)?,
            });
        }
        rows.extend(
            recurrence_rows
                .into_iter()
                .map(|(key, value)| encode::WriteRow {
                    cf: ColumnFamily::Recurrence,
                    key,
                    value,
                }),
        );
        self.commit_rows(&rows)
    }

    pub(crate) fn commit_online_rows<I>(&self, rows: I) -> Result<Seq>
    where
        I: IntoIterator<Item = (Vec<u8>, Vec<u8>)>,
    {
        let rows = rows
            .into_iter()
            .map(|(key, value)| encode::WriteRow {
                cf: ColumnFamily::Online,
                key,
                value,
            })
            .collect::<Vec<_>>();
        self.commit_rows(&rows)
    }

    pub(crate) fn commit_dedup_ingest(
        &self,
        mut constellation: Option<Constellation>,
        updated_base: Option<Constellation>,
        online_rows: Vec<(Vec<u8>, Vec<u8>)>,
        recurrence_rows: Vec<(Vec<u8>, Vec<u8>)>,
        subject: CxId,
        ledger_payload: Vec<u8>,
    ) -> Result<Seq> {
        self.with_durable_commit_lock(|| {
            let mut rows = Vec::new();
            let mut hook_guard = match &self.ledger_hook {
                Some(hook) => Some(ledger_hook::lock_hook(hook)?),
                None => None,
            };
            let staged_ledger = if let Some(hook) = hook_guard.as_deref() {
                let staged =
                    ledger_hook::stage_ingest_payload(hook, &mut rows, subject, ledger_payload)?;
                if let Some(cx) = constellation.as_mut() {
                    cx.provenance = staged
                        .first()
                        .ok_or_else(|| {
                            CalyxError::ledger_group_commit_failed("no staged ledger rows")
                        })?
                        .ledger_ref();
                }
                Some(staged)
            } else {
                let seq = self
                    .latest_seq()
                    .checked_add(1)
                    .ok_or_else(|| CalyxError::ledger_chain_broken("ledger sequence exhausted"))?;
                rows.push(encode::WriteRow {
                    cf: ColumnFamily::Ledger,
                    key: ledger_key(seq),
                    value: ledger_stub::encode(seq),
                });
                if let Some(cx) = constellation.as_mut() {
                    cx.provenance = calyx_core::LedgerRef { seq, hash: [0; 32] };
                }
                None
            };
            if let Some(cx) = constellation.as_ref() {
                self.stage_constellation_rows(&mut rows, cx)?;
            }
            if let Some(cx) = updated_base.as_ref() {
                if cx.vault_id != self.vault_id {
                    return Err(CalyxError::vault_access_denied(
                        "dedup recurrence base update belongs to another vault",
                    ));
                }
                rows.push(encode::WriteRow {
                    cf: ColumnFamily::Base,
                    key: base_key(cx.cx_id),
                    value: encode::encode_constellation_base(cx)?,
                });
            }
            for (key, value) in online_rows {
                rows.push(encode::WriteRow {
                    cf: ColumnFamily::Online,
                    key,
                    value,
                });
            }
            for (key, value) in recurrence_rows {
                rows.push(encode::WriteRow {
                    cf: ColumnFamily::Recurrence,
                    key,
                    value,
                });
            }
            let seq = self.commit_rows_locked(&rows)?;
            if let (Some(hook), Some(staged)) = (hook_guard.as_deref_mut(), staged_ledger.as_ref())
            {
                ledger_hook::commit_staged(hook, staged)?;
            }
            Ok(seq)
        })
    }

    pub(crate) fn commit_dedup_undo(
        &self,
        restored: Vec<Constellation>,
        updated_bases: Vec<Constellation>,
        recurrence_rows: Vec<(Vec<u8>, Vec<u8>)>,
        subject: CxId,
        ledger_payload: Vec<u8>,
    ) -> Result<Seq> {
        self.with_durable_commit_lock(|| {
            let mut rows = Vec::new();
            let mut hook_guard = match &self.ledger_hook {
                Some(hook) => Some(ledger_hook::lock_hook(hook)?),
                None => None,
            };
            let staged_ledger = if let Some(hook) = hook_guard.as_deref() {
                Some(ledger_hook::stage_ingest_payload(
                    hook,
                    &mut rows,
                    subject,
                    ledger_payload,
                )?)
            } else {
                let seq = self
                    .latest_seq()
                    .checked_add(1)
                    .ok_or_else(|| CalyxError::ledger_chain_broken("ledger sequence exhausted"))?;
                rows.push(encode::WriteRow {
                    cf: ColumnFamily::Ledger,
                    key: ledger_key(seq),
                    value: ledger_stub::encode(seq),
                });
                None
            };
            for cx in &restored {
                if cx.vault_id != self.vault_id {
                    return Err(CalyxError::vault_access_denied(
                        "dedup undo restore belongs to another vault",
                    ));
                }
                self.stage_constellation_rows(&mut rows, cx)?;
            }
            for cx in &updated_bases {
                if cx.vault_id != self.vault_id {
                    return Err(CalyxError::vault_access_denied(
                        "dedup undo base update belongs to another vault",
                    ));
                }
                rows.push(encode::WriteRow {
                    cf: ColumnFamily::Base,
                    key: base_key(cx.cx_id),
                    value: encode::encode_constellation_base(cx)?,
                });
            }
            for (key, value) in recurrence_rows {
                rows.push(encode::WriteRow {
                    cf: ColumnFamily::Recurrence,
                    key,
                    value,
                });
            }
            let rows = latest_rows(rows);
            let seq = self.commit_rows_locked(&rows)?;
            if let (Some(hook), Some(staged)) = (hook_guard.as_deref_mut(), staged_ledger.as_ref())
            {
                ledger_hook::commit_staged(hook, staged)?;
            }
            Ok(seq)
        })
    }
}

fn latest_rows(rows: Vec<encode::WriteRow>) -> Vec<encode::WriteRow> {
    let mut latest = Vec::<encode::WriteRow>::new();
    for row in rows {
        if let Some(index) = latest
            .iter()
            .position(|existing| existing.cf == row.cf && existing.key == row.key)
        {
            latest[index] = row;
        } else {
            latest.push(row);
        }
    }
    latest
}
