use std::fs;
use std::path::PathBuf;

use calyx_core::{Input, Modality, SlotId, SlotVector};
use calyx_poly::lenses::default_panel;
use calyx_poly::{
    Book, CounterpartyVolume, ERR_PANEL_REGISTRY_INVALID, HolderShare, Level, MakerShare,
    MakerShareEvidenceSource, MarketSnapshot, OnchainFill, OnchainFillSide, OracleRiskEvidence,
    materialize_poly_v1_panel_registry, measure_registered_poly_panel,
    read_poly_panel_registry_snapshot, validate_poly_panel_registry_snapshot,
    write_poly_panel_registry_snapshot,
};
use serde_json::json;

#[test]
fn issue039_poly_panel_registry_fsv() {
    let root = fsv_root();
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let root = fs::canonicalize(&root).expect("canonical FSV root");
    let region_vocab = vec!["global".to_string(), "us".to_string()];
    let snapshot = full_snapshot();

    let materialized =
        materialize_poly_v1_panel_registry(1, region_vocab.clone(), 1_785_500_039, &snapshot)
            .expect("materialize registry panel");
    assert_eq!(materialized.panel.slots.len(), 17);
    assert_eq!(materialized.registry.lens_snapshots().len(), 17);

    let registry_slots =
        measure_registered_poly_panel(&materialized.registry, &materialized.panel, &snapshot)
            .expect("registered panel measures full snapshot");
    let direct_slots = default_panel(1, region_vocab.clone()).measure_all(&snapshot);
    assert_eq!(
        registry_slots, direct_slots,
        "registry-backed lenses must emit the same vectors as the Poly SignalLens panel"
    );

    let artifact_path = write_poly_panel_registry_snapshot(&root, &materialized.snapshot)
        .expect("write registry snapshot");
    let readback = read_poly_panel_registry_snapshot(&artifact_path).expect("readback snapshot");
    assert_eq!(readback, materialized.snapshot);

    let missing = edge_missing_field_fails_closed(&materialized, &snapshot);
    let wrong_modality = edge_wrong_modality_fails_closed(&materialized, &snapshot);
    let malformed = edge_malformed_input_fails_closed(&materialized);
    let tampered = edge_tampered_duplicate_slot_fails_closed(&readback);
    let no_region = edge_missing_region_vocab_fails_closed(&snapshot);

    let report = json!({
        "issue": 39,
        "proof_claim": "Poly's v1 embedder-free SignalLens panel is registered into the real Calyx Registry with frozen contracts, deterministic probe proof, persisted panel state, and registry measurement parity with direct SignalLens output.",
        "minimum_sufficient_corpus": {
            "complete_known_truth_snapshots": 1,
            "edge_probes": 5,
            "why_this_is_sufficient": "one complete realistic snapshot exercises every v1 slot once through both direct and registry-backed paths; the five edge probes cover missing source fields, wrong modality, malformed bytes, tampered persisted state, and an unmeasurable region vocabulary.",
            "why_smaller_is_insufficient": "without the complete snapshot there is no all-slot deterministic probe; fewer edge probes would leave either runtime input validation, missing-field fail-closed behavior, or persisted artifact validation unproven.",
            "why_larger_is_wasteful": "market-scale data would repeat the same register->probe->measure->persist->readback path and would not add proof for the registry/frozen-contract wiring claim."
        },
        "source_of_truth": "persisted poly_panel_registry_v1.json read back from disk plus live Calyx Registry measurement",
        "artifact_path": artifact_path.display().to_string(),
        "slot_count": readback.slot_count,
        "registered_lens_count": readback.registered_lens_count,
        "determinism_probe_count": readback.determinism_probe_count,
        "registry_parity": slot_summary(&registry_slots),
        "edges": {
            "missing_field": missing,
            "wrong_modality": wrong_modality,
            "malformed_input": malformed,
            "tampered_duplicate_slot": tampered,
            "missing_region_vocab": no_region
        }
    });
    let report_path = root.join("issue039_poly_panel_registry_fsv_report.json");
    fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    println!("ISSUE039_POLY_PANEL_REGISTRY_FSV={}", report_path.display());
}

fn edge_missing_field_fails_closed(
    materialized: &calyx_poly::PolyPanelRegistryMaterialization,
    snapshot: &MarketSnapshot,
) -> serde_json::Value {
    let mut missing = snapshot.clone();
    missing.ofi = None;
    let err = measure_registered_poly_panel(&materialized.registry, &materialized.panel, &missing)
        .expect_err("missing required OFI field must fail through registry shape validation");
    assert_eq!(err.code(), "CALYX_LENS_DIM_MISMATCH");
    json!({ "code": err.code(), "message": err.message() })
}

fn edge_wrong_modality_fails_closed(
    materialized: &calyx_poly::PolyPanelRegistryMaterialization,
    snapshot: &MarketSnapshot,
) -> serde_json::Value {
    let first = materialized.panel.slots[0].lens_id;
    let input = Input::new(Modality::Text, snapshot.canonical_input_bytes().unwrap());
    let err = materialized
        .registry
        .measure(first, &input)
        .expect_err("wrong modality must fail closed");
    assert_eq!(err.code, "CALYX_LENS_DIM_MISMATCH");
    json!({ "code": err.code, "message": err.message })
}

