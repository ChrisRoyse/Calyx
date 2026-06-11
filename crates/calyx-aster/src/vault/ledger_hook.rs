use super::durable::RecoveredBatches;
use super::encode::WriteRow;
use crate::cf::ColumnFamily;
use calyx_core::{CalyxError, Constellation, LedgerRef, Result, SystemClock};
use calyx_ledger::{
    ActorId, CheckpointConfig, DefaultLedgerHook, EntryKind, LedgerAppender, MemoryLedgerStore,
    PayloadBuilder, StagedLedgerRow, SubjectId,
};
use serde_json::json;
use std::sync::{Mutex, MutexGuard};

pub(super) type AsterLedgerHook = Mutex<DefaultLedgerHook<MemoryLedgerStore, SystemClock>>;
pub(super) type AsterLedgerHookGuard<'a> =
    MutexGuard<'a, DefaultLedgerHook<MemoryLedgerStore, SystemClock>>;

pub(super) fn recover_hook(
    recovery: &RecoveredBatches,
    checkpoint: Option<CheckpointConfig>,
) -> Result<AsterLedgerHook> {
    let mut store = MemoryLedgerStore::default();
    for batch in &recovery.batches {
        for row in &batch.rows {
            if row.cf == ColumnFamily::Ledger {
                store.insert_raw(parse_ledger_seq(&row.key)?, row.value.clone());
            }
        }
    }
    let appender = LedgerAppender::open(store, SystemClock)?;
    let hook = match checkpoint {
        Some(config) => DefaultLedgerHook::with_checkpoint_config(appender, config)?,
        None => DefaultLedgerHook::new(appender),
    };
    Ok(Mutex::new(hook))
}

pub(super) fn lock_hook(hook: &AsterLedgerHook) -> Result<AsterLedgerHookGuard<'_>> {
    hook.lock()
        .map_err(|_| CalyxError::ledger_group_commit_failed("ledger hook lock poisoned"))
}

pub(super) fn refresh_hook(
    hook: &AsterLedgerHook,
    recovery: &RecoveredBatches,
    checkpoint: Option<CheckpointConfig>,
) -> Result<()> {
    let replacement = recover_hook(recovery, checkpoint)?
        .into_inner()
        .map_err(|_| CalyxError::ledger_group_commit_failed("new ledger hook lock poisoned"))?;
    let mut guard = lock_hook(hook)?;
    *guard = replacement;
    Ok(())
}

pub(super) fn stage_ingest(
    hook: &DefaultLedgerHook<MemoryLedgerStore, SystemClock>,
    rows: &mut Vec<WriteRow>,
    constellation: &Constellation,
) -> Result<Vec<StagedLedgerRow>> {
    stage_ingest_payload(
        hook,
        rows,
        constellation.cx_id,
        ingest_payload(constellation),
    )
}

pub(super) fn stage_ingest_payload(
    hook: &DefaultLedgerHook<MemoryLedgerStore, SystemClock>,
    rows: &mut Vec<WriteRow>,
    subject: calyx_core::CxId,
    payload: Vec<u8>,
) -> Result<Vec<StagedLedgerRow>> {
    let staged = hook.stage_with_checkpoints(
        EntryKind::Ingest,
        SubjectId::Cx(subject),
        payload,
        ActorId::Service("calyx-aster".to_string()),
    )?;
    for row in &staged {
        rows.push(WriteRow {
            cf: ColumnFamily::Ledger,
            key: row.key().to_vec(),
            value: row.value().to_vec(),
        });
    }
    Ok(staged)
}

pub(super) fn commit_staged(
    hook: &mut DefaultLedgerHook<MemoryLedgerStore, SystemClock>,
    staged: &[StagedLedgerRow],
) -> Result<LedgerRef> {
    let data_ref = staged
        .first()
        .ok_or_else(|| CalyxError::ledger_group_commit_failed("no staged ledger rows"))?
        .ledger_ref();
    for row in staged {
        hook.commit_staged(row)?;
    }
    Ok(data_ref)
}

fn ingest_payload(constellation: &Constellation) -> Vec<u8> {
    let mut payload = PayloadBuilder::default();
    payload
        .insert_str("cx_id", constellation.cx_id.to_string())
        .insert_str("input_hash", hex(&constellation.input_ref.hash))
        .insert_value(
            "input_ref",
            json!({
                "hash": constellation.input_ref.hash,
                "redacted": true,
            }),
        )
        .insert_u64("ts", constellation.created_at);
    calyx_ledger::RedactionPolicy::default().apply_to_payload(&payload)
}

fn parse_ledger_seq(key: &[u8]) -> Result<u64> {
    let bytes: [u8; 8] = key
        .try_into()
        .map_err(|_| CalyxError::ledger_corrupt(format!("ledger key length {} != 8", key.len())))?;
    Ok(u64::from_be_bytes(bytes))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cf::ledger_key;
    use calyx_ledger::{LedgerCfStore, decode};
    use std::collections::BTreeMap;

    #[test]
    fn aster_batch_uses_big_endian_ledger_keys() {
        let rows = [WriteRow {
            cf: ColumnFamily::Ledger,
            key: ledger_key(7),
            value: b"entry".to_vec(),
        }];

        assert_eq!(rows[0].cf, ColumnFamily::Ledger);
        assert_eq!(rows[0].key, ledger_key(7));
        assert_eq!(rows[0].value, b"entry");
    }

    #[test]
    fn recovered_hook_continues_existing_ledger_sequence() {
        let mut rows = Vec::new();
        let mut hook = recover_hook(
            &RecoveredBatches {
                batches: Vec::new(),
                last_recovered_seq: 0,
                torn_tail: None,
                temporal_policy: None,
                dedup_policy: None,
            },
            None,
        )
        .expect("recover empty hook");
        let guard = hook.get_mut().unwrap();
        let first = stage_ingest(guard, &mut rows, &sample_constellation()).expect("stage first");

        assert_eq!(first[0].ledger_ref().seq, 0);
        assert_eq!(guard.appender().next_seq(), 0);
        assert!(guard.appender().store().scan().unwrap().is_empty());
        assert_eq!(decode(&rows[0].value).unwrap().kind, EntryKind::Ingest);

        let committed = commit_staged(guard, &first).expect("commit first");

        assert_eq!(committed.seq, 0);
        assert_eq!(guard.appender().next_seq(), 1);
        assert_eq!(guard.appender().store().scan().unwrap().len(), 1);
    }

    fn sample_constellation() -> Constellation {
        Constellation {
            cx_id: calyx_core::CxId::from_bytes([7; 16]),
            vault_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap(),
            panel_version: 1,
            created_at: 42,
            input_ref: calyx_core::InputRef {
                hash: [3; 32],
                pointer: Some("synthetic://ledger-hook".to_string()),
                redacted: false,
            },
            modality: calyx_core::Modality::Text,
            slots: BTreeMap::new(),
            scalars: BTreeMap::new(),
            metadata: BTreeMap::new(),
            anchors: Vec::new(),
            provenance: LedgerRef {
                seq: 99,
                hash: [9; 32],
            },
            flags: calyx_core::CxFlags::default(),
        }
    }
}
