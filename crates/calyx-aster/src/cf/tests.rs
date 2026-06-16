use super::*;
use calyx_core::{AnchorKind, CxId, SlotId};
use proptest::prelude::*;

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
        [
            "base",
            "collections",
            "relational",
            "xterm",
            "temporal_xterm",
            "scalars",
            "anchors",
            "assay",
            "ledger",
            "recurrence",
            "graph",
            "online",
            "reactive",
            "anneal_rollback",
            "anneal_health",
            "anneal_checksums",
            "anneal_mistakes",
            "anneal_replay",
            "anneal_heads",
            "anneal_bandit",
            "anneal_soak",
            "anneal_report",
            "anneal_growth",
            "time_index",
            "document",
            "kv",
            "timeseries",
            "blob",
            "index_btree",
            "index_inverted",
            "anneal_operators",
            "kernel",
            "guard",
        ]
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
    assert_eq!(ledger_key(1), vec![0, 0, 0, 0, 0, 0, 0, 1]);
    assert_eq!(ledger_key(u64::MAX), vec![0xff; 8]);
    assert_eq!(
        recurrence_key(cx_id, 7),
        [vec![1; 16], vec![0, 0, 0, 0, 0, 0, 0, 7]].concat()
    );
    println!("LEDGER_KEY_1 {}", hex_bytes(&ledger_key(1)));
    println!("LEDGER_KEY_MAX {}", hex_bytes(&ledger_key(u64::MAX)));
    assert!(ledger_key(9) < ledger_key(10));
    assert!(recurrence_key(cx_id, 9) < recurrence_key(cx_id, 10));
    assert!(online_key(OnlineKeyKind::MistakeLog, 9) < online_key(OnlineKeyKind::MistakeLog, 10));
    assert!(scalar_key(ScalarId::new(1), cx_id) < scalar_key(ScalarId::new(2), cx_id));
    assert!(
        xterm_key(cx_id, SlotId::new(1), SlotId::new(9), XTermKind::Concat)
            < xterm_key(cx_id, SlotId::new(1), SlotId::new(10), XTermKind::Concat)
    );
    assert_eq!(
        temporal_xterm_key(cx_id, cx(2)),
        [vec![1; 16], vec![2; 16]].concat()
    );
    assert!(temporal_xterm_key(cx_id, cx(2)) < temporal_xterm_key(cx_id, cx(3)));
    assert!(anchor_key(cx_id, &AnchorKind::TestPass) < anchor_key(cx_id, &AnchorKind::Reward));
    assert!(
        anchor_key(cx_id, &AnchorKind::TestPass)
            < anchor_key(cx_id, &AnchorKind::Label("z".to_string()))
    );
    assert!(
        anchor_key(cx_id, &AnchorKind::Recurrence)
            < anchor_key(cx_id, &AnchorKind::Label(String::new()))
    );
    assert!(
        anchor_key(cx_id, &AnchorKind::Label("a".to_string()))
            < anchor_key(cx_id, &AnchorKind::Label("z".to_string()))
    );
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
fn cx_prefix_range_edges_are_byte_exact() {
    let zero = CxId::from_bytes([0; 16]);
    let range = cx_prefix_range(zero);
    let mut expected_end = vec![0; 16];
    expected_end[15] = 1;

    assert_eq!(base_key(zero), vec![0; 16]);
    assert_eq!(range.start, vec![0; 16]);
    assert_eq!(range.end, Some(expected_end));
    assert!(range.contains(&base_key(zero)));
    assert!(range.contains(&slot_key(zero)));
    assert!(!range.contains(&base_key(CxId::from_bytes({
        let mut bytes = [0; 16];
        bytes[15] = 1;
        bytes
    }))));

    let max = CxId::from_bytes([0xff; 16]);
    let max_range = cx_prefix_range(max);
    assert_eq!(max_range.end, None);
    assert!(max_range.contains(&base_key(max)));

    let cx_a = cx(0x10);
    let cx_b = cx(0x11);
    let recurrence_range = recurrence_prefix_range(cx_a);
    assert!(recurrence_range.contains(&recurrence_key(cx_a, 0)));
    assert!(!recurrence_range.contains(&recurrence_key(cx_b, 0)));

    let temporal_range = temporal_xterm_prefix_range(cx_a);
    assert!(temporal_range.contains(&temporal_xterm_key(cx_a, cx_b)));
    assert!(!temporal_range.contains(&temporal_xterm_key(cx_b, cx_a)));
}

