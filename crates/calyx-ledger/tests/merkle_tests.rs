use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::{FixedClock, Result};
use calyx_ledger::{
    ActorId, DirectoryLedgerStore, EntryKind, LedgerAppender, LedgerCfStore, MemoryLedgerStore,
    MerkleExportBundle, SubjectId, leaf_hash, merkle_root, merkle_root_of_hashes, sign_root,
    verify_signature,
};
use serde_json::json;

const FOUR_HASH_GOLDEN: [u8; 32] = [
    82, 42, 98, 143, 4, 63, 90, 174, 186, 178, 142, 168, 154, 115, 204, 5, 151, 210, 9, 148, 62,
    139, 152, 76, 9, 33, 56, 82, 179, 175, 232, 20,
];

#[test]
fn merkle_single_and_pair_roots_match_domain_hashes() {
    let zero = [0; 32];
    let one = [1; 32];

    assert_eq!(merkle_root_of_hashes(&[zero]), leaf_hash(&zero));
    assert_eq!(
        merkle_root_of_hashes(&[zero, one]),
        calyx_ledger::combine_hash(&leaf_hash(&zero), &leaf_hash(&one))
    );
}

#[test]
fn merkle_four_hash_golden_is_stable() {
    let hashes = [[0; 32], [1; 32], [2; 32], [3; 32]];
    let root = merkle_root_of_hashes(&hashes);

    println!("PH36_FOUR_HASH_GOLDEN={}", hex(&root));
    assert_eq!(root, FOUR_HASH_GOLDEN);
}

#[test]
fn merkle_store_range_requires_contiguous_rows() {
    let (store, entries) = memory_store_with_entries(4).unwrap();
    let expected_hashes: Vec<_> = entries.iter().map(|entry| entry.entry_hash).collect();

    assert_eq!(
        merkle_root(&store, 0..4).unwrap(),
        merkle_root_of_hashes(&expected_hashes)
    );
    assert_eq!(merkle_root(&store, 2..2).unwrap(), [0; 32]);

    let mut missing = MemoryLedgerStore::default();
    for row in store.scan().unwrap().into_iter().filter(|row| row.seq != 2) {
        missing.insert_raw(row.seq, row.bytes);
    }
    let error = merkle_root(&missing, 0..4).unwrap_err();
    assert_eq!(error.code, "CALYX_LEDGER_CORRUPT");
    assert!(error.message.contains("missing ledger row for seq 2"));
}

#[test]
fn merkle_edges_cover_odd_and_large_ranges() {
    let odd = [[7; 32], [8; 32], [9; 32]];
    let odd_root = merkle_root_of_hashes(&odd);
    assert_ne!(odd_root, [0; 32]);

    let large: Vec<_> = (0..1000).map(pattern_hash).collect();
    let large_root = merkle_root_of_hashes(&large);
    assert_ne!(large_root, [0; 32]);
}

#[test]
fn merkle_signed_export_round_trips_and_detects_tamper() {
    let root = merkle_root_of_hashes(&[[4; 32], [5; 32], [6; 32]]);
    let seed = [42; 32];
    let bundle = MerkleExportBundle::signed(0..3, root, &seed);

    assert_eq!(bundle.signature, Some(sign_root(0..3, &root, &seed)));
    assert!(verify_signature(&bundle));

    let mut tampered_root = bundle.clone();
    tampered_root.root[0] ^= 0xff;
    assert!(!verify_signature(&tampered_root));

    let mut tampered_start = bundle.clone();
    tampered_start.range_start = 1;
    assert!(!verify_signature(&tampered_start));

    let mut tampered_end = bundle.clone();
    tampered_end.range_end = 4;
    assert!(!verify_signature(&tampered_end));

    let mut replayed_range = bundle.clone();
    replayed_range.range_start = 10;
    replayed_range.range_end = 13;
    assert!(!verify_signature(&replayed_range));

    assert!(!verify_signature(&MerkleExportBundle::unsigned(0..3, root)));
}

#[test]
fn merkle_export_bundle_is_canonical_json_serializable() {
    let root = merkle_root_of_hashes(&[[11; 32]]);
    let bundle = MerkleExportBundle::signed(10..11, root, &[9; 32]);
    let bytes = serde_json::to_vec(&bundle).unwrap();
    let decoded: MerkleExportBundle = serde_json::from_slice(&bytes).unwrap();

    assert_eq!(decoded, bundle);
    assert!(verify_signature(&decoded));
}

