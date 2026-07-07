//! Issue #94 - book-depth and liquidity feature extraction FSV.
//!
//! Source of truth: public CLOB book snapshot artifacts and normalized local feature rows, read
//! back from disk independently.

use std::path::Path;

use calyx_poly::admission::{
    AdmissionDecision, AdmissionInputs, AdmissionParams, evaluate_admission,
};
use calyx_poly::book_liquidity::{
    BOOK_LIQUIDITY_SCHEMA_VERSION, BookLiquidityFeatureRequest, BookLiquidityFeatureRow,
    BookLiquidityStatus, ERR_BOOK_LIQUIDITY_CROSSED, PUBLIC_BOOK_SNAPSHOT_ARTIFACT_KIND,
    PublicBookLevel, PublicBookSnapshot, read_book_liquidity_features, read_public_book_snapshot,
    run_book_liquidity_feature_extraction,
};
use serde_json::{Value, json};

#[path = "fsv_support.rs"]
mod support;
use support::{
    collect_files, known_healthy_market_integrity, known_healthy_oracle_risk,
    known_healthy_wash_trade, named_fsv_root, reset_dir, write_blake3sums, write_json,
};

const SNAPSHOT_TS: u64 = 1_785_600_094;

#[test]
fn issue094_book_liquidity_fsv() {
    let (root, _keep) = named_fsv_root("POLY_ISSUE094_FSV_ROOT", "poly-issue094-book");
    reset_dir(&root);

    let happy = happy_known_book_extracts_expected_features(&root);
    let empty = edge_empty_book_marks_degraded_without_admission(&root);
    let crossed = edge_crossed_book_fails_closed(&root);
    let stale = edge_stale_snapshot_marks_degraded_without_admission(&root);

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let readback = json!({
        "issue": 94,
        "proof_claim": "Poly extracts deterministic read-only CLOB book-depth, midpoint, spread, volume, visible-liquidity, and liquidity-admission features from a public snapshot artifact, persists normalized rows, and refuses/degrades empty, crossed, and stale books without producing trading fields.",
        "minimum_sufficient_corpus": {
            "happy_public_book_snapshots": 1,
            "happy_bid_levels": 2,
            "happy_ask_levels": 2,
            "edge_snapshots": 3,
            "why_this_is_sufficient": "Two bid and two ask levels are the smallest book that proves both top-of-book math and multi-level depth aggregation; one snapshot each proves empty, crossed, and stale fail-closed behavior.",
            "why_smaller_is_insufficient": "One level per side would not prove depth aggregation beyond top-of-book, and omitting any edge would leave one #94 refusal/degraded path unproven.",
            "why_larger_is_wasteful": "More levels or snapshots would repeat the same sum, min/max, readback, and admission-liquidity paths without adding proof; scale is not the #94 claim."
        },
        "official_source_context": {
            "public_endpoint": "https://clob.polymarket.com/book",
            "docs": "https://docs.polymarket.com/api-reference/market-data/get-order-book"
        },
        "happy_path": happy,
        "edge_cases": {
            "empty_book": empty,
            "crossed_book": crossed,
            "stale_snapshot": stale
        },
        "physical_files": files
    });
    let readback_path = root.join("readback.json");
    write_json(&readback_path, &readback);
    write_blake3sums(&root);
    println!(
        "ISSUE094_BOOK_LIQUIDITY_READBACK={}",
        readback_path.display()
    );
}

fn happy_known_book_extracts_expected_features(root: &Path) -> Value {
    let run = run_book_liquidity_feature_extraction(
        &request("happy", known_book(), SNAPSHOT_TS + 10, 60, 90.0),
        &root.join("happy"),
    )
    .expect("happy book extraction");
    let raw = read_public_book_snapshot(&run.raw_snapshot_path).expect("read raw snapshot");
    let row = read_feature(&run.feature_path);
    assert_eq!(raw, run.raw_snapshot);
    assert_eq!(row, run.feature_row);
    assert_eq!(row.status, BookLiquidityStatus::Ready);
    assert_close(row.best_bid.unwrap(), 0.48);
    assert_close(row.best_ask.unwrap(), 0.52);
    assert_close(row.midpoint.unwrap(), 0.50);
    assert_close(row.spread.unwrap(), 0.04);
    assert_close(row.bid_depth, 150.0);
    assert_close(row.ask_depth, 100.0);
    assert_close(row.visible_book_volume, 250.0);
    assert_close(row.visible_liquidity.unwrap(), 100.0);
    assert_close(row.depth_imbalance.unwrap(), 0.20);
    assert!(row.liquidity_ok);
    let decision = admission_from_row(&row);
    assert!(decision.admitted, "{}", decision.reason);
    assert_no_trade_keys(&serde_json::to_value(&row).expect("row JSON"));
    evidence(&row, &decision)
}

fn edge_empty_book_marks_degraded_without_admission(root: &Path) -> Value {
    let run = run_book_liquidity_feature_extraction(
        &request("empty", (Vec::new(), Vec::new()), SNAPSHOT_TS + 10, 60, 1.0),
        &root.join("edge-empty"),
    )
    .expect("empty book degrades");
    let row = read_feature(&run.feature_path);
    assert_eq!(row.status, BookLiquidityStatus::EmptyBook);
    assert!(row.degraded);
    assert!(!row.liquidity_ok);
    assert_eq!(row.best_bid, None);
    assert_eq!(row.spread, None);
    let decision = admission_from_row(&row);
    assert!(!decision.admitted);
    assert_eq!(decision.code, "CALYX_POLY_ADMISSION_INSUFFICIENT_LIQUIDITY");
    evidence(&row, &decision)
}

