use std::fs;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::*;
use crate::vault::encode::WriteRow;
use crate::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Constellation, CxFlags, CxId, InputRef, LedgerRef, Modality, VaultId, VaultStore,
};
use calyx_ledger::{ActorId, EntryKind, SubjectId};

#[test]
fn open_waits_for_durable_commit_lock_before_reading_rows_and_anchor() {
    let root = test_vault_dir("issue973-open-lock");
    fs::create_dir_all(root.join("cf").join(ColumnFamily::Ledger.name())).unwrap();

    let guard = crate::file_lock::FileLockGuard::acquire(&durable_commit_lock_path(&root))
        .expect("acquire writer commit lock");
    let (sender, receiver) = mpsc::channel();
    let thread_root = root.clone();
    let handle = thread::spawn(move || {
        let result = AsterLedgerCfStore::open(&thread_root)
            .and_then(|store| store.scan().map(|rows| rows.len()));
        sender.send(result).expect("send open result");
    });

    assert!(
        receiver.recv_timeout(Duration::from_millis(100)).is_err(),
        "ledger view opened while a writer-owned durable commit lock was held"
    );

    drop(guard);
    let row_count = receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("ledger view should open after commit lock release")
        .expect("open ledger view");
    assert_eq!(row_count, 0);
    handle.join().expect("open thread");
    fs::remove_dir_all(root).ok();
}

#[test]
fn targeted_reader_matches_physical_ledger_rows() {
    let root = test_vault_dir("issue1001-point-reader");
    let vault_id = vault_id();
    let vault = AsterVault::new_durable(
        &root,
        vault_id,
        b"point-reader-salt",
        VaultOptions::default(),
    )
    .expect("open durable vault");
    for seed in 0..4 {
        vault
            .put(sample_constellation(vault_id, seed))
            .expect("put sample row");
    }
    vault.flush().expect("flush physical rows");
    drop(vault);

    let physical = AsterLedgerCfStore::open(&root).expect("open full ledger view");
    let physical_rows = physical.scan().expect("scan physical rows");
    let mut wanted = physical_rows
        .iter()
        .map(|row| row.seq)
        .collect::<BTreeSet<_>>();
    let missing_seq = physical_rows.len() as u64 + 10;
    wanted.insert(missing_seq);
    let targeted = read_ledger_seqs(&root, &wanted).expect("targeted read");

    for row in &physical_rows {
        assert_eq!(targeted.get(&row.seq).cloned(), Some(row.clone()));
    }
    assert_eq!(targeted.get(&missing_seq), None);

    fs::remove_dir_all(root).ok();
}

#[test]
fn targeted_reader_replays_wal_for_uncheckpointed_ledger_row() {
    let root = test_vault_dir("issue1001-point-reader-wal");
    let vault_id = vault_id();
    let vault = AsterVault::new_durable(
        &root,
        vault_id,
        b"point-reader-wal-salt",
        VaultOptions::default(),
    )
    .expect("open durable vault");
    let cx_id = vault
        .put(sample_constellation(vault_id, 9))
        .expect("put uncheckpointed row");
    let stored = vault
        .get(cx_id, vault.snapshot())
        .expect("read uncheckpointed constellation");
    let seq = stored.provenance.seq;
    let ledger_dir = root.join("cf").join(ColumnFamily::Ledger.name());
    let sst_files = ledger_dir
        .read_dir()
        .map(|entries| entries.count())
        .unwrap_or(0);
    assert_eq!(sst_files, 0, "ledger row should still be WAL-only");

    let physical = AsterLedgerCfStore::open(&root)
        .expect("open full ledger view")
        .read_seq(seq)
        .expect("read physical WAL-backed seq")
        .expect("physical WAL-backed row exists");
    let targeted =
        read_ledger_seqs(&root, &BTreeSet::from([seq])).expect("targeted WAL-backed read");
    assert_eq!(targeted.get(&seq), Some(&physical));

    fs::remove_dir_all(root).ok();
}

#[test]
fn targeted_reader_uses_key_range_for_drifted_ledger_file_seq() {
    let root = test_vault_dir("issue1001-ledger-key-range");
    let ledger_dir = root.join("cf").join(ColumnFamily::Ledger.name());
    fs::create_dir_all(&ledger_dir).expect("create ledger CF dir");
    crate::sst::write_sst(
        ledger_dir.join("00000000000000000005-0000.sst"),
        [(ledger_key(10).as_slice(), b"ledger-row-10".as_slice())],
    )
    .expect("write drifted ledger SST");
    fs::write(ledger_dir.join("not-a-canonical-sst-name.sst"), b"bad").expect("write bad SST name");

    let targeted = read_ledger_seqs(&root, &BTreeSet::from([10])).expect("targeted key-range read");
    assert_eq!(
        targeted.get(&10).map(|row| row.bytes.as_slice()),
        Some(b"ledger-row-10".as_slice())
    );

    fs::remove_dir_all(root).ok();
}