#[test]
#[ignore = "manual FSV writes PH36 Merkle readback artifacts"]
fn ph36_merkle_root_ed25519_manual_fsv() {
    let root = fsv_root().join("merkle-root-ed25519");
    let ledger_dir = root.join("ledger-cf");
    reset_child_dir(&root, &ledger_dir);

    let before_rows = DirectoryLedgerStore::open(&ledger_dir)
        .unwrap()
        .scan()
        .unwrap()
        .len();
    let entries = append_directory_entries(&ledger_dir, 4).unwrap();
    let store = DirectoryLedgerStore::open(&ledger_dir).unwrap();
    let rows = store.scan().unwrap();
    let hashes: Vec<_> = entries.iter().map(|entry| entry.entry_hash).collect();
    let expected = merkle_root_of_hashes(&hashes);
    let root_0_4 = merkle_root(&store, 0..4).unwrap();
    let root_0_3 = merkle_root(&store, 0..3).unwrap();
    let signed = MerkleExportBundle::signed(0..4, root_0_4, &[42; 32]);
    let mut tampered_root = signed.clone();
    tampered_root.root[31] ^= 0x55;
    let mut tampered_start = signed.clone();
    tampered_start.range_start = 1;
    let mut tampered_end = signed.clone();
    tampered_end.range_end = 5;
    let mut replayed_range = signed.clone();
    replayed_range.range_start = 10;
    replayed_range.range_end = 14;
    let missing_error = merkle_root(&store, 0..5).unwrap_err();

    let readback = json!({
        "before_rows": before_rows,
        "after_rows": rows.len(),
        "row_files": rows.iter().map(|row| format!("{:016x}.ledger", row.seq)).collect::<Vec<_>>(),
        "root_0_4": hex(&root_0_4),
        "root_0_3": hex(&root_0_3),
        "expected_root_0_4": hex(&expected),
        "golden_four_hash_root": hex(&FOUR_HASH_GOLDEN),
        "root_matches_expected": root_0_4 == expected,
        "signature_round_trip": verify_signature(&signed),
        "signature_root_tamper_detected": !verify_signature(&tampered_root),
        "signature_range_start_tamper_detected": !verify_signature(&tampered_start),
        "signature_range_end_tamper_detected": !verify_signature(&tampered_end),
        "signature_wrong_range_replay_detected": !verify_signature(&replayed_range),
        "signed_bundle": {
            "range_start": signed.range_start,
            "range_end": signed.range_end,
            "root": hex(&signed.root),
            "signature": hex(&signed.signature.expect("signature")),
            "signer_pubkey": hex(&signed.signer_pubkey.expect("pubkey")),
        },
        "missing_row_error": missing_error.code,
        "ledger_dir": ledger_dir,
    });
    fs::write(
        root.join("merkle-root-ed25519-readback.json"),
        serde_json::to_vec_pretty(&readback).unwrap(),
    )
    .unwrap();
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    assert_eq!(before_rows, 0);
    assert_eq!(rows.len(), 4);
    assert_eq!(root_0_4, expected);
    assert!(verify_signature(&signed));
    assert!(!verify_signature(&tampered_root));
    assert!(!verify_signature(&tampered_start));
    assert!(!verify_signature(&tampered_end));
    assert!(!verify_signature(&replayed_range));
    assert_eq!(missing_error.code, "CALYX_LEDGER_CORRUPT");
}

fn memory_store_with_entries(
    count: u64,
) -> Result<(MemoryLedgerStore, Vec<calyx_ledger::LedgerEntry>)> {
    let mut appender = LedgerAppender::open(MemoryLedgerStore::default(), FixedClock::new(100))?;
    for seq in 0..count {
        append_entry(&mut appender, seq)?;
    }
    let entries = appender.scan_entries()?;
    Ok((appender.into_store(), entries))
}

fn append_directory_entries(
    ledger_dir: &Path,
    count: u64,
) -> Result<Vec<calyx_ledger::LedgerEntry>> {
    let mut appender = LedgerAppender::open(
        DirectoryLedgerStore::open(ledger_dir)?,
        FixedClock::new(200),
    )?;
    for seq in 0..count {
        append_entry(&mut appender, seq)?;
    }
    appender.scan_entries()
}

fn append_entry<S: LedgerCfStore>(
    appender: &mut LedgerAppender<S, FixedClock>,
    seq: u64,
) -> Result<()> {
    appender.append(
        EntryKind::Admin,
        SubjectId::Query(vec![seq as u8]),
        format!(r#"{{"seq":{seq}}}"#).into_bytes(),
        ActorId::Service("merkle-test".to_string()),
    )?;
    Ok(())
}

fn pattern_hash(value: u16) -> [u8; 32] {
    let mut hash = [0; 32];
    hash[..2].copy_from_slice(&value.to_be_bytes());
    hash[2] = (value % 251) as u8;
    hash
}

fn fsv_root() -> PathBuf {
    std::env::var_os("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("calyx-ph36-merkle-fsv"))
}

fn reset_child_dir(root: &Path, child: &Path) {
    fs::create_dir_all(root).unwrap();
    if child.exists() {
        fs::remove_dir_all(child).unwrap();
    }
    fs::create_dir_all(child).unwrap();
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(hex_digit(byte >> 4));
        out.push(hex_digit(byte & 0x0f));
    }
    out
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => char::from(b'0' + value),
        10..=15 => char::from(b'a' + value - 10),
        _ => unreachable!("nibble out of range"),
    }
}
