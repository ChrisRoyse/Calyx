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

    let mut expected = cx;
    expected.provenance = got.provenance.clone();
    assert_eq!(got, expected);
    assert_ne!(got.provenance.hash, [0; 32]);
    assert!(matches!(
        got.slots.get(&SlotId::new(1)),
        Some(SlotVector::Absent {
            reason: AbsentReason::LensUnavailable
        })
    ));
}

#[test]
fn put_outcomes_are_authoritative_and_ordered_without_post_read() {
    let vault = AsterVault::with_clock(
        vault_id(),
        b"issue1547-outcomes".to_vec(),
        FixedClock::new(1_547),
    );
    let cx = sample_constellation(&vault);
    let id = cx.cx_id;
    let before = vault.latest_seq();
    eprintln!("ISSUE1547_SINGLE before_seq={before} base_present=false");
    let inserted = vault.put_with_outcome(cx.clone()).unwrap();
    assert_eq!(inserted.cx_id, id);
    assert_eq!(inserted.disposition, PutDisposition::Inserted);
    let inserted_seq = vault.latest_seq();
    let persisted = vault.get(id, inserted_seq).unwrap();
    assert_eq!(persisted.cx_id, id);
    assert!(
        vault
            .read_cf_at(inserted_seq, ColumnFamily::Base, &base_key(id))
            .unwrap()
            .is_some()
    );
    eprintln!(
        "ISSUE1547_SINGLE after_seq={inserted_seq} disposition={:?} base_present=true",
        inserted.disposition
    );

    let duplicate = vault.put_with_outcome(cx).unwrap();
    assert_eq!(duplicate.disposition, PutDisposition::ExistingIdentical);
    assert_eq!(vault.latest_seq(), inserted_seq);
    eprintln!(
        "ISSUE1547_DUPLICATE before_seq={inserted_seq} after_seq={} disposition={:?}",
        vault.latest_seq(),
        duplicate.disposition
    );

    let second = sample_constellation(&AsterVault::with_clock(
        vault_id(),
        b"different-id".to_vec(),
        FixedClock::new(1_547),
    ));
    let outcomes = vault
        .put_batch_with_outcomes([second.clone(), second.clone()])
        .unwrap();
    assert_eq!(outcomes.len(), 2);
    assert_eq!(outcomes[0].disposition, PutDisposition::Inserted);
    assert_eq!(
        outcomes[1].disposition,
        PutDisposition::InBatchDuplicate { anchors_added: 0 }
    );
    let batch_readback = vault.get(second.cx_id, vault.latest_seq()).unwrap();
    assert_eq!(batch_readback.cx_id, second.cx_id);
    eprintln!(
        "ISSUE1547_BATCH outcomes={:?} persisted_cx_id={}",
        outcomes, batch_readback.cx_id
    );
}

#[test]
fn observation_put_retains_first_payload_while_strict_put_rejects_differences() {
    let vault = AsterVault::with_clock(
        vault_id(),
        b"issue1547-observation-policy".to_vec(),
        FixedClock::new(1_547),
    );
    let first = sample_constellation(&vault);
    let id = first.cx_id;
    assert_eq!(
        vault.put_with_outcome(first).unwrap().disposition,
        PutDisposition::Inserted
    );
    let stored_before = vault.get(id, vault.latest_seq()).unwrap();

    let mut repeated = stored_before.clone();
    repeated.created_at += 1;
    repeated.input_ref.pointer = Some("synthetic://later-observation".to_string());
    repeated
        .metadata
        .insert(METADATA_CHUNK_ID.to_string(), "later-chunk".to_string());
    repeated.scalars.insert("tokens".to_string(), 99.0);
    let seq_before = vault.latest_seq();
    let strict_error = vault.put_with_outcome(repeated.clone()).unwrap_err();
    assert_eq!(strict_error.code, "CALYX_ASTER_CORRUPT_SHARD");
    assert_eq!(vault.latest_seq(), seq_before);
    assert_eq!(vault.get(id, seq_before).unwrap(), stored_before);

    let observation = vault.put_observation_with_outcome(repeated).unwrap();
    assert_eq!(observation.disposition, PutDisposition::ExistingIdentical);
    assert_eq!(vault.latest_seq(), seq_before);
    assert_eq!(vault.get(id, seq_before).unwrap(), stored_before);

    let mut invalid_identity = stored_before.clone();
    invalid_identity.input_ref.hash[0] ^= 0xff;
    let identity_error = vault
        .put_observation_with_outcome(invalid_identity)
        .unwrap_err();
    assert_eq!(identity_error.code, "CALYX_ASTER_CORRUPT_SHARD");
    assert!(identity_error.message.contains("input_hash"));
    assert_eq!(vault.latest_seq(), seq_before);
    assert_eq!(vault.get(id, seq_before).unwrap(), stored_before);
}

