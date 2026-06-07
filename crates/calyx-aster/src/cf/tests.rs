use super::*;
use calyx_core::{AnchorKind, CxId, SlotId};

fn cx(byte: u8) -> CxId {
    CxId::from_bytes([byte; 16])
}

#[test]
fn column_family_names_match_prd_layout() {
    let static_names: Vec<_> = ColumnFamily::STATIC
        .iter()
        .map(ColumnFamily::name)
        .collect();
    assert_eq!(
        static_names,
        ["base", "xterm", "scalars", "anchors", "ledger", "online"]
    );

    let slot = ColumnFamily::slot(SlotId::new(7));
    let raw = ColumnFamily::slot_raw(SlotId::new(7));

    assert_eq!(slot.name(), "slot_07");
    assert_eq!(raw.name(), "slot_07.raw");
    assert!(slot.is_slot());
    assert!(raw.is_raw_slot());
    assert_eq!(raw.slot_id(), Some(SlotId::new(7)));
}

#[test]
fn keys_use_big_endian_ordering_for_range_scans() {
    let cx_id = cx(1);

    assert_eq!(base_key(cx_id), vec![1; 16]);
    assert!(ledger_key(9) < ledger_key(10));
    assert!(online_key(OnlineKeyKind::MistakeLog, 9) < online_key(OnlineKeyKind::MistakeLog, 10));
    assert!(scalar_key(ScalarId::new(1), cx_id) < scalar_key(ScalarId::new(2), cx_id));
    assert!(
        xterm_key(cx_id, SlotId::new(1), SlotId::new(9), XTermKind::Concat)
            < xterm_key(cx_id, SlotId::new(1), SlotId::new(10), XTermKind::Concat)
    );
    assert!(anchor_key(cx_id, &AnchorKind::TestPass) < anchor_key(cx_id, &AnchorKind::Reward));
}

#[test]
fn prefix_ranges_include_only_matching_key_prefixes() {
    let cx_a = cx(0x10);
    let cx_b = cx(0x11);
    let range = anchor_prefix_range(cx_a);

    assert!(range.contains(&anchor_key(cx_a, &AnchorKind::Label("gold".to_string()))));
    assert!(range.contains(&anchor_key(cx_a, &AnchorKind::Reward)));
    assert!(!range.contains(&anchor_key(cx_b, &AnchorKind::Reward)));

    let scalar = scalar_prefix_range(ScalarId::new(42));
    assert!(scalar.contains(&scalar_key(ScalarId::new(42), cx_a)));
    assert!(!scalar.contains(&scalar_key(ScalarId::new(43), cx_a)));

    let open_ended = prefix_range(&[0xff, 0xff]);
    assert_eq!(open_ended.end, None);
    assert!(open_ended.contains(&[0xff, 0xff, 0x00]));
}

#[test]
fn ledger_range_is_half_open() {
    let range = ledger_range(100, 103);

    assert!(range.contains(&ledger_key(100)));
    assert!(range.contains(&ledger_key(102)));
    assert!(!range.contains(&ledger_key(99)));
    assert!(!range.contains(&ledger_key(103)));
}

#[test]
fn hash_prefix_mismatch_fails_closed() {
    let panel = 7_u32.to_be_bytes();
    let full = full_content_hash([
        b"synthetic-input".as_slice(),
        panel.as_slice(),
        b"salt".as_slice(),
    ]);
    let cx_id = cx_id_from_full_hash(&full);

    verify_cx_hash_prefix(cx_id, &full).expect("hash prefix matches");

    let mut altered = full;
    altered[0] ^= 0xff;
    let error = verify_cx_hash_prefix(cx_id, &altered).expect_err("altered hash rejected");

    assert_eq!(error.code, "CALYX_ASTER_CORRUPT_SHARD");
}
