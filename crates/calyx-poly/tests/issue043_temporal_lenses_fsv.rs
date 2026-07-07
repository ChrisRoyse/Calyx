use std::fs;
use std::path::PathBuf;

use calyx_core::{AbsentReason, SlotId, SlotVector};
use calyx_poly::lenses;
use calyx_poly::{
    Book, CounterpartyVolume, E2_RECENCY_KEY, E3_PERIODIC_KEY, E4_POSITIONAL_KEY,
    ERR_TEMPORAL_INVALID, HolderShare, Level, MakerShare, MakerShareEvidenceSource, MarketSnapshot,
    OnchainFill, OnchainFillSide, OracleRiskEvidence, TemporalLensKind, compute_temporal_vector,
    materialize_poly_v1_panel_registry, read_poly_panel_registry_snapshot,
    write_poly_panel_registry_snapshot,
};
use serde_json::json;

#[test]
fn issue043_temporal_lenses_fsv() {
    let root = fsv_root();
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let root = fs::canonicalize(&root).expect("canonical FSV root");
    let snapshot = known_snapshot();

    let temporal_source = json!({
        "artifact_kind": "poly.issue043.temporal_source",
        "snapshot_ts": snapshot.snapshot_ts,
        "temporal_reference_ts": snapshot.temporal_reference_ts,
        "sequence_position": snapshot.sequence_position,
        "sequence_total": snapshot.sequence_total
    });
    let temporal_source_path = root.join("issue043_temporal_source.json");
    let temporal_source_bytes = serde_json::to_vec_pretty(&temporal_source).unwrap();
    fs::write(&temporal_source_path, &temporal_source_bytes).unwrap();
    let temporal_source_readback = fs::read(&temporal_source_path).unwrap();
    assert_eq!(temporal_source_readback, temporal_source_bytes);

    let e2 = compute_temporal_vector(&snapshot, TemporalLensKind::E2Recency).expect("E2");
    let e3 = compute_temporal_vector(&snapshot, TemporalLensKind::E3Periodic).expect("E3");
    let e4 = compute_temporal_vector(&snapshot, TemporalLensKind::E4Positional).expect("E4");
    assert_close_slice(&e2, &[0.5]);
    assert_close_slice(&e3, &[1.0, 1.0 - 1.0 / 3.5]);
    assert_close_slice(&e4, &[1.0, 0.0, 1.0, 0.0]);

    let panel = lenses::default_panel(43, vec!["global".to_string()]);
    let slots = panel.measure_all(&snapshot);
    assert_eq!(
        slots.get(&SlotId::new(14)),
        Some(&SlotVector::Dense {
            dim: 1,
            data: e2.clone()
        })
    );
    assert_eq!(
        slots.get(&SlotId::new(11)),
        Some(&SlotVector::Dense {
            dim: 2,
            data: e3.clone()
        })
    );
    assert_eq!(
        slots.get(&SlotId::new(15)),
        Some(&SlotVector::Dense {
            dim: 4,
            data: e4.clone()
        })
    );

    let materialized =
        materialize_poly_v1_panel_registry(1, vec!["global".to_string()], 1_785_500_043, &snapshot)
            .expect("materialize temporal registry");
    let registry_path = write_poly_panel_registry_snapshot(&root, &materialized.snapshot)
        .expect("write temporal registry");
    let registry_readback =
        read_poly_panel_registry_snapshot(&registry_path).expect("read temporal registry");
    let temporal_flags = temporal_flag_summary(&registry_readback.panel.slots);
    assert_temporal_flags(&temporal_flags);

    let missing_reference = edge_missing_reference();
    let invalid_reference = edge_invalid_reference();
    let missing_sequence = edge_missing_sequence();
    let invalid_sequence = edge_invalid_sequence();

    let report = json!({
        "issue": 43,
        "proof_claim": "Poly uses Calyx Registry E2/E3/E4 temporal lenses as retrieval-only sidecars, measures known temporal vectors from snapshot source fields, and persists panel slots with retrieval_only and excluded_from_dedup enabled.",
        "minimum_sufficient_corpus": {
            "happy_snapshots": 1,
            "temporal_lenses": 3,
            "edge_snapshots": 4,
            "why_this_is_sufficient": "One known-truth snapshot with event time, reference time, and sequence position exercises E2, E3, and E4 through the real Calyx temporal lenses and through default_panel/registry materialization; four edge snapshots prove missing/invalid reference and missing/invalid sequence fail closed.",
            "why_smaller_is_insufficient": "Without one complete temporal snapshot, one of E2/E3/E4 is unproven; fewer edge snapshots would leave either reference-time or sequence-position failure behavior unverified.",
            "why_larger_is_wasteful": "More temporal rows repeat the same Calyx lens byte-input, vector, panel-slot, and registry-flag paths without adding proof for #43."
        },
        "source_of_truth": "persisted issue043_temporal_source.json and poly_panel_registry_v1.json read back from disk plus direct default_panel temporal slot measurement",
        "temporal_source_path": temporal_source_path.display().to_string(),
        "temporal_source_blake3": blake3_hex(&temporal_source_readback),
        "registry_path": registry_path.display().to_string(),
        "slot_count": registry_readback.slot_count,
        "registered_lens_count": registry_readback.registered_lens_count,
        "temporal_flags": temporal_flags,
        "happy_path": {
            E2_RECENCY_KEY: e2,
            E3_PERIODIC_KEY: e3,
            E4_POSITIONAL_KEY: e4
        },
        "edge_cases": {
            "missing_reference": missing_reference,
            "invalid_reference": invalid_reference,
            "missing_sequence": missing_sequence,
            "invalid_sequence": invalid_sequence
        }
    });
    let report_path = root.join("issue043_temporal_lenses_fsv_report.json");
    let report_bytes = serde_json::to_vec_pretty(&report).unwrap();
    fs::write(&report_path, &report_bytes).unwrap();
    let report_readback = fs::read(&report_path).unwrap();
    assert_eq!(report_readback, report_bytes);
    println!("ISSUE043_TEMPORAL_LENSES_FSV={}", report_path.display());
}

