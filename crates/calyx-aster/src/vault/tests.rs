use super::*;
use calyx_core::{
    AbsentReason, AnchorKind, AnchorValue, CxFlags, FixedClock, InputRef, LedgerRef,
    METADATA_CHUNK_ID, METADATA_DATABASE_NAME, Modality, SlotVector,
};
use calyx_ledger::{EntryKind, SubjectId, decode as decode_ledger};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("valid ULID")
}

fn sample_constellation(vault: &AsterVault<FixedClock>) -> Constellation {
    let input = b"same-input";
    let cx_id = vault.cx_id_for_input(input, 7);
    let mut input_hash = [0_u8; 32];
    input_hash[..input.len()].copy_from_slice(input);
    let mut slots = BTreeMap::new();
    slots.insert(
        SlotId::new(0),
        SlotVector::Dense {
            dim: 2,
            data: vec![0.25, 0.75],
        },
    );
    slots.insert(
        SlotId::new(1),
        SlotVector::Absent {
            reason: AbsentReason::LensUnavailable,
        },
    );
    let mut metadata = BTreeMap::new();
    metadata.insert(
        METADATA_CHUNK_ID.to_string(),
        "chunk-same-input".to_string(),
    );
    metadata.insert(
        METADATA_DATABASE_NAME.to_string(),
        "leapable_db_vault_tests".to_string(),
    );
    Constellation {
        cx_id,
        vault_id: vault_id(),
        panel_version: 7,
        created_at: 123,
        input_ref: InputRef {
            hash: input_hash,
            pointer: Some("synthetic://same-input".to_string()),
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata,
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 1,
            hash: [9; 32],
        },
        flags: CxFlags {
            ungrounded: true,
            ..CxFlags::default()
        },
    }
}

#[test]
fn put_get_roundtrips_base_and_slot_cfs() {
    let vault = AsterVault::with_clock(vault_id(), b"salt".to_vec(), FixedClock::new(123));
    let cx = sample_constellation(&vault);
    let id = cx.cx_id;

    vault.put(cx.clone()).expect("put");
    let got = vault.get(id, vault.snapshot()).expect("get");

    assert_eq!(got, cx);
    assert!(matches!(
        got.slots.get(&SlotId::new(1)),
        Some(SlotVector::Absent {
            reason: AbsentReason::LensUnavailable
        })
    ));
}

#[test]
fn duplicate_put_is_idempotent_noop() {
    let vault = AsterVault::with_clock(vault_id(), b"salt".to_vec(), FixedClock::new(123));
    let cx = sample_constellation(&vault);

    vault.put(cx.clone()).expect("first put");
    let seq_after_first = vault.snapshot();
    vault.put(cx).expect("duplicate put");

    assert_eq!(vault.snapshot(), seq_after_first);
}

#[test]
fn same_cxid_with_different_bytes_fails_closed() {
    let vault = AsterVault::with_clock(vault_id(), b"salt".to_vec(), FixedClock::new(123));
    let cx = sample_constellation(&vault);
    let mut changed = cx.clone();
    // `created_at` is deliberately excluded from constellation identity
    // (`normalized_anchor_identity` zeroes it) so a re-put with a newer
    // timestamp stays idempotent. Mutate an identity-bearing content field (the
    // input-reference bytes) to exercise the same-cxid/different-bytes invariant.
    changed.input_ref.hash[0] ^= 0xFF;

    vault.put(cx).expect("first put");
    let error = vault.put(changed).expect_err("collision rejected");

    assert_eq!(error.code, "CALYX_ASTER_CORRUPT_SHARD");
}

#[test]
fn anchor_writes_anchor_cf_and_updates_get() {
    let vault = AsterVault::with_clock(vault_id(), b"salt".to_vec(), FixedClock::new(123));
    let cx = sample_constellation(&vault);
    let id = cx.cx_id;
    let anchor = Anchor {
        kind: AnchorKind::Reward,
        value: AnchorValue::Number(1.0),
        source: "unit-test".to_string(),
        observed_at: 124,
        confidence: 1.0,
    };

    vault.put(cx).expect("put");
    vault.anchor(id, anchor.clone()).expect("anchor");
    let got = vault.get(id, vault.snapshot()).expect("get anchored");
    let anchor_bytes = vault
        .read_cf_at(
            vault.snapshot(),
            ColumnFamily::Anchors,
            &anchor_key(id, &AnchorKind::Reward),
        )
        .expect("read anchor cf")
        .expect("anchor row");

    assert_eq!(got.anchors.as_slice(), std::slice::from_ref(&anchor));
    assert_eq!(encode::decode_anchor(&anchor_bytes).unwrap(), anchor);
}

