use super::*;
use crate::{LedgerAppender, MemoryLedgerStore, RedactionPolicy};
use calyx_core::FixedClock;

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn tombstone(seq: u64) -> ErasureTombstone {
    ErasureTombstone {
        seq,
        vault_id: vault_id(),
        scope: ErasureScope::Cx(CxId::from_bytes([1; 16])),
        actor: ActorId::Service("calyx-aster".to_string()),
        erased_at: 777,
        records_deleted: 1,
    }
}

fn subject_tombstone() -> ErasureTombstone {
    ErasureTombstone {
        seq: 7,
        vault_id: vault_id(),
        scope: ErasureScope::Subject(SubjectId::Query(vec![2; 32])),
        actor: ActorId::Service("calyx-aster".to_string()),
        erased_at: 888,
        records_deleted: 2,
    }
}

#[test]
fn erasure_tombstone_payload_roundtrips_without_content() {
    let tombstone = tombstone(0);
    let payload = tombstone.as_ledger_payload();

    assert!(payload.len() < 128);
    assert_eq!(
        ErasureTombstone::from_ledger_payload(&payload).unwrap(),
        tombstone
    );
    assert!(RedactionPolicy::check_payload(&payload).is_ok());
    assert!(!payload.windows(4).any(|window| window == b"raw-"));
}

#[test]
fn subject_tombstone_payload_stays_under_128_bytes() {
    let tombstone = subject_tombstone();
    let payload = tombstone.as_ledger_payload();

    assert!(payload.len() < 128, "payload was {} bytes", payload.len());
    assert_eq!(
        ErasureTombstone::from_ledger_payload(&payload).unwrap(),
        tombstone
    );
    assert_eq!(tombstone.as_json_value()["n"], serde_json::json!(2));
}

#[test]
fn write_tombstone_appends_erase_entry() {
    let mut ledger =
        LedgerAppender::open(MemoryLedgerStore::default(), FixedClock::new(10)).expect("open");
    let ledger_ref = write_tombstone(&tombstone(0), &mut ledger).expect("write tombstone");
    let entries = ledger.scan_entries().expect("scan entries");

    assert_eq!(ledger_ref.seq, 0);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].kind, EntryKind::Erase);
    assert_eq!(
        tombstone_from_entry(&entries[0])
            .unwrap()
            .unwrap()
            .records_deleted,
        1
    );
}

#[test]
fn is_tombstoned_reads_matching_erase_entry() {
    let mut ledger =
        LedgerAppender::open(MemoryLedgerStore::default(), FixedClock::new(10)).expect("open");
    let scope = ErasureScope::Cx(CxId::from_bytes([1; 16]));

    assert!(!is_tombstoned(vault_id(), &scope, ledger.store()).unwrap());
    write_tombstone(&tombstone(0), &mut ledger).expect("write tombstone");
    assert!(is_tombstoned(vault_id(), &scope, ledger.store()).unwrap());
}

#[test]
fn append_only_store_rejects_tombstone_deletes() {
    let mut store = MemoryLedgerStore::default();
    assert_eq!(
        LedgerCfStore::tombstone(&mut store, 0).unwrap_err().code,
        "CALYX_LEDGER_APPEND_ONLY_VIOLATION"
    );
}

#[test]
fn write_tombstone_failure_surfaces_ledger_error() {
    let mut ledger = LedgerAppender::open(FailingStore, FixedClock::new(10)).expect("open ledger");
    let error = write_tombstone(&tombstone(0), &mut ledger).unwrap_err();

    assert_eq!(error.code, "CALYX_LEDGER_GROUP_COMMIT_FAILED");
}

struct FailingStore;

impl LedgerCfStore for FailingStore {
    fn scan(&self) -> Result<Vec<crate::LedgerRow>> {
        Ok(Vec::new())
    }

    fn put_new(&mut self, _seq: u64, _bytes: &[u8]) -> Result<()> {
        Err(CalyxError::ledger_group_commit_failed(
            "synthetic tombstone append failure",
        ))
    }
}
