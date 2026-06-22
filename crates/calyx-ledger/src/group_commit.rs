//! Group-commit hook for adding Ledger rows to a storage write batch.

use calyx_core::{CalyxError, Clock, LedgerRef, Result};

use crate::append::{LedgerAppender, LedgerCfStore, MemoryLedgerStore, PreparedLedgerEntry};
use crate::checkpoint::{CheckpointConfig, CheckpointScheduler, OverlayLedgerStore};
use crate::entry::{ActorId, SubjectId};
use crate::kind::EntryKind;

/// Storage batch surface required by the Ledger group-commit hook.
pub trait LedgerWriteBatch {
    fn put_ledger_row(&mut self, key: Vec<u8>, value: Vec<u8>) -> Result<()>;
}

/// Legacy direct hook surface kept crate-private so storage engines cannot
/// advance Ledger state outside their durable staged commit path.
#[allow(dead_code)] // #652: retained only to fail closed if crate-local legacy code calls it.
pub(crate) trait LedgerGroupCommitHook: Send + Sync {
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

/// One ledger row prepared for a storage batch but not yet committed to the appender tip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StagedLedgerRow {
    key: Vec<u8>,
    value: Vec<u8>,
    ledger_ref: LedgerRef,
    prepared: PreparedLedgerEntry,
    checkpoint_range_end: Option<u64>,
}

impl StagedLedgerRow {
    pub fn key(&self) -> &[u8] {
        &self.key
    }

    pub fn value(&self) -> &[u8] {
        &self.value
    }

    pub fn ledger_ref(&self) -> LedgerRef {
        self.ledger_ref.clone()
    }
}

/// Default hook backed by a `LedgerAppender`.
#[derive(Debug)]
pub struct DefaultLedgerHook<S = MemoryLedgerStore, C = calyx_core::SystemClock> {
    appender: LedgerAppender<S, C>,
    checkpoint: Option<CheckpointScheduler>,
}

impl<S, C> DefaultLedgerHook<S, C>
where
    S: LedgerCfStore,
    C: Clock,
{
    pub const fn new(appender: LedgerAppender<S, C>) -> Self {
        Self {
            appender,
            checkpoint: None,
        }
    }

    pub fn with_checkpoint_config(
        appender: LedgerAppender<S, C>,
        config: CheckpointConfig,
    ) -> Result<Self> {
        let checkpoint = CheckpointScheduler::recover(config, appender.store())?;
        Ok(Self {
            appender,
            checkpoint: Some(checkpoint),
        })
    }

    pub const fn with_checkpoint_scheduler(
        appender: LedgerAppender<S, C>,
        checkpoint: CheckpointScheduler,
    ) -> Self {
        Self {
            appender,
            checkpoint: Some(checkpoint),
        }
    }

    pub const fn appender(&self) -> &LedgerAppender<S, C> {
        &self.appender
    }

    pub const fn checkpoint(&self) -> Option<&CheckpointScheduler> {
        self.checkpoint.as_ref()
    }

    pub fn stage(
        &self,
        kind: EntryKind,
        subject: SubjectId,
        payload: Vec<u8>,
        actor: ActorId,
    ) -> Result<StagedLedgerRow> {
        let prepared = self
            .appender
            .prepare(kind, subject, payload, actor)
            .map_err(group_commit_failed)?;
        Ok(StagedLedgerRow {
            key: ledger_batch_key(prepared.seq()),
            value: prepared.bytes().to_vec(),
            ledger_ref: prepared.ledger_ref(),
            prepared,
            checkpoint_range_end: None,
        })
    }

    pub fn stage_with_checkpoints(
        &self,
        kind: EntryKind,
        subject: SubjectId,
        payload: Vec<u8>,
        actor: ActorId,
    ) -> Result<Vec<StagedLedgerRow>> {
        let first = self.stage(kind, subject, payload, actor)?;
        let range_end = first
            .ledger_ref
            .seq
            .checked_add(1)
            .ok_or_else(|| CalyxError::ledger_chain_broken("ledger sequence exhausted"))?;
        let mut staged = vec![first];

        if let Some(checkpoint) = &self.checkpoint
            && checkpoint.should_checkpoint(range_end)
        {
            let overlay = OverlayLedgerStore::new(
                self.appender.store(),
                staged.iter().map(|row| row.prepared.clone()),
            )?;
            let predecessor = &staged.last().expect("staged data row").prepared;
            let prepared = checkpoint.prepare_checkpoint_after(
                &self.appender,
                &overlay,
                predecessor,
                range_end,
            )?;
            staged.push(StagedLedgerRow {
                key: ledger_batch_key(prepared.seq()),
                value: prepared.bytes().to_vec(),
                ledger_ref: prepared.ledger_ref(),
                prepared,
                checkpoint_range_end: Some(range_end),
            });
        }

        Ok(staged)
    }

    pub fn commit_staged(&mut self, staged: &StagedLedgerRow) -> Result<LedgerRef> {
        let ledger_ref = self
            .appender
            .commit_prepared(&staged.prepared)
            .map_err(group_commit_failed)?;
        if let (Some(checkpoint), Some(range_end)) =
            (self.checkpoint.as_mut(), staged.checkpoint_range_end)
        {
            checkpoint
                .advance_after_checkpoint(range_end)
                .map_err(group_commit_failed)?;
        }
        Ok(ledger_ref)
    }
}