fn edge_crossed_book_fails_closed(root: &Path) -> Value {
    let err = run_book_liquidity_feature_extraction(
        &request(
            "crossed",
            (vec![level(0.55, 100.0)], vec![level(0.52, 100.0)]),
            SNAPSHOT_TS + 10,
            60,
            1.0,
        ),
        &root.join("edge-crossed"),
    )
    .expect_err("crossed book must fail closed");
    assert_eq!(err.code(), ERR_BOOK_LIQUIDITY_CROSSED);
    json!({"code": err.code(), "message": err.message()})
}

fn edge_stale_snapshot_marks_degraded_without_admission(root: &Path) -> Value {
    let run = run_book_liquidity_feature_extraction(
        &request("stale", known_book(), SNAPSHOT_TS + 120, 60, 90.0),
        &root.join("edge-stale"),
    )
    .expect("stale book degrades");
    let row = read_feature(&run.feature_path);
    assert_eq!(row.status, BookLiquidityStatus::Stale);
    assert!(row.degraded);
    assert!(!row.liquidity_ok);
    assert_eq!(row.midpoint, Some(0.50));
    let decision = admission_from_row(&row);
    assert!(!decision.admitted);
    assert_eq!(decision.code, "CALYX_POLY_ADMISSION_INSUFFICIENT_LIQUIDITY");
    evidence(&row, &decision)
}

fn request(
    token_suffix: &str,
    book: (Vec<PublicBookLevel>, Vec<PublicBookLevel>),
    now_ts: u64,
    max_age_seconds: u64,
    min_visible_liquidity: f64,
) -> BookLiquidityFeatureRequest {
    let token_id = format!("issue94-{token_suffix}-token");
    BookLiquidityFeatureRequest {
        snapshot: PublicBookSnapshot {
            schema_version: BOOK_LIQUIDITY_SCHEMA_VERSION.to_string(),
            artifact_kind: PUBLIC_BOOK_SNAPSHOT_ARTIFACT_KIND.to_string(),
            source_kind: "polymarket_clob_book".to_string(),
            source_url: format!("https://clob.polymarket.com/book?token_id={token_id}"),
            condition_id: format!("issue94-{token_suffix}-condition"),
            token_id,
            snapshot_ts: SNAPSHOT_TS,
            captured_ts: SNAPSHOT_TS + 1,
            bids: book.0,
            asks: book.1,
            volume_24h: Some(1_234.5),
        },
        now_ts,
        max_age_seconds,
        min_visible_liquidity,
    }
}

fn known_book() -> (Vec<PublicBookLevel>, Vec<PublicBookLevel>) {
    (
        vec![level(0.48, 100.0), level(0.47, 50.0)],
        vec![level(0.52, 80.0), level(0.53, 20.0)],
    )
}

fn level(price: f64, size: f64) -> PublicBookLevel {
    PublicBookLevel { price, size }
}

fn admission_from_row(row: &BookLiquidityFeatureRow) -> AdmissionDecision {
    let mut inputs = good_inputs();
    inputs.liquidity_ok = row.liquidity_ok;
    evaluate_admission(&AdmissionParams::default(), &inputs)
}

fn good_inputs() -> AdmissionInputs {
    AdmissionInputs {
        p_win: 0.94,
        confidence: 0.74,
        sufficiency_ok: true,
        evidence_count: 2,
        source_derived_evidence_count: 2,
        stale_evidence_count: 0,
        circular_evidence_count: 0,
        super_intel_pass: true,
        guard_calibrated: true,
        grounding_anchor_count: AdmissionParams::default().min_grounding_anchors,
        guard_pass: true,
        liquidity_ok: true,
        market_integrity: known_healthy_market_integrity(),
        oracle_risk: known_healthy_oracle_risk(),
        wash_trade: known_healthy_wash_trade(),
        kill_switch_active: false,
        daily_error_score: 0.0,
    }
}

fn read_feature(path: &Path) -> BookLiquidityFeatureRow {
    read_book_liquidity_features(path).expect("read book liquidity feature row")
}

fn evidence(row: &BookLiquidityFeatureRow, decision: &AdmissionDecision) -> Value {
    json!({
        "token_id": row.token_id,
        "status": row.status,
        "degraded": row.degraded,
        "reason": row.reason,
        "best_bid": row.best_bid,
        "best_ask": row.best_ask,
        "midpoint": row.midpoint,
        "spread": row.spread,
        "bid_depth": row.bid_depth,
        "ask_depth": row.ask_depth,
        "visible_book_volume": row.visible_book_volume,
        "visible_liquidity": row.visible_liquidity,
        "depth_imbalance": row.depth_imbalance,
        "volume_24h": row.volume_24h,
        "liquidity_ok": row.liquidity_ok,
        "raw_snapshot_hash": row.raw_snapshot_hash,
        "admission": decision,
    })
}

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 1.0e-9,
        "actual={actual} expected={expected}"
    );
}

fn assert_no_trade_keys(value: &Value) {
    for key in ["authorized", "stake", "bankroll", "kelly", "order", "pnl"] {
        assert!(value.get(key).is_none(), "trade key survived: {key}");
    }
}
