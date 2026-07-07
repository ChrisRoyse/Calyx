use std::fs;
use std::path::PathBuf;

use calyx_core::{AbsentReason, SlotId, SlotVector};
use calyx_poly::{
    BOOK_SHAPE_VECTOR_DIM, Book, Level, MarketSnapshot, compute_book_shape_vector, lenses,
};
use serde_json::json;

#[test]
fn issue041_book_shape_lens_fsv() {
    let root = fsv_root();
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let root = fs::canonicalize(&root).expect("canonical FSV root");

    let snapshot = snapshot_with_book(known_book());
    let vector = compute_book_shape_vector(&snapshot).expect("known book vector");
    assert_eq!(vector.len(), BOOK_SHAPE_VECTOR_DIM as usize);
    let expected = expected_known_vector();
    assert_eq!(vector.len(), expected.len());
    for (actual, expected) in vector.iter().zip(&expected) {
        assert_close(*actual, *expected);
    }
    assert_close(l2_norm(&vector), 1.0);

    let panel = lenses::default_panel(41, vec!["global".to_string()]);
    let slots = panel.measure_all(&snapshot);
    assert_eq!(
        slots.get(&SlotId::new(12)),
        Some(&SlotVector::Dense {
            dim: BOOK_SHAPE_VECTOR_DIM,
            data: vector.clone(),
        })
    );

    let empty = edge_empty_book_is_absent();
    let crossed = edge_crossed_book_fails_closed();
    let invalid = edge_invalid_level_fails_closed();

    let report = json!({
        "issue": 41,
        "proof_claim": "Poly has a deterministic embedder-free book_shape SignalLens that encodes a multi-level CLOB depth profile into the default panel and fails closed for empty, crossed, or invalid books.",
        "minimum_sufficient_corpus": {
            "happy_snapshots": 1,
            "bid_levels": 5,
            "ask_levels": 5,
            "edge_snapshots": 3,
            "why_this_is_sufficient": "One five-level-per-side book is the smallest corpus that proves top-of-book fields plus the required L1..L5 bid/ask cumulative depth profile; three edge snapshots prove absent, crossed, and invalid-level fail-closed behavior.",
            "why_smaller_is_insufficient": "Fewer than five levels would not prove the accepted L1..L5 profile; omitting any edge leaves a required book-integrity branch unproven.",
            "why_larger_is_wasteful": "More books or deeper books repeat the same deterministic sort, cumulative-depth, vector, and panel-slot paths without adding proof for #41."
        },
        "source_of_truth": "persisted issue041_book_shape_lens_fsv_report.json read back from disk plus direct default_panel slot 12 measurement",
        "happy_path": {
            "slot": 12,
            "dim": BOOK_SHAPE_VECTOR_DIM,
            "l2_norm": l2_norm(&vector),
            "vector": vector
        },
        "edge_cases": {
            "empty_book": empty,
            "crossed_book": crossed,
            "invalid_level": invalid
        }
    });
    let report_path = root.join("issue041_book_shape_lens_fsv_report.json");
    let report_bytes = serde_json::to_vec_pretty(&report).unwrap();
    fs::write(&report_path, &report_bytes).unwrap();
    let readback_bytes = fs::read(&report_path).unwrap();
    assert_eq!(readback_bytes, report_bytes);
    let readback: serde_json::Value = serde_json::from_slice(&readback_bytes).unwrap();
    assert_eq!(readback["issue"], json!(41));
    assert_eq!(readback["happy_path"]["dim"], json!(BOOK_SHAPE_VECTOR_DIM));
    println!("ISSUE041_BOOK_SHAPE_LENS_FSV={}", report_path.display());
}

fn edge_empty_book_is_absent() -> serde_json::Value {
    let snapshot = snapshot_with_book((Vec::new(), Vec::new()));
    let err = compute_book_shape_vector(&snapshot).expect_err("empty book absent");
    assert_eq!(err, AbsentReason::LensUnavailable);
    json!({ "reason": format!("{err:?}") })
}

