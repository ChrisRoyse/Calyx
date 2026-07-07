use std::fs;

use super::*;
use crate::constellation::build_constellation;
use crate::lenses::default_panel;
use crate::model::MarketSnapshot;
use calyx_aster::cf::{ColumnFamily, anchor_key, ledger_key};
use calyx_aster::vault::{VaultOptions, encode};
use calyx_core::{AnchorKind, Modality, VaultId};
use calyx_ledger::{SubjectId, decode as decode_ledger};

fn temp_vault_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("calyx-poly-{name}-{}", std::process::id()));
    if dir.exists() {
        fs::remove_dir_all(&dir).expect("remove previous test vault");
    }
    fs::create_dir_all(&dir).expect("create test vault dir");
    dir
}

fn open_test_vault(dir: &std::path::Path) -> AsterVault {
    AsterVault::new_durable(
        dir,
        "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap(),
        b"pipeline-test-salt".to_vec(),
        VaultOptions::default(),
    )
    .expect("open durable test vault")
}

fn sample() -> MarketSnapshot {
    MarketSnapshot {
        token_id: "tok".into(),
        condition_id: "0xcond".into(),
        outcome_index: 0,
        slug: "will-btc-100k".into(),
        question: Some("Will BTC reach 100k?".into()),
        event_id: None,
        category: Some("crypto".into()),
        region: Some("global".into()),
        tags: vec![],
        resolution_source: Some("uma".into()),
        neg_risk: false,
        snapshot_ts: 1_785_500_000,
        price: Some(0.62),
        mid: Some(0.62),
        best_bid: Some(0.61),
        best_ask: Some(0.63),
        spread: Some(0.02),
        tick_size: Some(0.01),
        volume_24h: Some(125_000.0),
        liquidity: Some(40_000.0),
        one_hour_change: Some(0.01),
        one_day_change: Some(-0.03),
        ofi: Some(0.2),
        yes_no_residual: Some(0.0),
        secs_to_resolution: Some(86_400.0),
        holders: vec![],
        makers: vec![],
        counterparty_volumes: vec![],
        onchain_fills: vec![],
        temporal_reference_ts: None,
        sequence_position: None,
        sequence_total: None,
        oracle_risk: Default::default(),
        book: Default::default(),
    }
}

#[test]
fn ingest_persists_and_readback_matches_source_of_truth() {
    // FSV: ingest through the REAL durable AsterVault, then read the source of truth back.
    // (Previously used an in-memory HashMap that could never exhibit the collision-drop bug.)
    let dir = temp_vault_dir("ingest-source-of-truth");
    let store = open_test_vault(&dir);
    let panel = default_panel(1, vec!["global".into()]);
    let vid: VaultId = "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap();

    // Independently derive the expected content-addressed id.
    let expected = build_constellation(&sample(), &panel, vid, b"salt").unwrap();
    let id = ingest_snapshot(&store, &panel, &sample(), vid, b"salt").unwrap();
    assert_eq!(id, expected.cx_id, "ingest returns the content-addressed id");
    store.flush().expect("flush ingest source of truth");

    // Read the durable SOURCE OF TRUTH back and inspect it — never the return value alone.
    let stored = store.get(id, store.snapshot()).unwrap();
    assert_eq!(stored.cx_id, expected.cx_id);
    assert_eq!(stored.modality, Modality::Structured);
    assert_eq!(stored.scalars.get("price"), Some(&0.62));
    assert_eq!(stored.scalars.get("liquidity"), Some(&40_000.0));
    assert_eq!(
        stored.metadata.get("category").map(String::as_str),
        Some("crypto")
    );
    assert!(stored.flags.ungrounded);
    assert!(!stored.slots.is_empty());

    drop(store);
    fs::remove_dir_all(dir).expect("remove test vault");
}

