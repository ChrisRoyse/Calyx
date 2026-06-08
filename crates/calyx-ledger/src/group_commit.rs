//! Group-commit hook for adding Ledger rows to a storage write batch.

use calyx_core::{CalyxError, Clock, LedgerRef, Result};

use crate::append::{LedgerAppender, LedgerCfStore, MemoryLedgerStore};
use crate::entry::{ActorId, SubjectId};
use crate::kind::EntryKind;

/// Storage batch surface required by the Ledger group-commit hook.
pub trait LedgerWriteBatch {
    fn put_ledger_row(&mut self, key: Vec<u8>, value: Vec<u8>) -> Result<()>;
}

/// Hook called before a storage batch is durably committed.
pub trait LedgerGroupCommitHook: Send + Sync {
    fn on_commit(
        &mut self,
        batch: &mut dyn LedgerWriteBatch,
        kind: EntryKind,
        subject: SubjectId,
        payload: Vec<u8>,
        actor: ActorId,
    ) -> Result<LedgerRef>;
}

/// In-memory batch used by unit tests and adapters.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WriteBatch {
    ledger_rows: Vec<LedgerBatchRow>,
}

impl WriteBatch {
    pub fn ledger_rows(&self) -> &[LedgerBatchRow] {
        &self.ledger_rows
    }
}

impl LedgerWriteBatch for WriteBatch {
    fn put_ledger_row(&mut self, key: Vec<u8>, value: Vec<u8>) -> Result<()> {
        self.ledger_rows.push(LedgerBatchRow { key, value });
        Ok(())
    }
}

/// One ledger row staged into a group-commit batch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LedgerBatchRow {
    pub key: Vec<u8>,
    pub value: Vec<u8>,
}

/// Default hook backed by a `LedgerAppender`.
#[derive(Debug)]
pub struct DefaultLedgerHook<S = MemoryLedgerStore, C = calyx_core::SystemClock> {
    appender: LedgerAppender<S, C>,
}

impl<S, C> DefaultLedgerHook<S, C>
where
    S: LedgerCfStore,
    C: Clock,
{
    pub const fn new(appender: LedgerAppender<S, C>) -> Self {
        Self { appender }
    }

    pub const fn appender(&self) -> &LedgerAppender<S, C> {
        &self.appender
    }
}

impl<S, C> LedgerGroupCommitHook for DefaultLedgerHook<S, C>
where
    S: LedgerCfStore + Send + Sync,
    C: Clock + Send + Sync,
{
    fn on_commit(
        &mut self,
        batch: &mut dyn LedgerWriteBatch,
        kind: EntryKind,
        subject: SubjectId,
        payload: Vec<u8>,
        actor: ActorId,
    ) -> Result<LedgerRef> {
        let ledger_ref = self
            .appender
            .append(kind, subject, payload, actor)
            .map_err(group_commit_failed)?;
        let bytes = row_bytes(self.appender.store(), ledger_ref.seq)?;
        batch
            .put_ledger_row(ledger_batch_key(ledger_ref.seq), bytes)
            .map_err(group_commit_failed)?;
        Ok(ledger_ref)
    }
}

/// Big-endian ledger CF key; must match Aster `ledger_key`.
pub fn ledger_batch_key(seq: u64) -> Vec<u8> {
    seq.to_be_bytes().to_vec()
}

/// Storage operation categories mapped to ledger entry kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteOp {
    Ingest,
    VaultAdmin,
    Erase,
}

pub const fn ingest_kind_for(op: WriteOp) -> EntryKind {
    match op {
        WriteOp::Ingest => EntryKind::Ingest,
        WriteOp::VaultAdmin => EntryKind::Admin,
        WriteOp::Erase => EntryKind::Erase,
    }
}

fn row_bytes(store: &impl LedgerCfStore, seq: u64) -> Result<Vec<u8>> {
    store
        .scan()
        .map_err(group_commit_failed)?
        .into_iter()
        .find(|row| row.seq == seq)
        .map(|row| row.bytes)
        .ok_or_else(|| group_commit_failed("ledger appender did not expose appended row"))
}