#[test]
fn targeted_reader_finds_nonzero_durable_batch_index() {
    let root = test_vault_dir("issue1001-ledger-index");
    let vault = AsterVault::new_durable(
        &root,
        vault_id(),
        b"ledger-index-salt",
        VaultOptions::default(),
    )
    .expect("open durable vault");
    let ledger_ref = vault
        .commit_rows_with_ledger_entry_locked(
            vec![
                WriteRow {
                    cf: ColumnFamily::Kv,
                    key: b"prefix-a".to_vec(),
                    value: b"a".to_vec(),
                },
                WriteRow {
                    cf: ColumnFamily::Kv,
                    key: b"prefix-b".to_vec(),
                    value: b"b".to_vec(),
                },
            ],
            EntryKind::Ingest,
            SubjectId::Query(b"issue1001-nonzero-ledger-index".to_vec()),
            b"payload".to_vec(),
            ActorId::System,
        )
        .expect("commit nonzero ledger batch index");
    let seq = ledger_ref.seq;
    let durable_seq = vault.latest_seq();
    vault.flush().expect("flush physical rows");
    drop(vault);

    let expected_path = root
        .join("cf")
        .join(ColumnFamily::Ledger.name())
        .join(format!("{durable_seq:020}-0002.sst"));
    assert!(expected_path.exists(), "{}", expected_path.display());

    let physical = AsterLedgerCfStore::open(&root)
        .expect("open full ledger view")
        .read_seq(seq)
        .expect("read physical ledger seq")
        .expect("physical ledger row exists");
    let targeted = read_ledger_seqs(&root, &BTreeSet::from([seq])).expect("targeted nonzero read");
    assert_eq!(targeted.get(&seq), Some(&physical));

    fs::remove_dir_all(root).ok();
}

#[test]
fn targeted_reader_finds_row_from_high_durable_batch_index() {
    let root = test_vault_dir("issue1001-ledger-high-index");
    let vault = AsterVault::new_durable(
        &root,
        vault_id(),
        b"ledger-high-index-salt",
        VaultOptions::default(),
    )
    .expect("open durable vault");
    let high_index = 257usize;
    let prefix_rows = (0..high_index)
        .map(|index| WriteRow {
            cf: ColumnFamily::Kv,
            key: format!("prefix-{index:04}").into_bytes(),
            value: b"x".to_vec(),
        })
        .collect::<Vec<_>>();
    let ledger_ref = vault
        .commit_rows_with_ledger_entry_locked(
            prefix_rows,
            EntryKind::Ingest,
            SubjectId::Query(b"issue1001-ledger-bound".to_vec()),
            b"payload".to_vec(),
            ActorId::System,
        )
        .expect("commit beyond targeted ledger batch index");
    let seq = ledger_ref.seq;
    let durable_seq = vault.latest_seq();
    vault.flush().expect("flush physical rows");
    drop(vault);

    let expected_path = root
        .join("cf")
        .join(ColumnFamily::Ledger.name())
        .join(format!("{durable_seq:020}-{high_index:04}.sst"));
    assert!(expected_path.exists(), "{}", expected_path.display());
    let physical = AsterLedgerCfStore::open(&root)
        .expect("open full ledger view")
        .read_seq(seq)
        .expect("read physical ledger seq")
        .expect("physical ledger row exists");
    let targeted = read_ledger_seqs(&root, &BTreeSet::from([seq]))
        .expect("complete targeted read beyond direct bound");
    assert_eq!(targeted.get(&seq), Some(&physical));

    fs::remove_dir_all(root).ok();
}

fn test_vault_dir(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "calyx-aster-{name}-{}-{unique}",
        std::process::id()
    ))
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn sample_constellation(vault_id: VaultId, seed: u8) -> Constellation {
    Constellation {
        cx_id: CxId::from_bytes([seed; 16]),
        vault_id,
        panel_version: 1,
        created_at: 42 + u64::from(seed),
        input_ref: InputRef {
            hash: [seed; 32],
            pointer: Some(format!("synthetic://issue1001-point-reader/{seed}")),
            redacted: false,
        },
        modality: Modality::Text,
        slots: BTreeMap::new(),
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 0,
            hash: [0; 32],
        },
        flags: CxFlags::default(),
    }
}
