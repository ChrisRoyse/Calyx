use super::{AsterVault, encode, ledger_hook};
use crate::cf::{ColumnFamily, anchor_key, base_key, ledger_key};
use crate::ledger_view::parse_aster_ledger_seq;
use calyx_core::{Anchor, CalyxError, Clock, CxId, LedgerRef, Result, SystemClock, VaultStore};
use calyx_ledger::{
    ActorId, EntryKind, LedgerAppender, LedgerCfStore, LedgerHeadAnchor, LedgerRow, SubjectId,
};

struct LedgerEntryInput {
    kind: EntryKind,
    subject: SubjectId,
    payload: Vec<u8>,
    actor: ActorId,
}

impl<C> AsterVault<C>
where
    C: Clock,
{
    /// Adds an anchor and stamps the stored base row with the same ledger ref.
    pub fn anchor_with_ledger_entry(
        &self,
        id: CxId,
        anchor: Anchor,
        kind: EntryKind,
        subject: SubjectId,
        payload: Vec<u8>,
        actor: ActorId,
    ) -> Result<LedgerRef> {
        let entry = LedgerEntryInput {
            kind,
            subject,
            payload,
            actor,
        };
        anchor.validate_schema()?;
        self.with_durable_commit_lock(|| {
            let latest = self.snapshot();
            let mut constellation = self.get(id, latest)?;
            constellation.anchors.push(anchor.clone());
            let Some(hook) = &self.ledger_hook else {
                return self.anchor_with_raw_ledger_entry(id, &mut constellation, anchor, entry);
            };
            let mut guard = ledger_hook::lock_hook(hook)?;
            let staged = guard.stage_with_checkpoints(
                entry.kind,
                entry.subject,
                entry.payload,
                entry.actor,
            )?;
            let ledger_ref = staged
                .first()
                .ok_or_else(|| CalyxError::ledger_group_commit_failed("no staged ledger rows"))?
                .ledger_ref();
            constellation.provenance = ledger_ref.clone();
            let mut rows = anchor_rows(id, &constellation, &anchor)?;
            rows.extend(staged.iter().map(|row| encode::WriteRow {
                cf: ColumnFamily::Ledger,
                key: row.key().to_vec(),
                value: row.value().to_vec(),
            }));
            self.commit_rows_locked(&rows)?;
            for row in &staged {
                guard.commit_staged(row)?;
            }
            Ok(ledger_ref)
        })
    }

    /// Appends a provenance Ledger entry through Aster's durable group-commit path.
    pub fn append_ledger_entry(
        &self,
        kind: EntryKind,
        subject: SubjectId,
        payload: Vec<u8>,
        actor: ActorId,
    ) -> Result<LedgerRef> {
        self.with_durable_commit_lock(|| {
            let Some(hook) = &self.ledger_hook else {
                return self.append_ledger_entry_without_hook(kind, subject, payload, actor);
            };
            let mut guard = ledger_hook::lock_hook(hook)?;
            let staged = guard.stage_with_checkpoints(kind, subject, payload, actor)?;
            let ledger_ref = staged
                .first()
                .ok_or_else(|| CalyxError::ledger_group_commit_failed("no staged ledger rows"))?
                .ledger_ref();
            let rows = staged
                .iter()
                .map(|row| encode::WriteRow {
                    cf: ColumnFamily::Ledger,
                    key: row.key().to_vec(),
                    value: row.value().to_vec(),
                })
                .collect::<Vec<_>>();
            self.commit_rows_locked(&rows)?;
            for row in &staged {
                guard.commit_staged(row)?;
            }
            Ok(ledger_ref)
        })
    }

    fn append_ledger_entry_without_hook(
        &self,
        kind: EntryKind,
        subject: SubjectId,
        payload: Vec<u8>,
        actor: ActorId,
    ) -> Result<LedgerRef> {
        let store = AsterRawLedgerStore { vault: self };
        let mut appender = LedgerAppender::open(store, SystemClock)?;
        appender.append(kind, subject, payload, actor)
    }

    fn anchor_with_raw_ledger_entry(
        &self,
        id: CxId,
        constellation: &mut calyx_core::Constellation,
        anchor: Anchor,
        entry: LedgerEntryInput,
    ) -> Result<LedgerRef> {
        let store = AsterRawLedgerStore { vault: self };
        let appender = LedgerAppender::open(store, SystemClock)?;
        let prepared = appender.prepare(entry.kind, entry.subject, entry.payload, entry.actor)?;
        let ledger_ref = prepared.ledger_ref();
        constellation.provenance = ledger_ref.clone();
        let mut rows = anchor_rows(id, constellation, &anchor)?;
        rows.push(encode::WriteRow {
            cf: ColumnFamily::Ledger,
            key: ledger_key(prepared.seq()),
            value: prepared.bytes().to_vec(),
        });
        self.commit_rows_locked(&rows)?;
        Ok(ledger_ref)
    }

    pub(crate) fn has_real_ledger_hook(&self) -> bool {
        self.ledger_hook.is_some()
    }

    pub(crate) fn next_ledger_seq_locked(&self) -> Result<u64> {
        let Some(hook) = &self.ledger_hook else {
            let store = AsterRawLedgerStore { vault: self };
            return Ok(LedgerAppender::open(store, SystemClock)?.next_seq());
        };
        let guard = ledger_hook::lock_hook(hook)?;
        Ok(guard.appender().next_seq())
    }

    pub(crate) fn commit_rows_with_ledger_entry_locked(
        &self,
        mut rows: Vec<encode::WriteRow>,
        kind: EntryKind,
        subject: SubjectId,
        payload: Vec<u8>,
        actor: ActorId,
    ) -> Result<LedgerRef> {
        let Some(hook) = &self.ledger_hook else {
            return self.commit_rows_with_raw_ledger_entry(rows, kind, subject, payload, actor);
        };
        let mut guard = ledger_hook::lock_hook(hook)?;
        let staged = guard.stage_with_checkpoints(kind, subject, payload, actor)?;
        let ledger_ref = staged
            .first()
            .ok_or_else(|| CalyxError::ledger_group_commit_failed("no staged ledger rows"))?
            .ledger_ref();
        rows.extend(staged.iter().map(|row| encode::WriteRow {
            cf: ColumnFamily::Ledger,
            key: row.key().to_vec(),
            value: row.value().to_vec(),
        }));
        self.commit_rows_locked(&rows)?;
        for row in &staged {
            guard.commit_staged(row)?;
        }
        Ok(ledger_ref)
    }

    fn commit_rows_with_raw_ledger_entry(
        &self,
        mut rows: Vec<encode::WriteRow>,
        kind: EntryKind,
        subject: SubjectId,
        payload: Vec<u8>,
        actor: ActorId,
    ) -> Result<LedgerRef> {
        let store = AsterRawLedgerStore { vault: self };
        let appender = LedgerAppender::open(store, SystemClock)?;
        let prepared = appender.prepare(kind, subject, payload, actor)?;
        let ledger_ref = prepared.ledger_ref();
        rows.push(encode::WriteRow {
            cf: ColumnFamily::Ledger,
            key: ledger_key(prepared.seq()),
            value: prepared.bytes().to_vec(),
        });
        self.commit_rows_locked(&rows)?;
        Ok(ledger_ref)
    }
}