#[test]
fn duplicate_put_after_anchor_preserves_anchor_noop() {
    let vault = AsterVault::with_clock(vault_id(), b"salt".to_vec(), FixedClock::new(123));
    let cx = sample_constellation(&vault);
    let id = cx.cx_id;
    let anchor = Anchor {
        kind: AnchorKind::Reward,
        value: AnchorValue::Number(1.0),
        source: "unit-test".to_string(),
        observed_at: 124,
        confidence: 1.0,
    };

    vault.put(cx.clone()).expect("put");
    vault.anchor(id, anchor.clone()).expect("anchor");
    let seq_after_anchor = vault.snapshot();
    vault.put(cx).expect("duplicate put after anchor");
    let got = vault.get(id, vault.snapshot()).expect("get anchored");

    assert_eq!(vault.snapshot(), seq_after_anchor);
    assert_eq!(got.anchors.as_slice(), std::slice::from_ref(&anchor));
}

#[test]
fn duplicate_put_with_conflicting_anchor_fails_closed() {
    let vault = AsterVault::with_clock(vault_id(), b"salt".to_vec(), FixedClock::new(123));
    let mut cx = sample_constellation(&vault);
    let id = cx.cx_id;
    cx.anchors = vec![Anchor {
        kind: AnchorKind::SpeakerMatch,
        value: AnchorValue::Text("speaker-a".to_string()),
        source: "unit-test".to_string(),
        observed_at: 124,
        confidence: 1.0,
    }];
    let mut changed = cx.clone();
    changed.anchors[0].value = AnchorValue::Text("speaker-b".to_string());

    vault.put(cx.clone()).expect("first put");
    let error = vault
        .put(changed)
        .expect_err("same-CxId anchor conflict must fail closed");
    let got = vault.get(id, vault.snapshot()).expect("get original");

    assert_eq!(error.code, "CALYX_ASTER_CORRUPT_SHARD");
    assert_eq!(got.anchors, cx.anchors);
}

#[test]
fn binary_codecs_roundtrip_known_offsets_and_fail_closed() {
    let vault = AsterVault::with_clock(vault_id(), b"salt".to_vec(), FixedClock::new(123));
    let cx = sample_constellation(&vault);
    let header = encode::encode_header(&cx);

    assert_eq!(&header[0..16], cx.cx_id.as_bytes());
    assert_eq!(&header[32..36], &7_u32.to_be_bytes());
    assert_eq!(header.len(), encode::HEADER_LEN);
    assert_eq!(encode::decode_header(&header).unwrap().cx_id, cx.cx_id);

    let base = encode::encode_constellation_base(&cx).expect("encode base");
    let decoded = encode::decode_constellation_base(&base).expect("decode base");
    assert_eq!(decoded.cx_id, cx.cx_id);
    assert_eq!(decoded.input_ref, cx.input_ref);
    assert!(encode::decode_header(&header[..encode::HEADER_LEN - 1]).is_err());

    for vector in cx.slots.values() {
        let bytes = encode::encode_slot_vector(vector).expect("encode slot");
        assert_eq!(encode::decode_slot_vector(&bytes).unwrap(), *vector);
    }
    let anchor = Anchor {
        kind: AnchorKind::Label("axis".to_string()),
        value: AnchorValue::Text("grounded".to_string()),
        source: "unit-test".to_string(),
        observed_at: 125,
        confidence: 0.5,
    };
    let bytes = encode::encode_anchor(&anchor).expect("encode anchor");
    assert_eq!(encode::decode_anchor(&bytes).unwrap(), anchor);
    assert!(encode::decode_anchor(&bytes[..bytes.len() - 1]).is_err());
}

#[test]
fn anchor_vector_decode_rejects_non_finite_values() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&6_u16.to_be_bytes());
    bytes.push(5);
    bytes.extend_from_slice(&2_u32.to_be_bytes());
    bytes.extend_from_slice(&f32::NAN.to_bits().to_be_bytes());
    bytes.extend_from_slice(&1.0_f32.to_bits().to_be_bytes());
    bytes.extend_from_slice(&0_u32.to_be_bytes());
    bytes.extend_from_slice(&0_u64.to_be_bytes());
    bytes.extend_from_slice(&1.0_f32.to_bits().to_be_bytes());

    let err = encode::decode_anchor(&bytes).expect_err("nan vector must fail closed");
    assert!(err.to_string().contains("non-finite"));
}

