use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::{CxId, FixedClock};
use calyx_ledger::{
    ActorId, DirectoryLedgerStore, EntryKind, LedgerAppender, LedgerCfStore, LedgerEntry,
    LedgerRow, MemoryLedgerStore, SubjectId, decode, encode,
};
use proptest::prelude::*;
use serde_json::json;

#[test]
fn appender_appends_and_recovers_chain() {
    let mut appender = LedgerAppender::open(MemoryLedgerStore::default(), FixedClock::new(99))
        .expect("open empty appender");

    let refs = append_sample_entries(&mut appender, 3);
    let entries = appender.scan_entries().expect("scan entries");

    assert_eq!(refs[0].seq, 0);
    assert_eq!(refs[1].seq, 1);
    assert_eq!(refs[2].seq, 2);
    assert_eq!(entries[0].prev_hash, [0; 32]);
    assert_eq!(entries[1].prev_hash, entries[0].entry_hash);
    assert_eq!(entries[2].prev_hash, entries[1].entry_hash);
    assert_eq!(appender.next_seq(), 3);
    assert_eq!(appender.prev_hash(), entries[2].entry_hash);

    let store = appender.into_store();
    let reopened = LedgerAppender::open(store, FixedClock::new(100)).expect("reopen appender");
    assert_eq!(reopened.next_seq(), 3);
    assert_eq!(reopened.prev_hash(), entries[2].entry_hash);
}

#[test]
fn recovery_rejects_gap_and_stale_tip() {
    let mut gapped = MemoryLedgerStore::default();
    let entry = sample_entry(1, [0; 32], EntryKind::Ingest);
    gapped.insert_raw(1, encode(&entry));
    let error = LedgerAppender::open(gapped, FixedClock::new(1)).unwrap_err();
    assert_eq!(error.code, "CALYX_LEDGER_CHAIN_BROKEN");

    let mut appender = LedgerAppender::open(MemoryLedgerStore::default(), FixedClock::new(1))
        .expect("open appender");
    let concurrent = sample_entry(0, [0; 32], EntryKind::Admin);
    appender
        .store_mut()
        .put_new(0, &encode(&concurrent))
        .expect("simulate concurrent append");
    let stale = appender
        .append(
            EntryKind::Ingest,
            SubjectId::Cx(CxId::from_bytes([1; 16])),
            b"late".to_vec(),
            ActorId::Service("svc".to_string()),
        )
        .unwrap_err();
    assert_eq!(stale.code, "CALYX_LEDGER_CHAIN_BROKEN");
}

#[test]
fn delete_and_tombstone_fail_closed() {
    let mut store = MemoryLedgerStore::default();
    assert_eq!(
        store.delete(0).unwrap_err().code,
        "CALYX_LEDGER_APPEND_ONLY_VIOLATION"
    );
    assert_eq!(
        store.tombstone(0).unwrap_err().code,
        "CALYX_LEDGER_APPEND_ONLY_VIOLATION"
    );
}

proptest! {
    #[test]
    fn sequential_appends_preserve_hash_chain(count in 1usize..=100) {
        let mut appender = LedgerAppender::open(MemoryLedgerStore::default(), FixedClock::new(7))?;
        append_sample_entries(&mut appender, count);
        let entries = appender.scan_entries()?;

        prop_assert_eq!(entries.len(), count);
        prop_assert_eq!(entries[0].prev_hash, [0; 32]);
        for index in 1..entries.len() {
            prop_assert_eq!(entries[index].prev_hash, entries[index - 1].entry_hash);
        }
    }
}

#[test]
#[ignore = "manual aiwonder FSV for PH35 LedgerAppender disk rows"]
fn ph35_ledger_appender_aiwonder_fsv() {
    let root = fsv_root();
    fs::create_dir_all(&root).expect("create fsv root");
    let ledger_dir = root.join("ledger-cf");
    reset_child_dir(&root, &ledger_dir);

    let before_rows = DirectoryLedgerStore::open(&ledger_dir)
        .expect("open before store")
        .scan()
        .expect("scan before")
        .len();
    let mut appender = LedgerAppender::open(
        DirectoryLedgerStore::open(&ledger_dir).unwrap(),
        FixedClock::new(55),
    )
    .expect("open disk appender");
    append_sample_entries(&mut appender, 5);
    let after_append = appender.scan_entries().expect("scan after append");
    drop(appender);

    let reopened = LedgerAppender::open(
        DirectoryLedgerStore::open(&ledger_dir).unwrap(),
        FixedClock::new(56),
    )
    .expect("reopen disk appender");
    let reopened_entries = reopened.scan_entries().expect("scan reopened");
    let mut reopened_store = reopened.into_store();
    let delete_error = reopened_store.delete(2).unwrap_err();
    let tombstone_error = reopened_store.tombstone(3).unwrap_err();
    let rows = reopened_store
        .scan()
        .expect("scan rows after forbidden ops");
    let tombstone_marker_files = count_tombstone_marker_files(&ledger_dir);

    let readback = build_readback(
        before_rows,
        &after_append,
        &reopened_entries,
        &rows,
        delete_error.code,
        tombstone_error.code,
        tombstone_marker_files,
    );
    let json_path = root.join("ledger-appender-readback.json");
    fs::write(&json_path, serde_json::to_vec_pretty(&readback).unwrap()).unwrap();
    let range_path = root.join("ledger-range-0-5.txt");
    fs::write(&range_path, range_text(&rows)).unwrap();

    println!("PH35_APPENDER_FSV_ROOT={}", root.display());
    println!("PH35_APPENDER_READBACK={}", json_path.display());
    println!("PH35_APPENDER_RANGE={}", range_path.display());
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    assert_eq!(before_rows, 0);
    assert_eq!(after_append.len(), 5);
    assert_eq!(reopened_entries.len(), 5);
    assert_eq!(rows.len(), 5);
    assert_eq!(readback["chain_ok"], true);
    assert_eq!(
        readback["delete_error"],
        "CALYX_LEDGER_APPEND_ONLY_VIOLATION"
    );
    assert_eq!(
        readback["tombstone_error"],
        "CALYX_LEDGER_APPEND_ONLY_VIOLATION"
    );
    assert_eq!(readback["tombstone_marker_files"], 0);
}

