use super::{AsterVault, encode, ledger_hook};
use crate::cf::{ColumnFamily, ledger_key};
use crate::ledger_view::parse_aster_ledger_seq;
use calyx_core::{CalyxError, Clock, LedgerRef, Result, SystemClock, VaultStore};
use calyx_ledger::{ActorId, EntryKind, LedgerAppender, LedgerCfStore, LedgerRow, SubjectId};

impl<C> AsterVault<C>
where
    C: Clock,
{
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