fn edge_missing_reference() -> serde_json::Value {
    let mut snapshot = known_snapshot();
    snapshot.temporal_reference_ts = None;
    let e2 = compute_temporal_vector(&snapshot, TemporalLensKind::E2Recency)
        .expect_err("missing E2 reference absent");
    let e3 = compute_temporal_vector(&snapshot, TemporalLensKind::E3Periodic)
        .expect_err("missing E3 reference absent");
    assert_eq!(e2, AbsentReason::LensUnavailable);
    assert_eq!(e3, AbsentReason::LensUnavailable);
    json!({ "e2": format!("{e2:?}"), "e3": format!("{e3:?}") })
}

fn edge_invalid_reference() -> serde_json::Value {
    let mut snapshot = known_snapshot();
    snapshot.snapshot_ts = 100;
    snapshot.temporal_reference_ts = Some(99);
    let err = compute_temporal_vector(&snapshot, TemporalLensKind::E2Recency)
        .expect_err("reference before event must fail closed");
    let reason = format!("{err:?}");
    assert!(reason.contains(ERR_TEMPORAL_INVALID));
    json!({ "reason": reason })
}

fn edge_missing_sequence() -> serde_json::Value {
    let mut snapshot = known_snapshot();
    snapshot.sequence_position = None;
    let err = compute_temporal_vector(&snapshot, TemporalLensKind::E4Positional)
        .expect_err("missing sequence absent");
    assert_eq!(err, AbsentReason::LensUnavailable);
    json!({ "reason": format!("{err:?}") })
}