impl<S, C> LedgerGroupCommitHook for DefaultLedgerHook<S, C>
where
    S: LedgerCfStore + Send + Sync,
    C: Clock + Send + Sync,
{
    fn on_commit(
        &mut self,
        _batch: &mut dyn LedgerWriteBatch,
        _kind: EntryKind,
        _subject: SubjectId,
        _payload: Vec<u8>,
        _actor: ActorId,
    ) -> Result<LedgerRef> {
        Err(group_commit_failed(
            "direct LedgerGroupCommitHook::on_commit is disabled; use \
             stage_with_checkpoints and commit_staged after durable storage commit",
        ))
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

fn group_commit_failed(message: impl ToString) -> CalyxError {
    CalyxError::ledger_group_commit_failed(message.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::append::DirectoryLedgerStore;
    use crate::codec::decode;
    use calyx_core::{CxId, FixedClock};
    use std::fs;

    #[test]
    fn staged_commit_adds_one_ledger_row_to_batch() {
        let mut hook = sample_hook();
        let mut batch = WriteBatch::default();

        let ledger_ref = stage_commit(
            &mut hook,
            &mut batch,
            EntryKind::Ingest,
            sample_subject(1),
            b"{}".to_vec(),
            sample_actor(),
        )
        .expect("staged commit");

        assert_eq!(ledger_ref.seq, 0);
        assert_eq!(batch.ledger_rows().len(), 1);
        assert_eq!(batch.ledger_rows()[0].key, ledger_batch_key(0));
        let entry = decode(&batch.ledger_rows()[0].value).expect("decode ledger entry");
        assert_eq!(entry.seq, 0);
        assert_eq!(entry.kind, EntryKind::Ingest);
        assert_eq!(entry.prev_hash, [0; 32]);
    }

    #[test]
    fn sequential_staged_commits_stage_ordered_ledger_keys() {
        let mut hook = sample_hook();
        let mut batch = WriteBatch::default();

        for index in 0..3 {
            stage_commit(
                &mut hook,
                &mut batch,
                EntryKind::Measure,
                sample_subject(index),
                format!(r#"{{"input_hash":"{index:064x}"}}"#).into_bytes(),
                sample_actor(),
            )
            .expect("staged commit");
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

        stage_commit(
            &mut hook,
            &mut batch,
            EntryKind::Admin,
            sample_subject(7),
            Vec::new(),
            sample_actor(),
        )
        .expect("empty payload accepted");
        stage_commit(
            &mut hook,
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
    fn staged_row_does_not_advance_until_commit() {
        let mut hook = sample_hook();
        let mut batch = WriteBatch::default();

        let staged = hook
            .stage(
                EntryKind::Ingest,
                sample_subject(9),
                b"{}".to_vec(),
                sample_actor(),
            )
            .expect("stage row");

        assert_eq!(staged.ledger_ref().seq, 0);
        assert_eq!(hook.appender().next_seq(), 0);
        assert_eq!(hook.appender().prev_hash(), [0; 32]);
        assert!(hook.appender().store().scan().unwrap().is_empty());

        batch
            .put_ledger_row(staged.key().to_vec(), staged.value().to_vec())
            .expect("put staged row");
        let committed = hook.commit_staged(&staged).expect("commit staged");

        assert_eq!(committed, staged.ledger_ref());
        assert_eq!(hook.appender().next_seq(), 1);
        assert_eq!(hook.appender().store().scan().unwrap().len(), 1);
    }

    #[test]
    fn direct_on_commit_fails_closed_without_batch_or_tip_mutation() {
        let mut hook = sample_hook();
        let mut batch = WriteBatch::default();

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
        assert!(error.message.contains("direct LedgerGroupCommitHook"));
        assert!(batch.ledger_rows().is_empty());
        assert_eq!(hook.appender().next_seq(), 0);
        assert_eq!(hook.appender().prev_hash(), [0; 32]);
        assert!(hook.appender().store().scan().unwrap().is_empty());
    }

    #[test]
    #[ignore = "manual FSV for #652 direct group-commit misuse"]
    fn ph35_direct_on_commit_rejects_manual_fsv() {
        let root = std::env::var("CALYX_FSV_ROOT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("calyx-ph35-direct-on-commit-fsv"))
            .join("direct-on-commit-reject");
        let ledger_dir = root.join("ledger-cf");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&ledger_dir).expect("create FSV ledger dir");
        let store = DirectoryLedgerStore::open(&ledger_dir).expect("open directory store");
        let mut hook = DefaultLedgerHook::new(
            LedgerAppender::open(store, FixedClock::new(652)).expect("open appender"),
        );
        let mut batch = WriteBatch::default();

        let before_files = fs::read_dir(&ledger_dir).unwrap().count();
        let before_next_seq = hook.appender().next_seq();
        let before_prev_hash = hook.appender().prev_hash();
        let error = hook
            .on_commit(
                &mut batch,
                EntryKind::Ingest,
                sample_subject(6),
                b"{\"input_hash\":\"issue652\"}".to_vec(),
                sample_actor(),
            )
            .unwrap_err();
        let after_files = fs::read_dir(&ledger_dir).unwrap().count();
        let after_store_rows = hook.appender().store().scan().unwrap();

        let readback = serde_json::json!({
            "issue": 652,
            "ledger_dir": ledger_dir.display().to_string(),
            "before_ledger_file_count": before_files,
            "before_next_seq": before_next_seq,
            "before_prev_hash": hex(&before_prev_hash),
            "after_error_code": error.code,
            "after_error_message": error.message,
            "after_ledger_file_count": after_files,
            "after_store_rows": after_store_rows.len(),
            "after_next_seq": hook.appender().next_seq(),
            "after_prev_hash": hex(&hook.appender().prev_hash()),
            "after_batch_rows": batch.ledger_rows().len(),
        });
        let readback_path = root.join("direct-on-commit-readback.json");
        fs::write(
            &readback_path,
            serde_json::to_vec_pretty(&readback).unwrap(),
        )
        .unwrap();

        println!("PH35_DIRECT_ON_COMMIT_FSV_ROOT={}", root.display());
        println!("PH35_DIRECT_ON_COMMIT_READBACK={}", readback_path.display());
        println!("{}", serde_json::to_string_pretty(&readback).unwrap());

        assert_eq!(error.code, "CALYX_LEDGER_GROUP_COMMIT_FAILED");
        assert!(batch.ledger_rows().is_empty());
        assert_eq!(hook.appender().next_seq(), before_next_seq);
        assert_eq!(hook.appender().prev_hash(), before_prev_hash);
        assert!(after_store_rows.is_empty());
        assert_eq!(fs::read_dir(&ledger_dir).unwrap().count(), 0);
    }

    fn stage_commit(
        hook: &mut DefaultLedgerHook<MemoryLedgerStore, FixedClock>,
        batch: &mut WriteBatch,
        kind: EntryKind,
        subject: SubjectId,
        payload: Vec<u8>,
        actor: ActorId,
    ) -> Result<LedgerRef> {
        let staged = hook.stage_with_checkpoints(kind, subject, payload, actor)?;
        let data_ref = staged
            .first()
            .ok_or_else(|| group_commit_failed("no staged ledger rows"))?
            .ledger_ref();
        for row in &staged {
            batch.put_ledger_row(row.key().to_vec(), row.value().to_vec())?;
        }
        for row in &staged {
            hook.commit_staged(row)?;
        }
        Ok(data_ref)
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

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}
