use std::fs;
use std::path::PathBuf;

use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{SlotId, SlotVector, VaultId, VaultStore};
use calyx_poly::constellation::build_constellation;
use calyx_poly::features;
use calyx_poly::lenses::default_panel;
use calyx_poly::{
    Book, CounterpartyVolume, HolderShare, Level, MakerShare, MakerShareEvidenceSource,
    MarketSnapshot, OracleRiskEvidence,
};
use serde_json::json;

#[test]
fn issue031_constellation_vault_put_roundtrip_fsv() {
    let root = fsv_root();
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let vault_dir = root.join("domain-vault-source-of-truth");
    let vault_id: VaultId = "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap();
    let panel = default_panel(31, vec!["global".to_string()]);

    let full_snapshot = full_snapshot();
    let mut missing_feed_snapshot = full_snapshot.clone();
    missing_feed_snapshot.slug = "issue031-missing-feed".to_string();
    missing_feed_snapshot.ofi = features::order_flow_imbalance(0.0, 0.0);
    missing_feed_snapshot.region = None;
    missing_feed_snapshot.holders.clear();

    let full_cx =
        build_constellation(&full_snapshot, &panel, vault_id, b"issue031-panel-salt").unwrap();
    let missing_cx = build_constellation(
        &missing_feed_snapshot,
        &panel,
        vault_id,
        b"issue031-panel-salt",
    )
    .unwrap();
    assert_ne!(
        full_cx.cx_id, missing_cx.cx_id,
        "changed observed content must produce a distinct CxId"
    );
    assert_scalar_close(&full_cx, "ofi", 0.5);
    assert_scalar_close(&full_cx, "distance_from_50", 0.12);
    assert_scalar_close(&full_cx, "holder_herfindahl", 0.52);
    assert_scalar_close(&full_cx, "maker_herfindahl", 0.5);
    assert_scalar_close(&full_cx, "yes_no_residual", 0.03);
    assert!(!missing_cx.scalars.contains_key("ofi"));
    assert!(!missing_cx.scalars.contains_key("holder_herfindahl"));

    let mut bad_snapshot = full_snapshot.clone();
    bad_snapshot.slug = "issue031-nonfinite".to_string();
    bad_snapshot.liquidity = Some(f64::INFINITY);
    let bad_error = build_constellation(&bad_snapshot, &panel, vault_id, b"issue031-panel-salt")
        .expect_err("non-finite snapshot must fail closed before vault put");
    assert_eq!(bad_error.code(), "CALYX_POLY_SNAPSHOT_IDENTITY_NON_FINITE");

    let full_id = full_cx.cx_id;
    let missing_id = missing_cx.cx_id;
    let seq_after_first;
    let seq_after_duplicate;
    let seq_after_second;
    {
        let vault = AsterVault::new_durable(
            &vault_dir,
            vault_id,
            b"issue031-vault-salt".to_vec(),
            VaultOptions::default(),
        )
        .unwrap();

        vault.put(full_cx.clone()).unwrap();
        seq_after_first = vault.snapshot();
        vault.put(full_cx).unwrap();
        seq_after_duplicate = vault.snapshot();
        assert_eq!(
            seq_after_duplicate, seq_after_first,
            "byte-identical duplicate re-ingest must be an idempotent no-op"
        );

        vault.put(missing_cx).unwrap();
        seq_after_second = vault.snapshot();
        assert!(seq_after_second > seq_after_duplicate);
        vault.flush().unwrap();
    }

    let reopened = AsterVault::new_durable(
        &vault_dir,
        vault_id,
        b"issue031-vault-salt".to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let read_seq = reopened.snapshot();
    let stored_full = reopened.get(full_id, read_seq).unwrap();
    let stored_missing = reopened.get(missing_id, read_seq).unwrap();

    assert_eq!(stored_full.metadata.get("slug"), Some(&full_snapshot.slug));
    assert_eq!(
        stored_full.metadata.get("category").map(String::as_str),
        Some("crypto")
    );
    assert_scalar_close(&stored_full, "ofi", 0.5);
    assert_scalar_close(&stored_full, "holder_herfindahl", 0.52);
    assert_scalar_close(&stored_full, "maker_herfindahl", 0.5);
    assert!(matches!(
        stored_full.slots.get(&SlotId::new(5)),
        Some(SlotVector::Dense { .. })
    ));
    assert!(matches!(
        stored_missing.slots.get(&SlotId::new(5)),
        Some(SlotVector::Absent { .. })
    ));
    assert!(!stored_missing.scalars.contains_key("ofi"));
    assert!(!stored_missing.scalars.contains_key("holder_herfindahl"));

    let readback = json!({
        "issue": 31,
        "proof_claim": "calyx-poly builds real Constellations from known-truth MarketSnapshot records, stores them in a durable AsterVault, and can read the persisted state back after reopening the vault",
        "minimum_sufficient_corpus": {
            "valid_snapshots": 2,
            "duplicate_reingest": 1,
            "invalid_nonfinite_snapshot": 1,
            "why_this_is_sufficient": "the two valid snapshots exercise populated scalar/vector paths plus absent feed/holder paths; the duplicate exercises idempotent re-ingest; the invalid snapshot proves fail-closed identity before storage",
            "why_smaller_is_insufficient": "one valid snapshot would not prove absent-slot handling or distinct observed content; omitting the duplicate would not prove idempotence; omitting the invalid row would not prove fail-closed input rejection",
            "why_larger_is_wasteful": "larger corpora would repeat the same build->put->flush->reopen->get source-of-truth path without proving a new invariant for this issue"
        },
        "source_of_truth": "durable AsterVault reopened from disk, then read through VaultStore::get",
        "vault_dir": vault_dir.display().to_string(),
        "read_seq": read_seq,
        "seq_after_first": seq_after_first,
        "seq_after_duplicate": seq_after_duplicate,
        "seq_after_second": seq_after_second,
        "full": {
            "cx_id": full_id.to_string(),
            "ofi": stored_full.scalars.get("ofi"),
            "distance_from_50": stored_full.scalars.get("distance_from_50"),
            "holder_herfindahl": stored_full.scalars.get("holder_herfindahl"),
            "maker_herfindahl": stored_full.scalars.get("maker_herfindahl"),
            "yes_no_residual": stored_full.scalars.get("yes_no_residual"),
            "ofi_slot": slot_kind(stored_full.slots.get(&SlotId::new(5)).unwrap()),
            "metadata_slug": stored_full.metadata.get("slug")
        },
        "missing_feed": {
            "cx_id": missing_id.to_string(),
            "ofi_scalar_present": stored_missing.scalars.contains_key("ofi"),
            "holder_herfindahl_present": stored_missing.scalars.contains_key("holder_herfindahl"),
            "ofi_slot": slot_kind(stored_missing.slots.get(&SlotId::new(5)).unwrap())
        },
        "edges": {
            "duplicate_reingest_noop": seq_after_duplicate == seq_after_first,
            "distinct_observed_content_distinct_cxid": full_id != missing_id,
            "missing_feed_slot_absent": matches!(stored_missing.slots.get(&SlotId::new(5)), Some(SlotVector::Absent { .. })),
            "nonfinite_error_code": bad_error.code()
        }
    });
    let out = root.join("readback.json");
    fs::write(&out, serde_json::to_vec_pretty(&readback).unwrap()).unwrap();
    println!(
        "ISSUE031_CONSTELLATION_VAULT_PUT_READBACK={}",
        out.display()
    );
}

fn full_snapshot() -> MarketSnapshot {
    MarketSnapshot {
        token_id: "token-issue031".to_string(),
        condition_id: "condition-issue031".to_string(),
        outcome_index: 0,
        slug: "issue031-full".to_string(),
        question: Some("Issue 031 full constellation market?".to_string()),
        event_id: Some("event-issue031".to_string()),
        category: Some("crypto".to_string()),
        region: Some("global".to_string()),
        tags: vec!["fsv".to_string(), "calyx".to_string()],
        resolution_source: Some("uma".to_string()),
        neg_risk: false,
        snapshot_ts: 1_785_500_031,
        price: Some(0.62),
        mid: Some(0.62),
        best_bid: Some(0.61),
        best_ask: Some(0.63),
        spread: features::spread(0.61, 0.63),
        tick_size: Some(0.01),
        volume_24h: Some(125_000.0),
        liquidity: Some(40_000.0),
        one_hour_change: Some(0.01),
        one_day_change: Some(-0.03),
        ofi: features::order_flow_imbalance(75.0, 25.0),
        yes_no_residual: Some(features::yes_no_residual(0.62, 0.41)),
        secs_to_resolution: Some(86_400.0),
        holders: vec![
            HolderShare {
                wallet: "0xholder-a".to_string(),
                amount: 60.0,
                outcome_index: 0,
            },
            HolderShare {
                wallet: "0xholder-b".to_string(),
                amount: 40.0,
                outcome_index: 0,
            },
        ],
        makers: vec![
            MakerShare {
                maker: "0xmaker-a".to_string(),
                size: 100.0,
                evidence_source: MakerShareEvidenceSource::RestingClobOrderBook,
            },
            MakerShare {
                maker: "0xmaker-b".to_string(),
                size: 100.0,
                evidence_source: MakerShareEvidenceSource::RestingClobOrderBook,
            },
        ],
        counterparty_volumes: vec![CounterpartyVolume {
            counterparty: "0xcp-a".to_string(),
            volume: 25_000.0,
        }],
        onchain_fills: vec![],
        temporal_reference_ts: None,
        sequence_position: None,
        sequence_total: None,
        oracle_risk: OracleRiskEvidence {
            oracle: "uma".to_string(),
            dispute_risk: 0.02,
            active_dispute: false,
            liveness_seconds_remaining: 3_600.0,
        },
        book: Book {
            bids: vec![Level {
                price: 0.61,
                size: 100.0,
            }],
            asks: vec![Level {
                price: 0.63,
                size: 90.0,
            }],
        },
    }
}

fn slot_kind(slot: &SlotVector) -> &'static str {
    match slot {
        SlotVector::Dense { .. } => "dense",
        SlotVector::Sparse { .. } => "sparse",
        SlotVector::Multi { .. } => "multi",
        SlotVector::Absent { .. } => "absent",
    }
}

fn assert_scalar_close(cx: &calyx_core::Constellation, key: &str, expected: f64) {
    let actual = cx
        .scalars
        .get(key)
        .unwrap_or_else(|| panic!("missing scalar {key}"));
    assert!(
        (actual - expected).abs() < 1.0e-9,
        "scalar {key} expected {expected}, got {actual}"
    );
}

fn fsv_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/fsv/issue031_constellation_vault_put")
}
