use std::collections::BTreeMap;

use calyx_aster::cf::{ColumnFamily, base_key, slot_key};
use calyx_aster::vault::AsterVault;
use calyx_core::{
    Anchor, AnchorKind, AnchorValue, CALYX_RECORD_SCHEMA_VIOLATION, CxFlags, CxId, FixedClock,
    InputRef, LedgerRef, Modality, SlotId, SlotVector, VaultId, VaultStore,
};

#[test]
fn malformed_record_fails_before_cf_mutation() {
    let vault = AsterVault::with_clock(vault_id(), b"validation", FixedClock::new(123));
    let invalid = constellation(
        CxId::from_bytes([0xa1; 16]),
        SlotVector::Dense {
            dim: 2,
            data: vec![1.0],
        },
    );
    let before = storage_state(&vault, invalid.cx_id);

    let error = vault.put(invalid.clone()).expect_err("record rejected");
    let after = storage_state(&vault, invalid.cx_id);

    assert_eq!(error.code, CALYX_RECORD_SCHEMA_VIOLATION);
    assert_eq!(after, before);
    assert_eq!(after.snapshot, 0);
    assert!(!after.base_present);
    assert!(!after.slot_present);
}

#[test]
fn non_finite_scalar_and_anchor_fail_before_storage() {
    let vault = AsterVault::with_clock(vault_id(), b"validation", FixedClock::new(123));
    let mut scalar_bad = constellation(CxId::from_bytes([0xb1; 16]), dense(vec![1.0, 0.0]));
    scalar_bad
        .scalars
        .insert("quality".to_string(), f64::INFINITY);
    let mut anchor_bad = constellation(CxId::from_bytes([0xb2; 16]), dense(vec![1.0, 0.0]));
    anchor_bad.anchors.push(Anchor {
        kind: AnchorKind::Reward,
        value: AnchorValue::Number(1.0),
        source: "input-validation-test".to_string(),
        observed_at: 124,
        confidence: 1.1,
    });

    let scalar_error = vault.put(scalar_bad.clone()).unwrap_err();
    let anchor_error = vault.put(anchor_bad.clone()).unwrap_err();

    assert_eq!(scalar_error.code, CALYX_RECORD_SCHEMA_VIOLATION);
    assert_eq!(anchor_error.code, CALYX_RECORD_SCHEMA_VIOLATION);
    assert_eq!(storage_state(&vault, scalar_bad.cx_id).snapshot, 0);
    assert!(!storage_state(&vault, anchor_bad.cx_id).base_present);
}

#[test]
fn anchor_boundary_rejects_invalid_anchor_without_mutating_record() {
    let vault = AsterVault::with_clock(vault_id(), b"validation", FixedClock::new(123));
    let valid = constellation(CxId::from_bytes([0xc1; 16]), dense(vec![1.0, 0.0]));
    let id = valid.cx_id;
    vault.put(valid.clone()).expect("valid put");
    let before = vault.get(id, vault.snapshot()).expect("stored before");
    let bad_anchor = Anchor {
        kind: AnchorKind::Reward,
        value: AnchorValue::Number(f64::NAN),
        source: "input-validation-test".to_string(),
        observed_at: 125,
        confidence: 1.0,
    };

    let error = vault.anchor(id, bad_anchor).expect_err("anchor rejected");
    let after = vault.get(id, vault.snapshot()).expect("stored after");

    assert_eq!(error.code, CALYX_RECORD_SCHEMA_VIOLATION);
    assert_eq!(after, before);
}

#[derive(Debug, PartialEq, Eq)]
struct StorageState {
    snapshot: u64,
    base_present: bool,
    slot_present: bool,
}

fn storage_state(vault: &AsterVault<FixedClock>, cx_id: CxId) -> StorageState {
    let snapshot = vault.snapshot();
    StorageState {
        snapshot,
        base_present: vault
            .read_cf_at(snapshot, ColumnFamily::Base, &base_key(cx_id))
            .unwrap()
            .is_some(),
        slot_present: vault
            .read_cf_at(snapshot, ColumnFamily::slot(slot()), &slot_key(cx_id))
            .unwrap()
            .is_some(),
    }
}

fn constellation(cx_id: CxId, vector: SlotVector) -> calyx_core::Constellation {
    let mut slots = BTreeMap::new();
    slots.insert(slot(), vector);
    calyx_core::Constellation {
        cx_id,
        vault_id: vault_id(),
        panel_version: 1,
        created_at: 123,
        input_ref: InputRef {
            hash: [1; 32],
            pointer: Some(format!("synthetic://input-validation/{cx_id}")),
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 1,
            hash: [2; 32],
        },
        flags: CxFlags::default(),
    }
}

fn dense(data: Vec<f32>) -> SlotVector {
    SlotVector::Dense {
        dim: data.len() as u32,
        data,
    }
}

const fn slot() -> SlotId {
    SlotId::new(8)
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}
