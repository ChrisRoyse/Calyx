use super::*;
use crate::cf::{XTermKind, recurrence_key, scalar_key, slot_key, temporal_xterm_key, xterm_key};
use crate::mvcc::is_tombstone_value;
use calyx_core::{
    Clock, CxFlags, FixedClock, InputRef, LedgerRef, Modality, SlotId, SlotVector, VaultStore,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use ulid::Ulid;

fn vault_id() -> VaultId {
    VaultId::from_ulid(Ulid::from_bytes([0xAB; 16]))
}

fn context() -> VaultContext {
    VaultContext::new(
        vault_id(),
        b"erase-test-master-key-material",
        crate::vault::QuotaConfig::default(),
        "tank/calyx",
    )
    .unwrap()
}

fn cx<C>(vault: &AsterVault<C>, seed: &'static [u8], subject: Option<&SubjectId>) -> Constellation
where
    C: Clock,
{
    let id = vault.cx_id_for_input(seed, 1);
    let mut hash = [0_u8; 32];
    hash[..seed.len()].copy_from_slice(seed);
    let mut slots = BTreeMap::new();
    slots.insert(
        SlotId::new(0),
        SlotVector::Dense {
            dim: 2,
            data: vec![seed[0] as f32, 1.0],
        },
    );
    let mut metadata = BTreeMap::new();
    if let Some(subject) = subject {
        metadata.insert(
            METADATA_SUBJECT_ID.to_string(),
            subject_metadata_value(subject),
        );
    }
    Constellation {
        cx_id: id,
        vault_id: vault_id(),
        panel_version: 1,
        created_at: 123,
        input_ref: InputRef {
            hash,
            pointer: Some(format!("synthetic://{}", String::from_utf8_lossy(seed))),
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata,
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 1,
            hash: [seed[0]; 32],
        },
        flags: CxFlags {
            ungrounded: true,
            ..CxFlags::default()
        },
    }
}

#[test]
fn erase_cx_tombstones_base_slots_and_derived_rows() {
    let vault = AsterVault::with_clock(vault_id(), b"salt".to_vec(), FixedClock::new(777));
    let mut ctx = context();
    let registry = EraseRegistry::new();
    let first = cx(&vault, b"first", None);
    let second = cx(&vault, b"second", None);
    let first_id = first.cx_id;
    let second_id = second.cx_id;
    vault.put(first.clone()).unwrap();
    vault.put(second.clone()).unwrap();
    vault
        .write_cf(
            ColumnFamily::slot_raw(SlotId::new(0)),
            slot_key(first_id),
            b"raw-slot".to_vec(),
        )
        .unwrap();
    vault
        .write_cf(
            ColumnFamily::XTerm,
            xterm_key(first_id, SlotId::new(0), SlotId::new(1), XTermKind::Concat),
            b"xterm".to_vec(),
        )
        .unwrap();
    vault
        .write_cf(
            ColumnFamily::Recurrence,
            recurrence_key(first_id, 7),
            b"rec".to_vec(),
        )
        .unwrap();
    vault
        .write_cf(
            ColumnFamily::TemporalXTerm,
            temporal_xterm_key(second_id, first_id),
            b"temporal-right".to_vec(),
        )
        .unwrap();
    vault
        .write_cf(
            ColumnFamily::Scalars,
            scalar_key(crate::cf::ScalarId::new(3), first_id),
            b"scalar".to_vec(),
        )
        .unwrap();

    let result = vault
        .erase(EraseScope::Cx(first_id), &mut ctx, &registry)
        .unwrap();

    assert_eq!(result.records_deleted, 1);
    assert_eq!(result.shredded_at, 777);
    assert_eq!(
        vault
            .read_cf_at(vault.snapshot(), ColumnFamily::Base, &base_key(first_id))
            .unwrap(),
        None
    );
    assert!(vault.get(second_id, vault.snapshot()).is_ok());
    assert_eq!(
        vault
            .read_cf_at(
                vault.snapshot(),
                ColumnFamily::slot_raw(SlotId::new(0)),
                &slot_key(first_id)
            )
            .unwrap(),
        None
    );
    assert_eq!(
        vault
            .read_cf_at(
                vault.snapshot(),
                ColumnFamily::TemporalXTerm,
                &temporal_xterm_key(second_id, first_id)
            )
            .unwrap(),
        None
    );
    assert!(ctx.is_key_shredded_for_erasure());
}

#[test]
fn erase_vault_counts_constellations_and_shreds_key() {
    let vault = AsterVault::with_clock(vault_id(), b"salt".to_vec(), FixedClock::new(888));
    let mut ctx = context();
    let registry = EraseRegistry::new();
    for seed in [b"one".as_slice(), b"two".as_slice(), b"three".as_slice()] {
        vault.put(cx(&vault, seed, None)).unwrap();
    }
    let ciphertext = ctx.encrypt_value(b"erase-me", b"aad").unwrap();

    let result = vault.erase(EraseScope::Vault, &mut ctx, &registry).unwrap();

    assert_eq!(result.records_deleted, 3);
    assert!(ctx.is_key_shredded_for_erasure());
    assert_eq!(
        ctx.decrypt_value(&ciphertext, b"aad").unwrap_err().code,
        "CALYX_DECRYPTION_FAILED"
    );
    assert!(
        vault
            .scan_cf_at(vault.snapshot(), ColumnFamily::Base)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn erase_subject_is_scope_exact() {
    let vault = AsterVault::with_clock(vault_id(), b"salt".to_vec(), FixedClock::new(999));
    let mut ctx = context();
    let registry = EraseRegistry::new();
    let subject = SubjectId::Query(b"subject-alpha".to_vec());
    let a = cx(&vault, b"sub-a", Some(&subject));
    let b = cx(&vault, b"sub-b", Some(&subject));
    let c = cx(&vault, b"other", None);
    let c_id = c.cx_id;
    vault.put(a).unwrap();
    vault.put(b).unwrap();
    vault.put(c).unwrap();

    let result = vault
        .erase(EraseScope::Subject(subject), &mut ctx, &registry)
        .unwrap();

    assert_eq!(result.records_deleted, 2);
    assert_eq!(
        vault
            .scan_cf_at(vault.snapshot(), ColumnFamily::Base)
            .unwrap()
            .len(),
        1
    );
    assert!(vault.get(c_id, vault.snapshot()).is_ok());
}

#[test]
fn unknown_cx_is_idempotent_and_preserves_key() {
    let vault = AsterVault::with_clock(vault_id(), b"salt".to_vec(), FixedClock::new(111));
    let mut ctx = context();
    let registry = EraseRegistry::new();
    let ciphertext = ctx.encrypt_value(b"still-readable", b"aad").unwrap();

    let result = vault
        .erase(
            EraseScope::Cx(CxId::from_bytes([0x55; 16])),
            &mut ctx,
            &registry,
        )
        .unwrap();

    assert_eq!(result.records_deleted, 0);
    assert!(!ctx.is_key_shredded_for_erasure());
    assert_eq!(
        ctx.decrypt_value(&ciphertext, b"aad").unwrap(),
        b"still-readable"
    );
}

#[test]
fn failing_handler_propagates_before_shredding() {
    struct Failing;
    impl EraseHandler for Failing {
        fn erase(&self, _scope: &EraseScope, _vault_id: VaultId) -> Result<()> {
            Err(CalyxError::aster_corrupt_shard("derived handler failed"))
        }
    }

    let vault = AsterVault::with_clock(vault_id(), b"salt".to_vec(), FixedClock::new(222));
    let mut ctx = context();
    let mut registry = EraseRegistry::new();
    registry.add_handler(Failing);
    let stored = cx(&vault, b"fail", None);
    let id = stored.cx_id;
    vault.put(stored).unwrap();
    let ciphertext = ctx.encrypt_value(b"key-survives", b"aad").unwrap();

    let err = vault
        .erase(EraseScope::Cx(id), &mut ctx, &registry)
        .unwrap_err();

    assert_eq!(err.code, "CALYX_ASTER_CORRUPT_SHARD");
    assert!(!ctx.is_key_shredded_for_erasure());
    assert!(
        vault
            .read_cf_at(vault.snapshot(), ColumnFamily::Base, &base_key(id))
            .unwrap()
            .is_some()
    );
    assert_eq!(
        ctx.decrypt_value(&ciphertext, b"aad").unwrap(),
        b"key-survives"
    );
}

#[test]
#[ignore = "manual FSV for PH61 T01 erase byte readback"]
fn ph61_t01_erase_manual_fsv() {
    struct Failing;
    impl EraseHandler for Failing {
        fn erase(&self, _scope: &EraseScope, _vault_id: VaultId) -> Result<()> {
            Err(CalyxError::aster_corrupt_shard("derived handler failed"))
        }
    }

    let root = fsv_root().join("issue502-erase");
    reset_dir(&root);
    let registry = EraseRegistry::new();

    let happy_dir = root.join("happy").join("vault");
    let happy = durable_vault(&happy_dir);
    let mut happy_ctx = context();
    let first = cx(&happy, b"fsv-first", None);
    let second = cx(&happy, b"fsv-second", None);
    let third = cx(&happy, b"fsv-third", None);
    let first_id = first.cx_id;
    let second_id = second.cx_id;
    happy.put(first).unwrap();
    happy.put(second).unwrap();
    happy.put(third).unwrap();
    happy
        .write_cf(
            ColumnFamily::slot_raw(SlotId::new(0)),
            slot_key(first_id),
            b"FSV_RAW_SLOT_502".to_vec(),
        )
        .unwrap();
    happy
        .write_cf(
            ColumnFamily::Recurrence,
            recurrence_key(first_id, 502),
            b"FSV_RECURRENCE_502".to_vec(),
        )
        .unwrap();
    let ciphertext = happy_ctx
        .encrypt_value(b"FSV_ERASE_SENTINEL_502", b"issue502")
        .unwrap();
    let happy_before_base = happy
        .scan_cf_at(happy.snapshot(), ColumnFamily::Base)
        .unwrap()
        .len();
    let happy_before_target = happy
        .read_cf_at(happy.snapshot(), ColumnFamily::Base, &base_key(first_id))
        .unwrap()
        .is_some();
    let happy_result = happy
        .erase(EraseScope::Cx(first_id), &mut happy_ctx, &registry)
        .unwrap();
    happy.flush().unwrap();
    let replay = crate::wal::replay_dir(happy_dir.join("wal")).unwrap();
    let tombstone_rows = replay
        .records
        .iter()
        .flat_map(|record| encode::decode_write_batch(&record.payload).unwrap())
        .filter(|row| is_tombstone_value(&row.value))
        .count();

    let unknown_dir = root.join("unknown").join("vault");
    let unknown = durable_vault(&unknown_dir);
    let mut unknown_ctx = context();
    let unknown_ct = unknown_ctx
        .encrypt_value(b"UNKNOWN_EDGE_KEY_SURVIVES", b"issue502")
        .unwrap();
    let unknown_result = unknown
        .erase(
            EraseScope::Cx(CxId::from_bytes([0xEE; 16])),
            &mut unknown_ctx,
            &registry,
        )
        .unwrap();

    let empty_dir = root.join("empty-vault").join("vault");
    let empty = durable_vault(&empty_dir);
    let mut empty_ctx = context();
    let empty_ct = empty_ctx
        .encrypt_value(b"EMPTY_VAULT_KEY_SHRED", b"issue502")
        .unwrap();
    let empty_result = empty
        .erase(EraseScope::Vault, &mut empty_ctx, &registry)
        .unwrap();

    let failing_dir = root.join("failing-handler").join("vault");
    let failing = durable_vault(&failing_dir);
    let mut failing_ctx = context();
    let mut failing_registry = EraseRegistry::new();
    failing_registry.add_handler(Failing);
    let fail_cx = cx(&failing, b"fsv-fail", None);
    let fail_id = fail_cx.cx_id;
    failing.put(fail_cx).unwrap();
    let failing_ct = failing_ctx
        .encrypt_value(b"FAILING_HANDLER_KEY_SURVIVES", b"issue502")
        .unwrap();
    let failing_error = failing
        .erase(EraseScope::Cx(fail_id), &mut failing_ctx, &failing_registry)
        .unwrap_err();
    let cf_sentinel_hits = sentinel_hits(
        &happy_dir.join("cf"),
        &[b"FSV_RAW_SLOT_502", b"FSV_RECURRENCE_502"],
    );

    let readback = serde_json::json!({
        "happy": {
            "before_base_count": happy_before_base,
            "before_target_base_present": happy_before_target,
            "records_deleted": happy_result.records_deleted,
            "after_base_count": happy.scan_cf_at(happy.snapshot(), ColumnFamily::Base).unwrap().len(),
            "after_target_base_present": happy.read_cf_at(happy.snapshot(), ColumnFamily::Base, &base_key(first_id)).unwrap().is_some(),
            "survivor_second_present": happy.get(second_id, happy.snapshot()).is_ok(),
            "raw_slot_after_present": happy.read_cf_at(happy.snapshot(), ColumnFamily::slot_raw(SlotId::new(0)), &slot_key(first_id)).unwrap().is_some(),
            "key_shredded": happy_ctx.is_key_shredded_for_erasure(),
            "decrypt_after_code": happy_ctx.decrypt_value(&ciphertext, b"issue502").unwrap_err().code,
            "wal_record_count": replay.records.len(),
            "wal_tombstone_rows": tombstone_rows,
            "cf_sentinel_hits_after": &cf_sentinel_hits,
        },
        "unknown_cx_edge": {
            "records_deleted": unknown_result.records_deleted,
            "after_base_count": unknown.scan_cf_at(unknown.snapshot(), ColumnFamily::Base).unwrap().len(),
            "key_shredded": unknown_ctx.is_key_shredded_for_erasure(),
            "decrypt_after": String::from_utf8(unknown_ctx.decrypt_value(&unknown_ct, b"issue502").unwrap()).unwrap(),
        },
        "empty_vault_edge": {
            "records_deleted": empty_result.records_deleted,
            "after_base_count": empty.scan_cf_at(empty.snapshot(), ColumnFamily::Base).unwrap().len(),
            "key_shredded": empty_ctx.is_key_shredded_for_erasure(),
            "decrypt_after_code": empty_ctx.decrypt_value(&empty_ct, b"issue502").unwrap_err().code,
        },
        "failing_handler_edge": {
            "error_code": failing_error.code,
            "key_shredded": failing_ctx.is_key_shredded_for_erasure(),
            "decrypt_after": String::from_utf8(failing_ctx.decrypt_value(&failing_ct, b"issue502").unwrap()).unwrap(),
            "target_base_after_present": failing.read_cf_at(failing.snapshot(), ColumnFamily::Base, &base_key(fail_id)).unwrap().is_some(),
        },
        "paths": {
            "root": root,
            "happy_wal": happy_dir.join("wal").join("00000000000000000000.wal"),
        }
    });
    let readback_path = root.join("issue502-erase-readback.json");
    fs::write(
        &readback_path,
        serde_json::to_vec_pretty(&readback).unwrap(),
    )
    .unwrap();

    println!("ISSUE502_ERASE_FSV_ROOT={}", root.display());
    println!("ISSUE502_ERASE_READBACK={}", readback_path.display());
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    assert_eq!(happy_result.records_deleted, 1);
    assert!(tombstone_rows >= 3);
    assert_eq!(unknown_result.records_deleted, 0);
    assert_eq!(empty_result.records_deleted, 0);
    assert_eq!(failing_error.code, "CALYX_ASTER_CORRUPT_SHARD");
    assert!(cf_sentinel_hits.is_empty(), "{cf_sentinel_hits:?}");
    assert!(
        failing
            .read_cf_at(failing.snapshot(), ColumnFamily::Base, &base_key(fail_id))
            .unwrap()
            .is_some()
    );
}

fn sentinel_hits(root: &Path, needles: &[&[u8]]) -> Vec<String> {
    let mut hits = Vec::new();
    collect_sentinel_hits(root, needles, &mut hits);
    hits.sort();
    hits
}

fn collect_sentinel_hits(dir: &Path, needles: &[&[u8]], hits: &mut Vec<String>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_sentinel_hits(&path, needles, hits);
            continue;
        }
        if path.extension().and_then(|value| value.to_str()) != Some("sst") {
            continue;
        }
        let bytes = fs::read(&path).unwrap();
        if needles
            .iter()
            .any(|needle| bytes.windows(needle.len()).any(|window| window == *needle))
        {
            hits.push(path.display().to_string());
        }
    }
}

fn durable_vault(dir: &PathBuf) -> AsterVault {
    AsterVault::new_durable(
        dir,
        vault_id(),
        b"salt".to_vec(),
        crate::vault::VaultOptions::default(),
    )
    .unwrap()
}

fn fsv_root() -> PathBuf {
    std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-issue502-fsv"))
}

fn reset_dir(dir: &PathBuf) {
    let _ = fs::remove_dir_all(dir);
    fs::create_dir_all(dir).unwrap();
}
