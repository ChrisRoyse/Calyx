use super::*;
use calyx_core::{
    AbsentReason, AnchorKind, AnchorValue, Constellation, CxFlags, FixedClock, InputRef, LedgerRef,
    METADATA_CHUNK_ID, METADATA_DATABASE_NAME, Modality, SlotVector, VaultId, VaultStore,
};
use std::collections::BTreeMap;

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("valid ULID")
}

fn sample_constellation(vault: &AsterVault<FixedClock>) -> Constellation {
    let input = b"issue-886-same-input";
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
    Constellation {
        cx_id,
        vault_id: vault_id(),
        panel_version: 7,
        created_at: 123,
        input_ref: InputRef {
            hash: input_hash,
            pointer: Some("synthetic://issue-886-same-input".to_string()),
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::from([(
            METADATA_DATABASE_NAME.to_string(),
            "issue_886_anchor_merge_tests".to_string(),
        )]),
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

fn reward_anchor(value: f64) -> Anchor {
    Anchor {
        kind: AnchorKind::Reward,
        value: AnchorValue::Number(value),
        source: "issue-886-test".to_string(),
        observed_at: 886,
        confidence: 1.0,
    }
}

fn label_anchor(label: &str, value: &str) -> Anchor {
    Anchor {
        kind: AnchorKind::Label(label.to_string()),
        value: AnchorValue::Text(value.to_string()),
        source: "issue-886-test".to_string(),
        observed_at: 886,
        confidence: 1.0,
    }
}

#[test]
fn duplicate_put_merges_new_anchor_into_base_and_anchor_cf() {
    let vault = AsterVault::with_clock(vault_id(), b"salt".to_vec(), FixedClock::new(123));
    let base = sample_constellation(&vault);
    let mut anchored = base.clone();
    anchored.anchors = vec![reward_anchor(1.0)];
    anchored.flags.ungrounded = false;

    vault.put(base.clone()).expect("base put");
    let seq_after_base = vault.snapshot();
    vault.put(anchored.clone()).expect("anchored duplicate put");
    let snapshot = vault.snapshot();
    let got = vault.get(base.cx_id, snapshot).expect("get merged");
    let anchor_bytes = vault
        .read_cf_at(
            snapshot,
            ColumnFamily::Anchors,
            &anchor_key(base.cx_id, &AnchorKind::Reward),
        )
        .expect("read anchor cf")
        .expect("anchor row");

    assert!(snapshot > seq_after_base);
    assert_eq!(got.anchors, anchored.anchors);
    assert!(!got.flags.ungrounded);
    assert_eq!(
        encode::decode_anchor(&anchor_bytes).unwrap(),
        anchored.anchors[0]
    );
}

#[test]
fn duplicate_put_merges_anchor_when_incoming_created_at_differs() {
    let vault = AsterVault::with_clock(vault_id(), b"salt".to_vec(), FixedClock::new(123));
    let base = sample_constellation(&vault);
    let mut anchored = base.clone();
    anchored.created_at = base.created_at + 42;
    anchored.anchors = vec![reward_anchor(1.0)];
    anchored.flags.ungrounded = false;

    vault.put(base.clone()).expect("base put");
    vault.put(anchored.clone()).expect("anchored duplicate put");
    let got = vault
        .get(base.cx_id, vault.snapshot())
        .expect("get timestamp-normalized merge");

    assert_eq!(got.created_at, base.created_at);
    assert_eq!(got.anchors, anchored.anchors);
}

#[test]
fn duplicate_put_batch_merges_multiple_new_anchor_kinds() {
    let vault = AsterVault::with_clock(vault_id(), b"salt".to_vec(), FixedClock::new(123));
    let base = sample_constellation(&vault);
    let mut first = base.clone();
    first.anchors = vec![reward_anchor(1.0)];
    first.flags.ungrounded = false;
    let mut second = base.clone();
    second.anchors = vec![label_anchor("answer", "B")];
    second.flags.ungrounded = false;

    vault.put(base.clone()).expect("base put");
    let ids = vault
        .put_batch([first, second])
        .expect("batch duplicate put");
    let snapshot = vault.snapshot();
    let got = vault.get(base.cx_id, snapshot).expect("get merged");
    let anchor_rows = vault
        .scan_cf_at(snapshot, ColumnFamily::Anchors)
        .expect("scan anchor cf");

    assert_eq!(ids, vec![base.cx_id, base.cx_id]);
    assert_eq!(got.anchors.len(), 2);
    assert!(
        got.anchors
            .iter()
            .any(|anchor| anchor.kind == AnchorKind::Reward)
    );
    assert!(
        got.anchors
            .iter()
            .any(|anchor| anchor.kind == AnchorKind::Label("answer".to_string()))
    );
    assert_eq!(anchor_rows.len(), 2);
}

#[test]
fn duplicate_put_keeps_existing_anchor_when_incoming_has_same_kind() {
    let vault = AsterVault::with_clock(vault_id(), b"salt".to_vec(), FixedClock::new(123));
    let mut base = sample_constellation(&vault);
    base.anchors = vec![reward_anchor(1.0)];
    base.flags.ungrounded = false;
    let mut duplicate = base.clone();
    duplicate.anchors[0].source = "newer-duplicate-source".to_string();
    duplicate.provenance = LedgerRef {
        seq: 99,
        hash: [99; 32],
    };

    vault.put(base.clone()).expect("base put");
    let seq_after_base = vault.snapshot();
    vault.put(duplicate).expect("compatible duplicate put");
    let got = vault
        .get(base.cx_id, vault.snapshot())
        .expect("get preserved anchor");

    assert_eq!(vault.snapshot(), seq_after_base);
    assert_eq!(got.anchors, base.anchors);
}

#[test]
fn duplicate_put_with_same_cxid_different_metadata_still_fails_closed() {
    let vault = AsterVault::with_clock(vault_id(), b"salt".to_vec(), FixedClock::new(123));
    let base = sample_constellation(&vault);
    let mut changed = base.clone();
    changed
        .metadata
        .insert(METADATA_CHUNK_ID.to_string(), "other-chunk".to_string());
    changed.anchors = vec![reward_anchor(1.0)];
    changed.flags.ungrounded = false;

    vault.put(base.clone()).expect("base put");
    let error = vault
        .put(changed)
        .expect_err("non-anchor identity mismatch rejected");
    let got = vault
        .get(base.cx_id, vault.snapshot())
        .expect("get original");

    assert_eq!(error.code, "CALYX_ASTER_CORRUPT_SHARD");
    assert!(got.anchors.is_empty());
    assert!(got.flags.ungrounded);
}