#[test]
fn ledger_range_is_half_open() {
    let range = ledger_range(100, 103);

    assert!(range.contains(&ledger_key(100)));
    assert!(range.contains(&ledger_key(102)));
    assert!(!range.contains(&ledger_key(99)));
    assert!(!range.contains(&ledger_key(103)));
    assert!(!ledger_range(0, 10).contains(&ledger_key(10)));
    assert!(!ledger_range(0, 10).contains(&ledger_key(u64::MAX)));
}

#[test]
fn full_content_hash_golden_and_prefix_checks() {
    let hello = full_content_hash([b"hello".as_slice()]);
    let hello_hex = hex_bytes(&hello);
    println!("FULL_CONTENT_HASH_HELLO {hello_hex}");
    assert_eq!(
        hello_hex,
        "efec82454d7d10340f950b2518aa046b2af41950706e0498252ff3b6e35f1329"
    );

    let panel = 7_u32.to_be_bytes();
    let full = full_content_hash([b"input".as_slice(), panel.as_slice(), b"salt".as_slice()]);
    let full_hex = hex_bytes(&full);
    println!("FULL_CONTENT_HASH_INPUT_PANEL_SALT {full_hex}");
    assert_eq!(
        full_hex,
        "e6b6e4d58f61458e355cac08afc03c22ad0610d71673dc4ce3306bec2d29cfab"
    );

    let cx_id = cx_id_from_full_hash(&full);
    println!("CXID_PREFIX {}", hex_bytes(cx_id.as_bytes()));
    assert_eq!(cx_id.as_bytes().as_slice(), &full[0..16]);

    verify_cx_hash_prefix(cx_id, &full).expect("hash prefix matches");

    let mut altered = full;
    altered[0] ^= 0xff;
    let error = verify_cx_hash_prefix(cx_id, &altered).expect_err("altered hash rejected");

    assert_eq!(error.code, "CALYX_ASTER_CORRUPT_SHARD");
}

#[test]
fn full_content_hash_edges_are_length_delimited() {
    let empty = full_content_hash(std::iter::empty::<&[u8]>());
    println!("FULL_CONTENT_HASH_EMPTY {}", hex_bytes(&empty));
    assert_eq!(
        hex_bytes(&empty),
        "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
    );

    let with_empty = full_content_hash([b"".as_slice()]);
    assert_ne!(empty, with_empty);

    let ab_c = full_content_hash([b"ab".as_slice(), b"c".as_slice()]);
    let a_bc = full_content_hash([b"a".as_slice(), b"bc".as_slice()]);
    println!("FULL_CONTENT_HASH_AB_C {}", hex_bytes(&ab_c));
    println!("FULL_CONTENT_HASH_A_BC {}", hex_bytes(&a_bc));
    assert_eq!(
        hex_bytes(&ab_c),
        "a73c4dc61d8c78f071bdd42fb23a25181931998160dcb7f5353c6b368f05ffa8"
    );
    assert_eq!(
        hex_bytes(&a_bc),
        "978f49b785cd02fa9b61553b99e34cf71c9162fac195b7d59089e2e2a52a460d"
    );
    assert_ne!(ab_c, a_bc);
}

