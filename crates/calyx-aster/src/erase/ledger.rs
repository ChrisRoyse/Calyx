use crate::cf::ColumnFamily;
use crate::ledger_view::parse_aster_ledger_seq;
use crate::vault::AsterVault;
use calyx_core::{CalyxError, Clock, Result, Ts};
use calyx_ledger::{
    ActorId, ErasureScope as LedgerErasureScope, ErasureTombstone, LedgerCfStore, LedgerRow,
    SubjectId, find_tombstone,
};

use super::EraseScope;

const ERASURE_ACTOR: &str = "calyx-aster";

pub(super) fn existing_tombstone<C>(
    vault: &AsterVault<C>,
    scope: &EraseScope,
    snapshot: u64,
) -> Result<Option<ErasureTombstone>>
where
    C: Clock,
{
    let ledger = ledger_snapshot(vault, snapshot)?;
    find_tombstone(vault.vault_id(), &ledger_scope(scope), &ledger)
}

pub(super) fn tombstone_for<C>(
    vault: &AsterVault<C>,
    scope: &EraseScope,
    records_deleted: usize,
    erased_at: Ts,
) -> Result<ErasureTombstone>
where
    C: Clock,
{
    Ok(ErasureTombstone {
        seq: vault.next_ledger_seq_locked()?,
        vault_id: vault.vault_id(),
        scope: ledger_scope(scope),
        actor: ActorId::Service(ERASURE_ACTOR.to_string()),
        erased_at,
        records_deleted,
    })
}

fn ledger_scope(scope: &EraseScope) -> LedgerErasureScope {
    match scope {
        EraseScope::Vault => LedgerErasureScope::Vault,
        EraseScope::Cx(id) => LedgerErasureScope::Cx(*id),
        EraseScope::Subject(subject) => LedgerErasureScope::Subject(subject.clone()),
    }
}

fn ledger_snapshot<C>(vault: &AsterVault<C>, snapshot: u64) -> Result<LedgerSnapshot>
where
    C: Clock,
{
    let mut rows = Vec::new();
    for (key, bytes) in vault.scan_cf_at(snapshot, ColumnFamily::Ledger)? {
        rows.push(LedgerRow {
            seq: parse_aster_ledger_seq(&key)?,
            bytes,
        });
    }
    rows.sort_by_key(|row| row.seq);
    Ok(LedgerSnapshot { rows })
}

struct LedgerSnapshot {
    rows: Vec<LedgerRow>,
}

impl LedgerCfStore for LedgerSnapshot {
    fn scan(&self) -> Result<Vec<LedgerRow>> {
        Ok(self.rows.clone())
    }

    fn put_new(&mut self, seq: u64, _bytes: &[u8]) -> Result<()> {
        Err(CalyxError::ledger_append_only_violation(format!(
            "erase ledger snapshot rejected append for seq {seq}"
        )))
    }
}

pub(super) fn tombstone_subject(tombstone: &ErasureTombstone) -> SubjectId {
    tombstone.ledger_subject()
}
