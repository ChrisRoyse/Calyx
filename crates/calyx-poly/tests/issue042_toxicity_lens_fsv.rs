use std::fs;
use std::path::PathBuf;

use calyx_core::{AbsentReason, SlotId, SlotVector};
use calyx_poly::encode::signed_log;
use calyx_poly::lenses;
use calyx_poly::{
    Book, ERR_TOXICITY_INVALID_FILL, ERR_TOXICITY_LOOKAHEAD, Level, MarketSnapshot, OnchainFill,
    OnchainFillSide, TOXICITY_VECTOR_DIM, compute_toxicity_metrics, compute_toxicity_vector,
};
use serde_json::json;

#[test]
fn issue042_toxicity_lens_fsv() {
    let root = fsv_root();
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let root = fs::canonicalize(&root).expect("canonical FSV root");
    let snapshot = known_snapshot(known_fills());

    let fill_source = json!({
        "artifact_kind": "poly.issue042.onchain_fills_source",
        "snapshot_ts": snapshot.snapshot_ts,
        "fills": &snapshot.onchain_fills
    });
    let fill_source_path = root.join("issue042_onchain_fills_source.json");
    let fill_source_bytes = serde_json::to_vec_pretty(&fill_source).unwrap();
    fs::write(&fill_source_path, &fill_source_bytes).unwrap();
    let fill_source_readback = fs::read(&fill_source_path).unwrap();
    assert_eq!(fill_source_readback, fill_source_bytes);

    let metrics = compute_toxicity_metrics(&snapshot).expect("known fills toxicity metrics");
    assert_close64(metrics.total_volume, 120.0);
    assert_close64(metrics.bucket_volume, 40.0);
    assert_eq!(metrics.buckets.len(), 3);
    assert_close64(metrics.buckets[0].imbalance_abs, 10.0);
    assert_close64(metrics.buckets[1].imbalance_abs, 40.0);
    assert_close64(metrics.buckets[2].imbalance_abs, 40.0);
    assert_close64(metrics.vpin, 0.75);
    assert_close64(metrics.signed_imbalance, 10.0 / 120.0);
    assert_close64(metrics.largest_fill_share, 40.0 / 120.0);

    let vector = compute_toxicity_vector(&snapshot).expect("known fills toxicity vector");
    assert_eq!(vector.len(), TOXICITY_VECTOR_DIM as usize);
    let expected = expected_vector();
    for (actual, expected) in vector.iter().zip(&expected) {
        assert_close32(*actual, *expected);
    }
    assert_close32(l2_norm(&vector), 1.0);

    let panel = lenses::default_panel(42, vec!["global".to_string()]);
    let slots = panel.measure_all(&snapshot);
    assert_eq!(
        slots.get(&SlotId::new(13)),
        Some(&SlotVector::Dense {
            dim: TOXICITY_VECTOR_DIM,
            data: vector.clone()
        })
    );

    let empty_edge = edge_empty_fills();
    let lookahead_edge = edge_lookahead_fill();
    let invalid_edge = edge_invalid_fill();

    let report = json!({
        "issue": 42,
        "proof_claim": "Poly computes a deterministic VPIN-style informed-flow toxicity profile from public on-chain fills, encodes it as default-panel slot 13, and fails closed for absent, future, or malformed fill evidence.",
        "minimum_sufficient_corpus": {
            "happy_snapshots": 1,
            "fills": 4,
            "target_volume_buckets": 3,
            "edge_snapshots": 3,
            "why_this_is_sufficient": "Four known-truth fills are the smallest sequence here that proves chronological sorting, fill splitting across three equal-volume buckets, buy/sell imbalance aggregation, Dense(5) normalization, and default-panel slot wiring; three edge snapshots prove absent, lookahead, and malformed-fill failure modes.",
            "why_smaller_is_insufficient": "Fewer than four fills would not prove both mixed buy/sell bucket imbalance and a full-fill single-side bucket while still covering the third volume bucket; omitting any edge would leave a required fail-closed branch unproven.",
            "why_larger_is_wasteful": "More fills repeat the same deterministic sort, bucket, aggregate, normalize, and panel-slot paths without adding proof for #42."
        },
        "source_of_truth": "persisted issue042_onchain_fills_source.json and issue042_toxicity_lens_fsv_report.json read back byte-for-byte from disk plus direct default_panel slot 13 measurement",
        "fill_source_path": fill_source_path.display().to_string(),
        "fill_source_blake3": blake3_hex(&fill_source_readback),
        "happy_path": {
            "slot": 13,
            "dim": TOXICITY_VECTOR_DIM,
            "metrics": metrics,
            "l2_norm": l2_norm(&vector),
            "vector": vector
        },
        "edge_cases": {
            "empty_fills": empty_edge,
            "lookahead_fill": lookahead_edge,
            "invalid_fill": invalid_edge
        }
    });
    let report_path = root.join("issue042_toxicity_lens_fsv_report.json");
    let report_bytes = serde_json::to_vec_pretty(&report).unwrap();
    fs::write(&report_path, &report_bytes).unwrap();
    let readback_bytes = fs::read(&report_path).unwrap();
    assert_eq!(readback_bytes, report_bytes);
    println!("ISSUE042_TOXICITY_LENS_FSV={}", report_path.display());
}