fn edge_crossed_book_fails_closed() -> serde_json::Value {
    let snapshot = snapshot_with_book((vec![level(0.55, 100.0)], vec![level(0.52, 100.0)]));
    let err = compute_book_shape_vector(&snapshot).expect_err("crossed book absent error");
    let text = format!("{err:?}");
    assert!(text.contains("CALYX_POLY_BOOK_SHAPE_CROSSED"));
    json!({ "reason": text })
}

fn edge_invalid_level_fails_closed() -> serde_json::Value {
    let snapshot = snapshot_with_book((vec![level(0.48, -1.0)], vec![level(0.52, 100.0)]));
    let err = compute_book_shape_vector(&snapshot).expect_err("invalid level absent error");
    let text = format!("{err:?}");
    assert!(text.contains("CALYX_POLY_BOOK_SHAPE_INVALID"));
    json!({ "reason": text })
}

fn snapshot_with_book(book: (Vec<Level>, Vec<Level>)) -> MarketSnapshot {
    MarketSnapshot {
        token_id: "issue041-token".to_string(),
        condition_id: "issue041-condition".to_string(),
        outcome_index: 0,
        slug: "issue041-book-shape".to_string(),
        question: Some("Issue 041 book shape known-truth market".to_string()),
        event_id: Some("issue041-event".to_string()),
        category: Some("crypto".to_string()),
        region: Some("global".to_string()),
        tags: vec!["book".to_string()],
        resolution_source: Some("uma".to_string()),
        neg_risk: false,
        snapshot_ts: 1_785_600_041,
        price: Some(0.50),
        mid: Some(0.50),
        best_bid: Some(0.48),
        best_ask: Some(0.52),
        spread: Some(0.04),
        tick_size: Some(0.01),
        volume_24h: Some(10_000.0),
        liquidity: Some(1_000.0),
        one_hour_change: Some(0.01),
        one_day_change: Some(0.02),
        ofi: Some(0.10),
        yes_no_residual: Some(0.0),
        secs_to_resolution: Some(3_600.0),
        holders: Vec::new(),
        makers: Vec::new(),
        counterparty_volumes: Vec::new(),
        onchain_fills: Vec::new(),
        temporal_reference_ts: None,
        sequence_position: None,
        sequence_total: None,
        oracle_risk: Default::default(),
        book: Book {
            bids: book.0,
            asks: book.1,
        },
    }
}

fn known_book() -> (Vec<Level>, Vec<Level>) {
    (
        vec![
            level(0.48, 100.0),
            level(0.47, 50.0),
            level(0.46, 25.0),
            level(0.45, 10.0),
            level(0.44, 5.0),
        ],
        vec![
            level(0.52, 80.0),
            level(0.53, 20.0),
            level(0.54, 10.0),
            level(0.55, 5.0),
            level(0.56, 5.0),
        ],
    )
}

fn level(price: f64, size: f64) -> Level {
    Level { price, size }
}

fn signed_log_f32(value: f64) -> f32 {
    (1.0 + value.abs()).ln() as f32
}

fn expected_known_vector() -> Vec<f32> {
    let mut raw = vec![
        0.48_f32,
        0.52,
        0.04,
        ((190.0 - 120.0) / 310.0) as f32,
        signed_log_f32(100.0),
        signed_log_f32(150.0),
        signed_log_f32(175.0),
        signed_log_f32(185.0),
        signed_log_f32(190.0),
        signed_log_f32(80.0),
        signed_log_f32(100.0),
        signed_log_f32(110.0),
        signed_log_f32(115.0),
        signed_log_f32(120.0),
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

fn assert_close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() <= 1.0e-6,
        "actual={actual} expected={expected}"
    );
}

fn fsv_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../target/fsv/issue041_book_shape_lens_20260705_001")
}