fn append_sample_entries<S, C>(
    appender: &mut LedgerAppender<S, C>,
    count: usize,
) -> Vec<calyx_core::LedgerRef>
where
    S: LedgerCfStore,
    C: calyx_core::Clock,
{
    (0..count)
        .map(|index| {
            appender
                .append(
                    sample_kind(index),
                    SubjectId::Cx(CxId::from_bytes([index as u8; 16])),
                    format!("payload-{index}").into_bytes(),
                    ActorId::Service("ledger-fsv".to_string()),
                )
                .expect("append sample")
        })
        .collect()
}

fn sample_entry(seq: u64, prev_hash: [u8; 32], kind: EntryKind) -> LedgerEntry {
    LedgerEntry::new(
        seq,
        prev_hash,
        kind,
        SubjectId::Cx(CxId::from_bytes([seq as u8; 16])),
        format!("payload-{seq}").into_bytes(),
        ActorId::Service("ledger-fsv".to_string()),
        7,
    )
}

fn sample_kind(index: usize) -> EntryKind {
    match index % 5 {
        0 => EntryKind::Ingest,
        1 => EntryKind::Measure,
        2 => EntryKind::Assay,
        3 => EntryKind::Kernel,
        _ => EntryKind::Admin,
    }
}

fn build_readback(
    before_rows: usize,
    after_append: &[LedgerEntry],
    reopened_entries: &[LedgerEntry],
    rows: &[LedgerRow],
    delete_error: &str,
    tombstone_error: &str,
    tombstone_marker_files: usize,
) -> serde_json::Value {
    json!({
        "before_rows": before_rows,
        "after_append_rows": after_append.len(),
        "reopened_rows": reopened_entries.len(),
        "row_files": rows.iter().map(|row| format!("{:016x}.ledger", row.seq)).collect::<Vec<_>>(),
        "seqs": rows.iter().map(|row| row.seq).collect::<Vec<_>>(),
        "rows": reopened_entries.iter().enumerate().map(|(index, entry)| {
            json!({
                "seq": entry.seq,
                "kind": entry.kind.as_str(),
                "prev_hash": hex(&entry.prev_hash),
                "entry_hash": hex(&entry.entry_hash),
                "prev_matches_prior": index == 0 && entry.prev_hash == [0; 32]
                    || index > 0 && entry.prev_hash == reopened_entries[index - 1].entry_hash,
            })
        }).collect::<Vec<_>>(),
        "chain_ok": chain_ok(reopened_entries),
        "delete_error": delete_error,
        "tombstone_error": tombstone_error,
        "tombstone_marker_files": tombstone_marker_files,
    })
}

fn chain_ok(entries: &[LedgerEntry]) -> bool {
    entries.iter().enumerate().all(|(index, entry)| {
        index == 0 && entry.prev_hash == [0; 32]
            || index > 0 && entry.prev_hash == entries[index - 1].entry_hash
    })
}

fn range_text(rows: &[LedgerRow]) -> String {
    let mut out = String::new();
    for row in rows {
        let entry = decode(&row.bytes).expect("decode row for readback text");
        out.push_str(&format!(
            "seq={} prev_hash={} entry_hash={} bytes={}\n",
            entry.seq,
            hex(&entry.prev_hash),
            hex(&entry.entry_hash),
            hex(&row.bytes)
        ));
    }
    out
}

fn fsv_root() -> PathBuf {
    std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-ph35-ledger-appender-fsv"))
}

fn reset_child_dir(root: &Path, child: &Path) {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    if child.exists() {
        let child_canonical = child.canonicalize().expect("canonical child path");
        assert!(child_canonical.starts_with(&root));
        fs::remove_dir_all(&child_canonical).expect("remove stale child dir");
    }
    fs::create_dir_all(child).expect("create child dir");
}

fn count_tombstone_marker_files(dir: &Path) -> usize {
    fs::read_dir(dir)
        .expect("read ledger dir")
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry.path().extension().and_then(|value| value.to_str()) == Some("tombstone")
        })
        .count()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