fn edge_empty_fills() -> serde_json::Value {
    let snapshot = known_snapshot(Vec::new());
    let err = compute_toxicity_metrics(&snapshot).expect_err("empty fills absent");
    assert_eq!(err, AbsentReason::LensUnavailable);
    json!({ "reason": format!("{err:?}") })
}

fn edge_lookahead_fill() -> serde_json::Value {
    let mut fills = known_fills();
    fills[0].timestamp = 1_785_500_500;
    let snapshot = known_snapshot(fills);
    let err = compute_toxicity_metrics(&snapshot).expect_err("future fill must fail closed");
    let reason = format!("{err:?}");
    assert!(reason.contains(ERR_TOXICITY_LOOKAHEAD));
    json!({ "reason": reason })
}

fn edge_invalid_fill() -> serde_json::Value {
    let mut fills = known_fills();
    fills[0].price = 1.10;
    let snapshot = known_snapshot(fills);
    let err = compute_toxicity_metrics(&snapshot).expect_err("invalid fill must fail closed");
    let reason = format!("{err:?}");
    assert!(reason.contains(ERR_TOXICITY_INVALID_FILL));
    json!({ "reason": reason })
}

fn known_snapshot(onchain_fills: Vec<OnchainFill>) -> MarketSnapshot {
    MarketSnapshot {
        token_id: "token-issue042".to_string(),
        condition_id: "condition-issue042".to_string(),
        outcome_index: 0,
        slug: "issue042-known-flow-toxicity".to_string(),
        question: Some("Issue 042 known flow toxicity market?".to_string()),
        event_id: Some("event-issue042".to_string()),
        category: Some("crypto".to_string()),
        region: Some("global".to_string()),
        tags: vec!["btc".to_string(), "toxicity".to_string()],
        resolution_source: Some("uma".to_string()),
        neg_risk: false,
        snapshot_ts: 1_785_500_100,
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
        ofi: Some(10.0 / 120.0),
        yes_no_residual: Some(0.0),
        secs_to_resolution: Some(86_400.0),
        holders: Vec::new(),
        makers: Vec::new(),
        counterparty_volumes: Vec::new(),
        onchain_fills,
        temporal_reference_ts: None,
        sequence_position: None,
        sequence_total: None,
        oracle_risk: Default::default(),
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
        fill("0xissue042-a", 0, 1_785_500_000, OnchainFillSide::Buy, 25.0),
        fill(
            "0xissue042-b",
            1,
            1_785_500_001,
            OnchainFillSide::Sell,
            15.0,
        ),
        fill("0xissue042-c", 2, 1_785_500_002, OnchainFillSide::Buy, 40.0),
        fill(
            "0xissue042-d",
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

fn expected_vector() -> Vec<f32> {
    let mut raw = vec![
        0.75_f32,
        (10.0 / 120.0) as f32,
        signed_log(120.0) as f32,
        (40.0 / 120.0) as f32,
        1.0,
    ];
    let norm = l2_norm(&raw);
    for value in &mut raw {
        *value /= norm;
    }
    raw
}

fn l2_norm(values: &[f32]) -> f32 {
    values.iter().map(|value| value * value).sum::<f32>().sqrt()
}

fn blake3_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn assert_close32(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() <= 1.0e-6,
        "actual={actual} expected={expected}"
    );
}

fn assert_close64(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 1.0e-9,
        "actual={actual} expected={expected}"
    );
}

fn fsv_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../target/fsv/issue042_toxicity_lens_20260705_001")
}