#[test]
fn observation_batch_returns_ordered_insert_and_in_batch_duplicate_outcomes() {
    let vault = AsterVault::with_clock(
        vault_id(),
        b"issue1547-observation-batch".to_vec(),
        FixedClock::new(1_547),
    );
    let first = sample_constellation(&vault);
    let mut repeated = first.clone();
    repeated.created_at += 1;
    repeated.input_ref.pointer = Some("synthetic://later-batch-observation".to_string());
    repeated.metadata.insert(
        METADATA_CHUNK_ID.to_string(),
        "later-batch-chunk".to_string(),
    );

    let outcomes = vault
        .put_observation_batch_with_outcomes([first.clone(), repeated])
        .unwrap();
    assert_eq!(outcomes.len(), 2);
    assert_eq!(outcomes[0].disposition, PutDisposition::Inserted);
    assert_eq!(
        outcomes[1].disposition,
        PutDisposition::InBatchDuplicate { anchors_added: 0 }
    );
    let stored = vault.get(first.cx_id, vault.latest_seq()).unwrap();
    assert_eq!(stored.input_ref.pointer, first.input_ref.pointer);
    assert_eq!(stored.metadata, first.metadata);
}

#[test]
fn batch_reads_base_once_per_unique_identity_from_one_snapshot() {
    let vault = AsterVault::with_clock(
        vault_id(),
        b"issue1547-unique-base-reads".to_vec(),
        FixedClock::new(1_547),
    );
    let existing = sample_constellation(&vault);
    vault.put(existing.clone()).unwrap();
    let inserted = sample_constellation(&AsterVault::with_clock(
        vault_id(),
        b"issue1547-new-identity".to_vec(),
        FixedClock::new(1_547),
    ));

    batch_ingest::reset_batch_read_counts();
    let outcomes = vault
        .put_batch_with_outcomes([
            inserted.clone(),
            inserted.clone(),
            existing.clone(),
            existing,
        ])
        .unwrap();

    assert_eq!(batch_ingest::batch_read_counts(), (2, 1));
    assert_eq!(outcomes[0].disposition, PutDisposition::Inserted);
    assert_eq!(
        outcomes[1].disposition,
        PutDisposition::InBatchDuplicate { anchors_added: 0 }
    );
    assert_eq!(outcomes[2].disposition, PutDisposition::ExistingIdentical);
    assert_eq!(outcomes[3].disposition, PutDisposition::ExistingIdentical);
    assert!(vault.get(inserted.cx_id, vault.latest_seq()).is_ok());
    eprintln!("ISSUE1547_IO_COUNTS inputs=4 unique_ids=2 base_lookups=2 snapshot_pins=1");
}