#[test]
fn durable_vault_writes_wal_sst_manifest_and_cold_opens() {
    let dir = test_dir("durable");
    let vault =
        AsterVault::new_durable(&dir, vault_id(), b"salt".to_vec(), VaultOptions::default())
            .expect("open durable");
    let cx = sample_constellation(&AsterVault::with_clock(
        vault_id(),
        b"salt".to_vec(),
        FixedClock::new(123),
    ));
    let id = cx.cx_id;

    vault.put(cx.clone()).expect("durable put");
    vault.flush().expect("flush durable");

    let wal = dir.join("wal/00000000000000000000.wal");
    let wal_bytes = fs::read(&wal).expect("read wal");
    assert_eq!(&wal_bytes[0..4], b"CXW1");
    let replay = crate::wal::replay_dir(dir.join("wal")).expect("replay wal");
    let wal_rows = encode::decode_write_batch(&replay.records[0].payload).expect("decode batch");
    let ledger_index = wal_rows
        .iter()
        .position(|row| row.cf == ColumnFamily::Ledger)
        .expect("ledger row in WAL batch");
    let base_index = wal_rows
        .iter()
        .position(|row| row.cf == ColumnFamily::Base)
        .expect("base row in WAL batch");
    assert!(ledger_index < base_index);
    let ledger_entry = decode_ledger(&wal_rows[ledger_index].value).expect("decode ledger entry");
    assert_eq!(wal_rows[ledger_index].key, ledger_key(0));
    assert_eq!(ledger_entry.seq, 0);
    assert_eq!(ledger_entry.prev_hash, [0; 32]);
    assert_eq!(ledger_entry.kind, EntryKind::Ingest);
    assert_eq!(ledger_entry.subject, SubjectId::Cx(id));

    assert!(dir.join("CURRENT").exists());
    assert_eq!(sst_count(dir.join("cf/base")), 2);

    let reopened = AsterVault::open(&dir, vault_id(), b"salt".to_vec(), VaultOptions::default())
        .expect("cold open");
    let stored_ledger = reopened
        .read_cf_at(reopened.snapshot(), ColumnFamily::Ledger, &ledger_key(0))
        .expect("read ledger cf")
        .expect("ledger row");
    let mut expected = cx;
    expected.provenance = LedgerRef {
        seq: ledger_entry.seq,
        hash: ledger_entry.entry_hash,
    };
    assert_eq!(reopened.snapshot(), 1);
    assert_eq!(stored_ledger, wal_rows[ledger_index].value);
    assert_eq!(reopened.get(id, reopened.snapshot()).unwrap(), expected);
    cleanup(dir);
}

#[test]
fn durable_memtable_oversize_rejects_before_wal_append() {
    let fsv_root = std::env::var_os("CALYX_FSV_ROOT").map(PathBuf::from);
    let dir = fsv_root.as_ref().map_or_else(
        || test_dir("memtable-oversize-preflight"),
        |root| {
            let dir = root.join("memtable-oversize-preflight").join("vault");
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("create fsv vault");
            dir
        },
    );
    let options = VaultOptions {
        memtable_byte_cap: 16,
        ..VaultOptions::default()
    };
    let vault =
        AsterVault::new_durable(&dir, vault_id(), b"salt".to_vec(), options).expect("open durable");

    let before_wal_bytes = wal_bytes(&dir);
    let before_seq = vault.snapshot();
    let error = vault
        .write_cf(ColumnFamily::Base, b"k".to_vec(), vec![0xAA; 64])
        .expect_err("oversize row rejects before WAL");

    assert_eq!(error.code, "CALYX_BACKPRESSURE");
    assert_eq!(vault.snapshot(), before_seq);
    assert_eq!(wal_bytes(&dir), before_wal_bytes);
    drop(vault);

    let reopened = AsterVault::open(&dir, vault_id(), b"salt".to_vec(), VaultOptions::default())
        .expect("cold open after rejected write");

    assert_eq!(reopened.snapshot(), before_seq);
    assert_eq!(
        reopened
            .read_cf_at(reopened.snapshot(), ColumnFamily::Base, b"k")
            .unwrap(),
        None
    );
    if let Some(root) = fsv_root {
        let readback = serde_json::json!({
            "error_code": error.code,
            "snapshot_before": before_seq,
            "cold_open_snapshot": reopened.snapshot(),
            "wal_bytes_before": before_wal_bytes,
            "wal_bytes_after": wal_bytes(&dir),
            "rejected_key_visible": false,
        });
        fs::write(
            root.join("memtable-oversize-preflight-readback.json"),
            serde_json::to_vec_pretty(&readback).unwrap(),
        )
        .unwrap();
    } else {
        cleanup(dir);
    }
}