fn edge_invalid_sequence() -> serde_json::Value {
    let mut snapshot = known_snapshot();
    snapshot.sequence_position = Some(11);
    snapshot.sequence_total = Some(10);
    let err = compute_temporal_vector(&snapshot, TemporalLensKind::E4Positional)
        .expect_err("position beyond total must fail closed");
    let reason = format!("{err:?}");
    assert!(reason.contains(ERR_TEMPORAL_INVALID));
    json!({ "reason": reason })
}

fn known_snapshot() -> MarketSnapshot {
    MarketSnapshot {
        token_id: "token-issue043".to_string(),
        condition_id: "condition-issue043".to_string(),
        outcome_index: 0,
        slug: "issue043-temporal-known-truth".to_string(),
        question: Some("Issue 043 temporal known truth?".to_string()),
        event_id: Some("event-issue043".to_string()),
        category: Some("crypto".to_string()),
        region: Some("global".to_string()),
        tags: vec!["temporal".to_string()],
        resolution_source: Some("uma".to_string()),
        neg_risk: false,
        snapshot_ts: 0,
        price: Some(0.62),
        mid: Some(0.62),
        best_bid: Some(0.61),
        best_ask: Some(0.63),
        spread: Some(0.02),
        tick_size: Some(0.01),
        volume_24h: Some(120.0),
        liquidity: Some(10_000.0),
        one_hour_change: Some(0.01),
        one_day_change: Some(-0.02),
        ofi: Some(0.2),
        yes_no_residual: Some(0.0),
        secs_to_resolution: Some(86_400.0),
        holders: vec![HolderShare {
            wallet: "0xholder-temporal".to_string(),
            amount: 100.0,
            outcome_index: 0,
        }],
        makers: vec![MakerShare {
            maker: "0xmaker-temporal".to_string(),
            size: 100.0,
            evidence_source: MakerShareEvidenceSource::RestingClobOrderBook,
        }],
        counterparty_volumes: vec![CounterpartyVolume {
            counterparty: "0xcp-temporal".to_string(),
            volume: 120.0,
        }],
        onchain_fills: known_fills(),
        temporal_reference_ts: Some(86_400),
        sequence_position: Some(5),
        sequence_total: Some(10),
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
                size: 100.0,
            }],
        },
    }
}

fn known_fills() -> Vec<OnchainFill> {
    vec![
        fill("0xissue043-a", 0, 0, OnchainFillSide::Buy, 25.0),
        fill("0xissue043-b", 1, 0, OnchainFillSide::Sell, 15.0),
        fill("0xissue043-c", 2, 0, OnchainFillSide::Buy, 40.0),
        fill("0xissue043-d", 3, 0, OnchainFillSide::Sell, 40.0),
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

fn temporal_flag_summary(slots: &[calyx_core::Slot]) -> serde_json::Value {
    json!(
        slots
            .iter()
            .filter(|slot| {
                matches!(
                    slot.slot_key.key(),
                    E2_RECENCY_KEY | E3_PERIODIC_KEY | E4_POSITIONAL_KEY
                )
            })
            .map(|slot| {
                json!({
                    "slot": slot.slot_id.get(),
                    "key": slot.slot_key.key(),
                    "retrieval_only": slot.retrieval_only,
                    "excluded_from_dedup": slot.excluded_from_dedup
                })
            })
            .collect::<Vec<_>>()
    )
}

fn assert_temporal_flags(flags: &serde_json::Value) {
    let rows = flags.as_array().expect("temporal flag rows");
    assert_eq!(rows.len(), 3);
    for row in rows {
        assert_eq!(row["retrieval_only"], json!(true));
        assert_eq!(row["excluded_from_dedup"], json!(true));
    }
}

fn blake3_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn assert_close_slice(actual: &[f32], expected: &[f32]) {
    assert_eq!(actual.len(), expected.len());
    for (actual, expected) in actual.iter().zip(expected) {
        assert!(
            (*actual - *expected).abs() <= 1.0e-6,
            "actual={actual} expected={expected}"
        );
    }
}

fn fsv_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../target/fsv/issue043_temporal_lenses_20260705_001")
}