fn group_commit_failed(message: impl ToString) -> CalyxError {
    CalyxError::ledger_group_commit_failed(message.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::decode;
    use calyx_core::{CxId, FixedClock};

    #[test]
    fn default_hook_adds_one_ledger_row_to_batch() {
        let mut hook = sample_hook();
        let mut batch = WriteBatch::default();

        let ledger_ref = hook
            .on_commit(
                &mut batch,
                EntryKind::Ingest,
                sample_subject(1),
                b"{}".to_vec(),
                sample_actor(),
            )
            .expect("hook append");

        assert_eq!(ledger_ref.seq, 0);
        assert_eq!(batch.ledger_rows().len(), 1);
        assert_eq!(batch.ledger_rows()[0].key, ledger_batch_key(0));
        let entry = decode(&batch.ledger_rows()[0].value).expect("decode ledger entry");
        assert_eq!(entry.seq, 0);
        assert_eq!(entry.kind, EntryKind::Ingest);
        assert_eq!(entry.prev_hash, [0; 32]);
    }

    #[test]
    fn sequential_hook_calls_stage_ordered_ledger_keys() {
        let mut hook = sample_hook();
        let mut batch = WriteBatch::default();

        for index in 0..3 {
            hook.on_commit(
                &mut batch,
                EntryKind::Measure,
                sample_subject(index),
                format!(r#"{{"input_hash":"{index:064x}"}}"#).into_bytes(),
                sample_actor(),
            )
            .expect("hook append");
        }

        let keys = batch
            .ledger_rows()
            .iter()
            .map(|row| row.key.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            keys,
            vec![
                ledger_batch_key(0),
                ledger_batch_key(1),
                ledger_batch_key(2)
            ]
        );
    }

    #[test]
    fn hook_edges_cover_empty_payload_redaction_and_erase_kind() {
        let mut hook = sample_hook();
        let mut batch = WriteBatch::default();

        hook.on_commit(
            &mut batch,
            EntryKind::Admin,
            sample_subject(7),
            Vec::new(),
            sample_actor(),
        )
        .expect("empty payload accepted");
        hook.on_commit(
            &mut batch,
            EntryKind::Erase,
            sample_subject(8),
            br#"{"input_hash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#
                .to_vec(),
            sample_actor(),
        )
        .expect("hash-only payload accepted");

        let erase = decode(&batch.ledger_rows()[1].value).expect("decode erase");
        assert_eq!(erase.kind, ingest_kind_for(WriteOp::Erase));
    }

    #[test]
    fn batch_failure_returns_group_commit_error() {
        let mut hook = sample_hook();
        let mut batch = FailingBatch;

        let error = hook
            .on_commit(
                &mut batch,
                EntryKind::Ingest,
                sample_subject(1),
                b"{}".to_vec(),
                sample_actor(),
            )
            .unwrap_err();

        assert_eq!(error.code, "CALYX_LEDGER_GROUP_COMMIT_FAILED");
    }

    fn sample_hook() -> DefaultLedgerHook<MemoryLedgerStore, FixedClock> {
        DefaultLedgerHook::new(
            LedgerAppender::open(MemoryLedgerStore::default(), FixedClock::new(44))
                .expect("open appender"),
        )
    }

    fn sample_subject(seed: u8) -> SubjectId {
        SubjectId::Cx(CxId::from_bytes([seed; 16]))
    }

    fn sample_actor() -> ActorId {
        ActorId::Service("ledger-hook-test".to_string())
    }

    struct FailingBatch;

    impl LedgerWriteBatch for FailingBatch {
        fn put_ledger_row(&mut self, _key: Vec<u8>, _value: Vec<u8>) -> Result<()> {
            Err(CalyxError::disk_pressure("synthetic batch write failure"))
        }
    }
}