#[test]
#[ignore = "manual FSV for PH35 ledger group-commit WAL rows"]
fn ph35_ledger_group_commit_manual_fsv() {
    let root = fsv_root().join("group-commit-hook");
    reset_dir(&root);
    let vault_dir = root.join("vault");
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"salt".to_vec(),
        VaultOptions::default(),
    )
    .expect("open durable");
    let cx = sample_constellation(&AsterVault::with_clock(
        vault_id(),
        b"salt".to_vec(),
        FixedClock::new(123),
    ));
    let id = cx.cx_id;

    let before = vault
        .read_cf_at(0, ColumnFamily::Ledger, &ledger_key(0))
        .expect("read before");
    vault.put(cx).expect("durable put");
    vault.flush().expect("flush durable");

    let wal_path = vault_dir.join("wal/00000000000000000000.wal");
    let wal_bytes = fs::read(&wal_path).expect("read wal");
    let replay = crate::wal::replay_dir(vault_dir.join("wal")).expect("replay wal");
    let wal_rows = encode::decode_write_batch(&replay.records[0].payload).expect("decode batch");
    let ledger_index = row_index(&wal_rows, ColumnFamily::Ledger);
    let base_index = row_index(&wal_rows, ColumnFamily::Base);
    let ledger_entry = decode_ledger(&wal_rows[ledger_index].value).expect("decode ledger entry");
    let after = vault
        .read_cf_at(vault.snapshot(), ColumnFamily::Ledger, &ledger_key(0))
        .expect("read after")
        .expect("ledger row");
    let got = vault.get(id, vault.snapshot()).expect("get stored");
    let payload_json: serde_json::Value =
        serde_json::from_slice(&ledger_entry.payload).expect("payload json");

    let readback = serde_json::json!({
        "before_ledger_row_present": before.is_some(),
        "after_ledger_row_present": true,
        "ledger_cf_matches_wal_row": after == wal_rows[ledger_index].value,
        "same_wal_record": true,
        "wal_record_seq": replay.records[0].seq,
        "wal_row_count": wal_rows.len(),
        "ledger_row_index": ledger_index,
        "base_row_index": base_index,
        "ledger_before_base": ledger_index < base_index,
        "ledger_key_hex": hex(&wal_rows[ledger_index].key),
        "base_key_hex": hex(&wal_rows[base_index].key),
        "entry": {
            "seq": ledger_entry.seq,
            "prev_hash": hex(&ledger_entry.prev_hash),
            "kind": ledger_entry.kind.as_str(),
            "subject_is_cx": matches!(ledger_entry.subject, SubjectId::Cx(value) if value == id),
            "entry_hash": hex(&ledger_entry.entry_hash),
            "payload": payload_json,
        },
        "stored_constellation_provenance": {
            "seq": got.provenance.seq,
            "hash": hex(&got.provenance.hash),
        },
        "wal_file": wal_path,
        "wal_bytes": wal_bytes.len(),
        "wal_prefix_hex": hex(&wal_bytes[..wal_bytes.len().min(256)]),
    });
    let readback_path = root.join("ledger-group-commit-readback.json");
    fs::write(
        &readback_path,
        serde_json::to_vec_pretty(&readback).unwrap(),
    )
    .unwrap();

    println!("PH35_GROUP_COMMIT_FSV_ROOT={}", root.display());
    println!("PH35_GROUP_COMMIT_READBACK={}", readback_path.display());
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    assert_eq!(before, None);
    assert!(ledger_index < base_index);
    assert_eq!(ledger_entry.seq, 0);
    assert_eq!(ledger_entry.prev_hash, [0; 32]);
    assert_eq!(ledger_entry.kind, EntryKind::Ingest);
    assert_eq!(got.provenance.seq, ledger_entry.seq);
    assert_eq!(got.provenance.hash, ledger_entry.entry_hash);
    assert_eq!(after, wal_rows[ledger_index].value);
}

fn test_dir(name: &str) -> PathBuf {
    let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "calyx-aster-vault-{name}-{}-{id}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create test dir");
    dir
}

fn sst_count(dir: PathBuf) -> usize {
    fs::read_dir(dir)
        .unwrap()
        .filter(|entry| entry.as_ref().unwrap().path().extension().unwrap() == "sst")
        .count()
}

fn wal_bytes(dir: &Path) -> u64 {
    let wal = dir.join("wal");
    if !wal.is_dir() {
        return 0;
    }
    fs::read_dir(wal)
        .unwrap()
        .map(|entry| fs::metadata(entry.unwrap().path()).unwrap().len())
        .sum()
}

fn cleanup(dir: PathBuf) {
    fs::remove_dir_all(dir).expect("cleanup test dir");
}

fn fsv_root() -> PathBuf {
    std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-ph35-group-commit-fsv"))
}

fn reset_dir(dir: &PathBuf) {
    let _ = fs::remove_dir_all(dir);
    fs::create_dir_all(dir).expect("create fsv dir");
}

fn row_index(rows: &[encode::WriteRow], cf: ColumnFamily) -> usize {
    rows.iter()
        .position(|row| row.cf == cf)
        .expect("row for CF")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