#[test]
fn batch_ingest_serializes_and_hashes_each_slot_once() {
    let vault = AsterVault::with_clock(
        vault_id(),
        b"prepared-counts".to_vec(),
        FixedClock::new(123),
    );
    let cx = sample_constellation(&vault);
    let slot_count = cx.slots.len();
    encode::reset_slot_operation_counts();

    vault.put_batch([cx]).expect("prepared batch ingest");

    assert_eq!(
        encode::slot_operation_counts(),
        (slot_count, slot_count),
        "accepted batch rows must encode and hash every slot exactly once"
    );
}

#[test]
#[ignore = "manual full-state verification for issue #1530"]
fn issue1530_manual_fsv_reads_back_durable_prepared_batch_rows() {
    let root = std::env::var_os("CALYX_ISSUE1530_FSV_ROOT")
        .map(PathBuf::from)
        .expect("set CALYX_ISSUE1530_FSV_ROOT to a fresh path");
    assert!(!root.exists(), "FSV root must be fresh: {}", root.display());
    let vault = AsterVault::new_durable_with_clock(
        &root,
        vault_id(),
        b"issue1530-prepared-batch-fsv".to_vec(),
        VaultOptions::default(),
        FixedClock::new(1_530_000),
    )
    .expect("open durable FSV vault");
    let before = serde_json::json!({
        "base": vault.scan_cf_at(vault.latest_seq(), ColumnFamily::Base).unwrap().len(),
        "slot0": vault.scan_cf_at(vault.latest_seq(), ColumnFamily::slot(SlotId::new(0))).unwrap().len(),
        "slot1": vault.scan_cf_at(vault.latest_seq(), ColumnFamily::slot(SlotId::new(1))).unwrap().len(),
    });

    encode::reset_slot_operation_counts();
    vault
        .put_batch(Vec::<Constellation>::new())
        .expect("empty batch is a no-op");
    let empty_state = serde_json::json!({
        "counts": encode::slot_operation_counts(),
        "latest_seq": vault.latest_seq(),
        "base_rows": vault.scan_cf_at(vault.latest_seq(), ColumnFamily::Base).unwrap().len(),
    });

    let mut invalid = sample_constellation(&vault);
    invalid.slots.insert(
        SlotId::new(0),
        SlotVector::Sparse {
            dim: 4,
            entries: vec![
                calyx_core::SparseEntry { idx: 1, val: 0.25 },
                calyx_core::SparseEntry { idx: 1, val: 0.75 },
            ],
        },
    );
    let invalid_error = vault
        .put_batch([invalid])
        .expect_err("duplicate sparse indices fail closed");
    let invalid_state = serde_json::json!({
        "error_code": invalid_error.code,
        "latest_seq": vault.latest_seq(),
        "base_rows": vault.scan_cf_at(vault.latest_seq(), ColumnFamily::Base).unwrap().len(),
    });

    let cx = sample_constellation(&vault);
    let cx_id = cx.cx_id;
    let slot_count = cx.slots.len();
    encode::reset_slot_operation_counts();
    vault.put_batch([cx]).expect("prepared batch ingest");
    let operation_counts = encode::slot_operation_counts();
    assert_eq!(operation_counts, (slot_count, slot_count));
    vault.flush().expect("flush physical SST rows");
    let after = serde_json::json!({
        "latest_seq": vault.latest_seq(),
        "encode_count": operation_counts.0,
        "hash_count": operation_counts.1,
        "base": vault.scan_cf_at(vault.latest_seq(), ColumnFamily::Base).unwrap().len(),
        "slot0": vault.scan_cf_at(vault.latest_seq(), ColumnFamily::slot(SlotId::new(0))).unwrap().len(),
        "slot1": vault.scan_cf_at(vault.latest_seq(), ColumnFamily::slot(SlotId::new(1))).unwrap().len(),
    });
    encode::reset_slot_operation_counts();
    vault
        .put_batch([sample_constellation(&vault)])
        .expect("duplicate batch row resolves idempotently");
    let duplicate_state = serde_json::json!({
        "counts": encode::slot_operation_counts(),
        "base_rows": vault.scan_cf_at(vault.latest_seq(), ColumnFamily::Base).unwrap().len(),
        "slot0_rows": vault.scan_cf_at(vault.latest_seq(), ColumnFamily::slot(SlotId::new(0))).unwrap().len(),
    });
    drop(vault);

    let reopened = AsterVault::open_with_clock(
        &root,
        vault_id(),
        b"issue1530-prepared-batch-fsv".to_vec(),
        VaultOptions::default(),
        FixedClock::new(1_530_001),
    )
    .expect("reopen durable FSV vault");
    let stored = reopened
        .get(cx_id, reopened.snapshot())
        .expect("read back constellation from physical state");
    assert_eq!(stored.slots.len(), slot_count);
    let physical_files = fs::read_dir(root.join("cf"))
        .expect("read physical CF root")
        .flat_map(|entry| {
            let entry = entry.expect("CF directory");
            fs::read_dir(entry.path())
                .into_iter()
                .flatten()
                .map(|file| file.expect("CF file").path().display().to_string())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let report = serde_json::json!({
        "issue": 1530,
        "source_of_truth": root.display().to_string(),
        "before": before,
        "edge_empty_after": empty_state,
        "edge_invalid_after": invalid_state,
        "edge_duplicate_after": duplicate_state,
        "happy_after": after,
        "reopened": {
            "cx_id": stored.cx_id.to_string(),
            "slot_count": stored.slots.len(),
            "provenance_seq": stored.provenance.seq,
        },
        "physical_files": physical_files,
    });
    let artifact = root.join("issue1530-fsv.json");
    fs::write(&artifact, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    let persisted: serde_json::Value =
        serde_json::from_slice(&fs::read(&artifact).unwrap()).unwrap();
    assert_eq!(persisted["happy_after"]["base"], 1);
    println!(
        "ISSUE1530_FSV={}",
        serde_json::to_string(&persisted).unwrap()
    );
}

#[test]
fn prepared_base_and_slot_rows_are_byte_identical_to_canonical_codecs() {
    let vault = AsterVault::with_clock(
        vault_id(),
        b"prepared-parity".to_vec(),
        FixedClock::new(123),
    );
    let mut cx = sample_constellation(&vault);
    cx.slots.insert(
        SlotId::new(2),
        SlotVector::Sparse {
            dim: 8,
            entries: vec![
                calyx_core::SparseEntry { idx: 7, val: -0.25 },
                calyx_core::SparseEntry { idx: 1, val: 0.5 },
            ],
        },
    );
    cx.slots.insert(
        SlotId::new(3),
        SlotVector::Multi {
            token_dim: 2,
            tokens: vec![vec![0.1, 0.2], vec![0.3, 0.4]],
        },
    );
    cx.validate_schema().expect("fixture schema");
    let canonical_base = encode::encode_constellation_base(&cx).expect("canonical base");
    let expected_slots = cx
        .slots
        .iter()
        .map(|(slot, vector)| {
            (
                *slot,
                encode::encode_slot_vector(vector).expect("slot bytes"),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let prepared = prepared::PreparedConstellationEncoding::new(&cx).expect("prepare slots");
    let mut rows = Vec::new();

    prepared::stage_validated_constellation_rows(&mut rows, &cx, prepared)
        .expect("stage prepared rows");

    assert_eq!(
        rows.iter()
            .find(|row| row.cf == ColumnFamily::Base)
            .expect("base row")
            .value,
        canonical_base
    );
    for (slot, expected) in expected_slots {
        let actual = rows
            .iter()
            .find(|row| row.cf == ColumnFamily::slot(slot))
            .expect("prepared slot row");
        assert_eq!(actual.value, expected);
    }
}

mod durability_fsv;
mod selected_and_durable;

mod read_only;

mod derived_content_watermark;
mod read_leases;
mod support;
mod write_latest_readback;
use support::{cleanup, fsv_root, hex, reset_dir, row_index, sst_count, test_dir, wal_bytes};