fn edge_malformed_input_fails_closed(
    materialized: &calyx_poly::PolyPanelRegistryMaterialization,
) -> serde_json::Value {
    let first = materialized.panel.slots[0].lens_id;
    let input = Input::new(Modality::Structured, b"{not-json".to_vec());
    let err = materialized
        .registry
        .measure(first, &input)
        .expect_err("malformed structured bytes must fail closed");
    assert_eq!(err.code, "CALYX_LENS_UNREACHABLE");
    json!({ "code": err.code, "message": err.message })
}

fn edge_tampered_duplicate_slot_fails_closed(
    readback: &calyx_poly::PolyPanelRegistrySnapshot,
) -> serde_json::Value {
    let mut tampered = readback.clone();
    tampered.panel.slots[1].slot_id = tampered.panel.slots[0].slot_id;
    tampered.slots[1].slot_id = tampered.slots[0].slot_id;
    let err = validate_poly_panel_registry_snapshot(&tampered)
        .expect_err("duplicate persisted slot must fail closed");
    assert_eq!(err.code(), ERR_PANEL_REGISTRY_INVALID);
    json!({ "code": err.code(), "message": err.message() })
}

fn edge_missing_region_vocab_fails_closed(snapshot: &MarketSnapshot) -> serde_json::Value {
    let err = match materialize_poly_v1_panel_registry(1, Vec::new(), 1_785_500_039, snapshot) {
        Ok(_) => panic!("region_oh probe cannot be absent during frozen registration"),
        Err(err) => err,
    };
    assert_eq!(err.code(), "CALYX_LENS_DIM_MISMATCH");
    json!({ "code": err.code(), "message": err.message() })
}

fn full_snapshot() -> MarketSnapshot {
    MarketSnapshot {
        token_id: "token-issue039".to_string(),
        condition_id: "condition-issue039".to_string(),
        outcome_index: 0,
        slug: "issue039-btc-above-known-threshold".to_string(),
        question: Some("Will BTC finish above the known threshold?".to_string()),
        event_id: Some("event-issue039".to_string()),
        category: Some("crypto".to_string()),
        region: Some("global".to_string()),
        tags: vec!["btc".to_string(), "crypto".to_string()],
        resolution_source: Some("uma".to_string()),
        neg_risk: false,
        snapshot_ts: 1_785_500_039,
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
        yes_no_residual: Some(0.03),
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
        makers: vec![MakerShare {
            maker: "0xmaker-a".to_string(),
            size: 250.0,
            evidence_source: MakerShareEvidenceSource::RestingClobOrderBook,
        }],
        counterparty_volumes: vec![CounterpartyVolume {
            counterparty: "0xcp-a".to_string(),
            volume: 25_000.0,
        }],
        onchain_fills: known_onchain_fills(),
        temporal_reference_ts: Some(1_785_586_439),
        sequence_position: Some(5),
        sequence_total: Some(10),
        oracle_risk: OracleRiskEvidence {
            oracle: "uma".to_string(),
            dispute_risk: 0.02,
            active_dispute: false,
            liveness_seconds_remaining: 3_600.0,
        },
        book: Book {
            bids: vec![
                Level {
                    price: 0.61,
                    size: 100.0,
                },
                Level {
                    price: 0.60,
                    size: 50.0,
                },
                Level {
                    price: 0.59,
                    size: 25.0,
                },
            ],
            asks: vec![
                Level {
                    price: 0.63,
                    size: 90.0,
                },
                Level {
                    price: 0.64,
                    size: 40.0,
                },
                Level {
                    price: 0.65,
                    size: 10.0,
                },
            ],
        },
    }
}

fn known_onchain_fills() -> Vec<OnchainFill> {
    vec![
        fill("0xreg-fill-a", 0, 1_785_500_000, OnchainFillSide::Buy, 25.0),
        fill(
            "0xreg-fill-b",
            1,
            1_785_500_001,
            OnchainFillSide::Sell,
            15.0,
        ),
        fill("0xreg-fill-c", 2, 1_785_500_002, OnchainFillSide::Buy, 40.0),
        fill(
            "0xreg-fill-d",
            3,
            1_785_500_003,
            OnchainFillSide::Sell,
            40.0,
        ),
    ]
}

fn fill(
    tx_hash: &str,
    log_index: u32,
    timestamp: u64,
    side: OnchainFillSide,
    size: f64,
) -> OnchainFill {
    OnchainFill {
        tx_hash: tx_hash.to_string(),
        log_index,
        timestamp,
        maker: format!("0xmaker-{log_index}"),
        taker: format!("0xtaker-{log_index}"),
        side,
        price: 0.62,
        size,
    }
}

fn slot_summary(slots: &std::collections::BTreeMap<SlotId, SlotVector>) -> serde_json::Value {
    json!(
        slots
            .iter()
            .map(|(slot, vector)| (slot.get(), slot_kind(vector)))
            .collect::<Vec<_>>()
    )
}

fn slot_kind(slot: &SlotVector) -> &'static str {
    match slot {
        SlotVector::Dense { .. } => "dense",
        SlotVector::Sparse { .. } => "sparse",
        SlotVector::Multi { .. } => "multi",
        SlotVector::Absent { .. } => "absent",
    }
}

fn fsv_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../target/fsv/issue039_poly_panel_registry_20260705_001")
}