proptest! {
    #[test]
    fn base_and_slot_keys_sort_by_cx_id(a in any::<[u8; 16]>(), b in any::<[u8; 16]>()) {
        let a_id = CxId::from_bytes(a);
        let b_id = CxId::from_bytes(b);
        prop_assert_eq!(base_key(a_id).cmp(&base_key(b_id)), a.cmp(&b));
        prop_assert_eq!(slot_key(a_id).cmp(&slot_key(b_id)), a.cmp(&b));
    }

    #[test]
    fn ledger_keys_sort_by_big_endian_sequence(a in any::<u64>(), b in any::<u64>()) {
        prop_assert_eq!(ledger_key(a).cmp(&ledger_key(b)), a.cmp(&b));
    }

    #[test]
    fn scalar_keys_sort_scalar_first_then_cx(
        s1 in any::<u32>(),
        s2 in any::<u32>(),
        cx1 in any::<[u8; 16]>(),
        cx2 in any::<[u8; 16]>(),
    ) {
        let left = scalar_key(ScalarId::new(s1), CxId::from_bytes(cx1));
        let right = scalar_key(ScalarId::new(s2), CxId::from_bytes(cx2));
        prop_assert_eq!(left.cmp(&right), (s1, cx1).cmp(&(s2, cx2)));
    }

    #[test]
    fn xterm_keys_sort_by_cx_slot_pair_then_kind(
        cx_bytes in any::<[u8; 16]>(),
        a1 in any::<u16>(),
        b1 in any::<u16>(),
        k1 in 0u8..4,
        a2 in any::<u16>(),
        b2 in any::<u16>(),
        k2 in 0u8..4,
    ) {
        let cx_id = CxId::from_bytes(cx_bytes);
        let left = xterm_key(cx_id, SlotId::new(a1), SlotId::new(b1), xterm_kind(k1));
        let right = xterm_key(cx_id, SlotId::new(a2), SlotId::new(b2), xterm_kind(k2));
        prop_assert_eq!(left.cmp(&right), (a1, b1, k1).cmp(&(a2, b2, k2)));
    }

    #[test]
    fn temporal_xterm_keys_sort_by_cx_pair(
        a1 in any::<[u8; 16]>(),
        b1 in any::<[u8; 16]>(),
        a2 in any::<[u8; 16]>(),
        b2 in any::<[u8; 16]>(),
    ) {
        let left = temporal_xterm_key(CxId::from_bytes(a1), CxId::from_bytes(b1));
        let right = temporal_xterm_key(CxId::from_bytes(a2), CxId::from_bytes(b2));
        prop_assert_eq!(left.cmp(&right), (a1, b1).cmp(&(a2, b2)));
    }

    #[test]
    fn prefix_ranges_include_prefix_and_exclude_upper_bound(
        prefix in proptest::collection::vec(any::<u8>(), 1..=16),
        suffix in proptest::collection::vec(any::<u8>(), 0..8),
    ) {
        let range = prefix_range(&prefix);
        let mut matching = prefix.clone();
        matching.extend_from_slice(&suffix);
        prop_assert!(range.contains(&matching));
        if let Some(end) = &range.end {
            prop_assert!(!range.contains(end));
        }
    }

    #[test]
    fn content_hash_and_cxid_are_deterministic(
        input in proptest::collection::vec(any::<u8>(), 0..64),
        panel in any::<u32>(),
        salt in proptest::collection::vec(any::<u8>(), 0..32),
    ) {
        let panel_bytes = panel.to_be_bytes();
        let first = full_content_hash([input.as_slice(), panel_bytes.as_slice(), salt.as_slice()]);
        let second = full_content_hash([input.as_slice(), panel_bytes.as_slice(), salt.as_slice()]);
        prop_assert_eq!(first, second);
        prop_assert_eq!(cx_id_from_full_hash(&first), cx_id_from_full_hash(&second));
    }

    #[test]
    fn mutated_hash_prefix_fails_closed(index in 0usize..16) {
        let full = full_content_hash([b"collision-check".as_slice()]);
        let cx_id = cx_id_from_full_hash(&full);
        let mut mutated = full;
        mutated[index] ^= 0xff;
        let error = verify_cx_hash_prefix(cx_id, &mutated).expect_err("mutated prefix rejected");
        prop_assert_eq!(error.code, "CALYX_ASTER_CORRUPT_SHARD");
    }
}

fn xterm_kind(code: u8) -> XTermKind {
    match code {
        0 => XTermKind::Concat,
        1 => XTermKind::Interaction,
        2 => XTermKind::Agreement,
        3 => XTermKind::Delta,
        _ => unreachable!("proptest constrains xterm kind code"),
    }
}

fn hex_bytes(bytes: &[u8]) -> String {
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
