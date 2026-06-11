use calyx_core::{CalyxError, CxId, FixedClock, Result};
use calyx_ledger::{
    ActorId, CheckpointConfig, CheckpointPayload, DefaultLedgerHook, EntryKind, LedgerAppender,
    LedgerCfStore, LedgerRow, LedgerWriteBatch, MemoryLedgerStore, SubjectId, WriteBatch,
    merkle_root,
};

#[test]
fn scheduler_writes_periodic_admin_checkpoints() {
    let mut hook = checkpoint_hook(CheckpointConfig::new(5));
    let mut batch = WriteBatch::default();

    for seed in 0..15 {
        staged_commit(
            &mut hook,
            &mut batch,
            EntryKind::Ingest,
            sample_subject(seed),
            format!(r#"{{"input_hash":"{seed:064x}"}}"#).into_bytes(),
            ActorId::Service("checkpoint-test".to_string()),
        )
        .expect("commit");
    }

    let entries = hook.appender().scan_entries().expect("scan");
    let checkpoints = checkpoint_entries(&entries);
    assert_eq!(checkpoints.len(), 3);
    assert_eq!(
        checkpoints.iter().map(|(seq, _)| *seq).collect::<Vec<_>>(),
        [5, 11, 17]
    );
    assert_eq!(batch.ledger_rows().len(), 18);
}

#[test]
fn checkpoint_payload_root_matches_direct_merkle_root() {
    let mut hook = checkpoint_hook(CheckpointConfig::new(3));
    let mut batch = WriteBatch::default();

    for seed in 0..3 {
        staged_commit(
            &mut hook,
            &mut batch,
            EntryKind::Ingest,
            sample_subject(seed),
            format!(r#"{{"input_hash":"{seed:064x}"}}"#).into_bytes(),
            ActorId::Service("checkpoint-test".to_string()),
        )
        .expect("commit");
    }

    let entries = hook.appender().scan_entries().expect("scan");
    let checkpoints = checkpoint_entries(&entries);
    assert_eq!(checkpoints.len(), 1);
    let payload = &checkpoints[0].1;
    let expected = merkle_root(
        hook.appender().store(),
        payload.range_start..payload.range_end,
    )
    .expect("direct root");

    assert_eq!(payload.tag, "checkpoint_v1");
    assert_eq!(payload.range_start, 0);
    assert_eq!(payload.range_end, 3);
    assert_eq!(payload.root_bytes().unwrap(), expected);
    assert_ne!(expected, [0; 32]);
    assert_eq!(payload.signature, None);
    assert_eq!(payload.signer_pubkey, None);
}

#[test]
fn checkpoint_edges_cover_every_entry_and_never_fire() {
    let mut every = checkpoint_hook(CheckpointConfig::new(1));
    let mut every_batch = WriteBatch::default();
    for seed in 0..3 {
        staged_commit(
            &mut every,
            &mut every_batch,
            EntryKind::Ingest,
            sample_subject(seed),
            b"{}".to_vec(),
            ActorId::Service("checkpoint-test".to_string()),
        )
        .expect("commit");
    }
    assert_eq!(
        checkpoint_entries(&every.appender().scan_entries().unwrap()).len(),
        3
    );

    let mut never = checkpoint_hook(CheckpointConfig::new(u64::MAX));
    let mut never_batch = WriteBatch::default();
    staged_commit(
        &mut never,
        &mut never_batch,
        EntryKind::Ingest,
        sample_subject(9),
        b"{}".to_vec(),
        ActorId::Service("checkpoint-test".to_string()),
    )
    .expect("commit");
    assert!(checkpoint_entries(&never.appender().scan_entries().unwrap()).is_empty());
}

#[test]
fn signed_checkpoint_payload_has_public_signature_fields() {
    let mut hook = checkpoint_hook(CheckpointConfig::new(2).with_sign_key([42; 32]));
    let mut batch = WriteBatch::default();
    for seed in 0..2 {
        staged_commit(
            &mut hook,
            &mut batch,
            EntryKind::Ingest,
            sample_subject(seed),
            b"{}".to_vec(),
            ActorId::Service("checkpoint-test".to_string()),
        )
        .expect("commit");
    }

    let entries = hook.appender().scan_entries().unwrap();
    let payload = checkpoint_entries(&entries).remove(0).1;

    assert_eq!(payload.signature.as_ref().unwrap().len(), 128);
    assert_eq!(payload.signer_pubkey.as_ref().unwrap().len(), 64);
}

#[test]
fn checkpoint_root_failure_does_not_write_partial_entry() {
    let appender = LedgerAppender::open(MemoryLedgerStore::default(), FixedClock::new(10))
        .expect("open appender");
    let prepared = appender
        .prepare(
            EntryKind::Ingest,
            sample_subject(1),
            b"{}".to_vec(),
            ActorId::Service("checkpoint-test".to_string()),
        )
        .expect("prepare");
    let scheduler = calyx_ledger::CheckpointScheduler::new(CheckpointConfig::new(1)).unwrap();
    let error = scheduler
        .prepare_checkpoint_after(&appender, &FailingStore, &prepared, 1)
        .unwrap_err();

    assert_eq!(error.code, "CALYX_DISK_PRESSURE");
    assert!(appender.store().scan().unwrap().is_empty());
}

fn checkpoint_hook(config: CheckpointConfig) -> DefaultLedgerHook<MemoryLedgerStore, FixedClock> {
    let appender = LedgerAppender::open(MemoryLedgerStore::default(), FixedClock::new(10))
        .expect("open appender");
    DefaultLedgerHook::with_checkpoint_config(appender, config).expect("checkpoint hook")
}

fn staged_commit(
    hook: &mut DefaultLedgerHook<MemoryLedgerStore, FixedClock>,
    batch: &mut WriteBatch,
    kind: EntryKind,
    subject: SubjectId,
    payload: Vec<u8>,
    actor: ActorId,
) -> Result<()> {
    let staged = hook.stage_with_checkpoints(kind, subject, payload, actor)?;
    for row in &staged {
        batch.put_ledger_row(row.key().to_vec(), row.value().to_vec())?;
    }
    for row in &staged {
        hook.commit_staged(row)?;
    }
    Ok(())
}

fn sample_subject(seed: u8) -> SubjectId {
    SubjectId::Cx(CxId::from_bytes([seed; 16]))
}

fn checkpoint_entries(entries: &[calyx_ledger::LedgerEntry]) -> Vec<(u64, CheckpointPayload)> {
    entries
        .iter()
        .filter(|entry| entry.kind == EntryKind::Admin)
        .map(|entry| {
            (
                entry.seq,
                CheckpointPayload::decode(&entry.payload).expect("checkpoint payload"),
            )
        })
        .collect()
}

struct FailingStore;

impl LedgerCfStore for FailingStore {
    fn scan(&self) -> Result<Vec<LedgerRow>> {
        Err(CalyxError::disk_pressure(
            "synthetic checkpoint root read failure",
        ))
    }

    fn put_new(&mut self, seq: u64, _bytes: &[u8]) -> Result<()> {
        Err(CalyxError::ledger_append_only_violation(format!(
            "read-only failing store for seq {seq}"
        )))
    }
}
