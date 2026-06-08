use super::durable::RecoveredBatches;
use super::encode::WriteRow;
use crate::cf::ColumnFamily;
use calyx_core::{CalyxError, Constellation, LedgerRef, Result, SystemClock};
use calyx_ledger::{
    ActorId, DefaultLedgerHook, EntryKind, LedgerAppender, LedgerGroupCommitHook, LedgerWriteBatch,
    MemoryLedgerStore, PayloadBuilder, SubjectId,
};
use serde_json::json;
use std::sync::Mutex;

pub(super) type AsterLedgerHook = Mutex<DefaultLedgerHook<MemoryLedgerStore, SystemClock>>;

pub(super) fn recover_hook(recovery: &RecoveredBatches) -> Result<AsterLedgerHook> {
    let mut store = MemoryLedgerStore::default();
    for batch in &recovery.batches {
        for row in &batch.rows {
            if row.cf == ColumnFamily::Ledger {
                store.insert_raw(parse_ledger_seq(&row.key)?, row.value.clone());
            }
        }
    }
    let appender = LedgerAppender::open(store, SystemClock)?;
    Ok(Mutex::new(DefaultLedgerHook::new(appender)))
}

pub(super) fn append_ingest(
    hook: &AsterLedgerHook,
    rows: &mut Vec<WriteRow>,
    constellation: &Constellation,
) -> Result<LedgerRef> {
    let mut batch = AsterBatch { rows };
    let mut hook = hook
        .lock()
        .map_err(|_| CalyxError::ledger_group_commit_failed("ledger hook lock poisoned"))?;
    hook.on_commit(
        &mut batch,
        EntryKind::Ingest,
        SubjectId::Cx(constellation.cx_id),
        ingest_payload(constellation),
        ActorId::Service("calyx-aster".to_string()),
    )
}

struct AsterBatch<'a> {
    rows: &'a mut Vec<WriteRow>,
}

impl LedgerWriteBatch for AsterBatch<'_> {
    fn put_ledger_row(&mut self, key: Vec<u8>, value: Vec<u8>) -> Result<()> {
        self.rows.push(WriteRow {
            cf: ColumnFamily::Ledger,
            key,
            value,
        });
        Ok(())
    }
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
    use calyx_ledger::decode;

    #[test]
    fn aster_batch_uses_big_endian_ledger_keys() {
        let mut rows = Vec::new();
        let mut batch = AsterBatch { rows: &mut rows };

        batch
            .put_ledger_row(ledger_key(7), b"entry".to_vec())
            .expect("put ledger");

        assert_eq!(rows[0].cf, ColumnFamily::Ledger);
        assert_eq!(rows[0].key, ledger_key(7));
        assert_eq!(rows[0].value, b"entry");
    }

    #[test]
    fn recovered_hook_continues_existing_ledger_sequence() {
        let mut rows = Vec::new();
        let mut hook = recover_hook(&RecoveredBatches {
            batches: Vec::new(),
            last_recovered_seq: 0,
            torn_tail: None,
        })
        .expect("recover empty hook");
        let ledger_ref = hook
            .get_mut()
            .unwrap()
            .on_commit(
                &mut AsterBatch { rows: &mut rows },
                EntryKind::Ingest,
                SubjectId::Query(vec![1]),
                b"{}".to_vec(),
                ActorId::Service("test".to_string()),
            )
            .expect("append");

        assert_eq!(ledger_ref.seq, 0);
        assert_eq!(decode(&rows[0].value).unwrap().kind, EntryKind::Ingest);
    }
}