#[test]
fn distinct_microstructure_persists_both_records_not_silently_dropped() {
    // Regression + FSV for #181: two observations that agree on the old 6-field identity subset
    // (token_id, ts, price, mid, spread, volume_24h) but differ in microstructure (a whale posts
    // deep liquidity within the same second) must BOTH persist as distinct records — the second
    // must never be silently dropped while `put` returns Ok.
    let dir = temp_vault_dir("distinct-microstructure");
    let store = open_test_vault(&dir);
    let panel = default_panel(1, vec!["global".into()]);
    let vid: VaultId = "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap();

    let a = sample(); // liquidity 40_000, best_bid 0.61, ofi 0.2
    let mut b = sample();
    b.best_bid = Some(0.615);
    b.liquidity = Some(250_000.0);
    b.ofi = Some(0.4);
    // The old identity subset is byte-identical between A and B.
    assert_eq!(a.token_id, b.token_id);
    assert_eq!(a.snapshot_ts, b.snapshot_ts);
    assert_eq!(a.price, b.price);
    assert_eq!(a.spread, b.spread);
    assert_eq!(a.volume_24h, b.volume_24h);

    let id_a = ingest_snapshot(&store, &panel, &a, vid, b"salt").unwrap();
    let id_b = ingest_snapshot(&store, &panel, &b, vid, b"salt").unwrap();
    store.flush().expect("flush distinct microstructure");
    assert_ne!(
        id_a, id_b,
        "distinct microstructure must yield distinct CxIds"
    );

    // Both records recoverable from the durable SOURCE OF TRUTH with their own values.
    let stored_a = store.get(id_a, store.snapshot()).unwrap();
    let stored_b = store.get(id_b, store.snapshot()).unwrap();
    assert_eq!(stored_a.scalars.get("liquidity"), Some(&40_000.0));
    assert_eq!(
        stored_b.scalars.get("liquidity"),
        Some(&250_000.0),
        "the whale snapshot's liquidity must not be lost (the #181 bug)"
    );

    drop(store);
    fs::remove_dir_all(dir).expect("remove test vault");
}

#[test]
fn byte_identical_reingest_is_idempotent_source_of_truth() {
    // Edge: ingesting the byte-identical snapshot twice dedups to exactly one durable record.
    let dir = temp_vault_dir("idempotent-reingest");
    let store = open_test_vault(&dir);
    let panel = default_panel(1, vec!["global".into()]);
    let vid: VaultId = "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap();

    let id1 = ingest_snapshot(&store, &panel, &sample(), vid, b"salt").unwrap();
    let id2 = ingest_snapshot(&store, &panel, &sample(), vid, b"salt").unwrap();
    store.flush().expect("flush idempotent reingest");
    assert_eq!(id1, id2, "identical input dedups to one CxId");
    let stored = store.get(id1, store.snapshot()).unwrap();
    assert_eq!(stored.scalars.get("liquidity"), Some(&40_000.0));

    drop(store);
    fs::remove_dir_all(dir).expect("remove test vault");
}

#[test]
fn grounding_writes_outcome_anchor_to_source_of_truth() {
    let dir = temp_vault_dir("grounding-source-of-truth");
    let store = open_test_vault(&dir);
    let panel = default_panel(1, vec!["global".into()]);
    let vid: VaultId = "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap();
    let id = ingest_snapshot(&store, &panel, &sample(), vid, b"salt").unwrap();

    let res = Resolution {
        condition_id: "0xcond".into(),
        winning_outcome_index: 0,
        winning_label: "YES".into(),
        resolved_ts: 1_785_600_000,
        source: "uma".into(),
        disputed: false,
    };
    let refs = ground_market(&store, &[id], &res, 0).unwrap();
    store.flush().expect("flush grounding source of truth");
    let snapshot = store.snapshot();

    let anchor_row = store
        .read_cf_at(
            snapshot,
            ColumnFamily::Anchors,
            &anchor_key(id, &AnchorKind::TestPass),
        )
        .expect("read anchor row")
        .expect("anchor row must exist");
    let anchor = encode::decode_anchor(&anchor_row).expect("decode anchor row");
    assert_eq!(anchor.value, calyx_core::AnchorValue::Bool(true));
    assert_eq!(anchor.confidence, 1.0);
    let label_row = store
        .read_cf_at(
            snapshot,
            ColumnFamily::Anchors,
            &anchor_key(id, &AnchorKind::Label("outcome".to_string())),
        )
        .expect("read label anchor row")
        .expect("label anchor row must exist");
    let label_anchor = encode::decode_anchor(&label_row).expect("decode label anchor row");
    assert_eq!(
        label_anchor.value,
        calyx_core::AnchorValue::Enum("YES".to_string())
    );
    assert_eq!(label_anchor.confidence, 1.0);

    let ledger_row = store
        .read_cf_at(snapshot, ColumnFamily::Ledger, &ledger_key(refs[0].seq))
        .expect("read ledger row")
        .expect("ledger row must exist");
    let ledger = decode_ledger(&ledger_row).expect("decode ledger row");
    let payload: serde_json::Value =
        serde_json::from_slice(&ledger.payload).expect("decode grounding payload");
    assert_eq!(ledger.kind, EntryKind::Grounding);
    assert!(matches!(ledger.subject, SubjectId::Cx(cx) if cx == id));
    assert_eq!(payload["anchors"].as_array().unwrap().len(), 2);
    assert_eq!(refs[0].seq, 1);

    drop(store);
    fs::remove_dir_all(dir).expect("remove test vault");
}