fn anchor_rows(
    id: CxId,
    constellation: &calyx_core::Constellation,
    anchor: &Anchor,
) -> Result<Vec<encode::WriteRow>> {
    Ok(vec![
        encode::WriteRow {
            cf: ColumnFamily::Base,
            key: base_key(id),
            value: encode::encode_constellation_base(constellation)?,
        },
        encode::WriteRow {
            cf: ColumnFamily::Anchors,
            key: anchor_key(id, &anchor.kind),
            value: encode::encode_anchor(anchor)?,
        },
    ])
}

struct AsterRawLedgerStore<'a, C> {
    vault: &'a AsterVault<C>,
}

impl<C> LedgerCfStore for AsterRawLedgerStore<'_, C>
where
    C: Clock,
{
    fn scan(&self) -> Result<Vec<LedgerRow>> {
        let mut rows = Vec::new();
        for (key, bytes) in self
            .vault
            .scan_cf_at(self.vault.snapshot(), ColumnFamily::Ledger)?
        {
            rows.push(LedgerRow {
                seq: parse_aster_ledger_seq(&key)?,
                bytes,
            });
        }
        rows.sort_by_key(|row| row.seq);
        Ok(rows)
    }

    fn put_new(&mut self, seq: u64, bytes: &[u8]) -> Result<()> {
        let key = ledger_key(seq);
        if self
            .vault
            .read_cf_at(self.vault.snapshot(), ColumnFamily::Ledger, &key)?
            .is_some()
        {
            return Err(CalyxError::ledger_append_only_violation(format!(
                "ledger seq {seq} already exists"
            )));
        }
        self.vault
            .write_cf(ColumnFamily::Ledger, key, bytes.to_vec())
            .map(|_| ())
    }

    fn head_anchor(&self) -> Result<Option<LedgerHeadAnchor>> {
        match &self.vault.durable {
            Some(durable) => crate::ledger_head::read_head_anchor(durable.root()),
            None => Ok(None),
        }
    }

    fn put_head_anchor(&mut self, anchor: &LedgerHeadAnchor) -> Result<()> {
        if let Some(durable) = &self.vault.durable {
            crate::ledger_head::write_head_anchor(durable.root(), anchor)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use calyx_core::{FixedClock, VaultId, VaultStore};
    use calyx_ledger::{RedactionPolicy, decode};

    #[test]
    fn append_ledger_entry_without_hook_writes_real_row() {
        let vault = AsterVault::with_clock(vault_id(), b"ledger-append", FixedClock::new(1));

        let ledger_ref = vault
            .append_ledger_entry(
                EntryKind::Assay,
                SubjectId::Query(vec![1; 16]),
                br#"{"tag":"oracle_self_consistency_v1"}"#.to_vec(),
                ActorId::Service("calyx-oracle".to_string()),
            )
            .expect("append ledger entry");

        let bytes = vault
            .read_cf_at(
                vault.snapshot(),
                ColumnFamily::Ledger,
                &ledger_key(ledger_ref.seq),
            )
            .expect("read ledger")
            .expect("ledger row");
        assert!(RedactionPolicy::check_payload(&decode(&bytes).unwrap().payload).is_ok());
    }

    fn vault_id() -> VaultId {
        "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("vault id")
    }
}
