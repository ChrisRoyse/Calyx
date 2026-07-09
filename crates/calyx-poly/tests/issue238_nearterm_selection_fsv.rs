//! Issue #238 - near-term live capture selection.
//!
//! Source of truth: deterministic known-truth Gamma candidates plus persisted selector report.

#[path = "fsv_support.rs"]
mod support;

use std::fs;
use std::path::Path;

use calyx_poly::crypto_ingestor::{
    CryptoIngestorConfig, ERR_CRYPTO_INGESTOR_NO_MARKET, select_crypto_capture_market,
};
use calyx_poly::{GammaJoinKey, GammaMarketRecord, GammaOutcomeShape};
use serde_json::{Value, json};

use support::{collect_files, named_fsv_root, reset_dir, write_blake3sums, write_json};

const CAPTURE_TS: u64 = 1_785_600_000;

#[test]
fn issue238_nearterm_crypto_selection_known_truth_fsv() {
    let (root, _keep) = named_fsv_root(
        "POLY_ISSUE238_NEARTERM_FSV_ROOT",
        "issue238-nearterm-selection",
    );
    assert_c_drive(&root);
    reset_dir(&root);
    let candidates = vec![
        market("far-eligible", CAPTURE_TS + 7_200, true, true),
        market("too-soon", CAPTURE_TS + 30, true, true),
        market("near-eligible", CAPTURE_TS + 600, true, true),
        market("non-binary", CAPTURE_TS + 120, false, true),
        market("no-book", CAPTURE_TS + 90, true, false),
    ];
    let config = CryptoIngestorConfig {
        min_secs_to_resolution: 60,
        max_secs_to_resolution: Some(10_000),
        ..CryptoIngestorConfig::default()
    };
    let selected =
        select_crypto_capture_market(&candidates, CAPTURE_TS, &config).expect("select nearest");
    assert_eq!(selected.market_id, "near-eligible");
    let excluded_config = CryptoIngestorConfig {
        min_secs_to_resolution: 60,
        max_secs_to_resolution: Some(10_000),
        excluded_condition_ids: vec!["0xnear-eligible".to_string()],
        ..CryptoIngestorConfig::default()
    };
    let selected_after_exclusion =
        select_crypto_capture_market(&candidates, CAPTURE_TS, &excluded_config)
            .expect("select nearest non-excluded");
    assert_eq!(selected_after_exclusion.market_id, "far-eligible");

    let too_tight = CryptoIngestorConfig {
        min_secs_to_resolution: 60,
        max_secs_to_resolution: Some(500),
        ..CryptoIngestorConfig::default()
    };
    let err = select_crypto_capture_market(&candidates, CAPTURE_TS, &too_tight)
        .expect_err("over-tight max window must fail closed");
    assert_eq!(err.code(), ERR_CRYPTO_INGESTOR_NO_MARKET);

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let report = json!({
        "issue": 238,
        "proof_claim": "The live crypto selector chooses the nearest eligible future binary market for capture, skips terminal-adjacent rows below the configured minimum window, and fails closed when no candidate fits the configured window.",
        "minimum_sufficient_proof_corpus": {
            "known_truth_candidate_count": candidates.len(),
            "selected_candidate_count": 1,
            "edge_cases": 4,
            "why_this_is_sufficient": "Five known-truth candidates are the smallest corpus that proves nearest-wins among two valid markets, exclusion promotes the next eligible market, plus too-soon, non-binary, and disabled-order-book rejection paths.",
            "why_smaller_is_insufficient": "Fewer candidates would not simultaneously prove nearest-vs-far ordering, exclusion behavior, and the three skip/fail-closed edges.",
            "why_larger_is_wasteful": "More markets repeat the same selector predicates without adding a #238 invariant."
        },
        "source_of_truth": "deterministic GammaMarketRecord candidates persisted into this report and read back from disk",
        "capture_ts": CAPTURE_TS,
        "config": {
            "min_secs_to_resolution": config.min_secs_to_resolution,
            "max_secs_to_resolution": config.max_secs_to_resolution
        },
        "selected": {
            "market_id": selected.market_id,
            "secs_to_resolution": selected.end_ts.unwrap() - CAPTURE_TS
        },
        "edges": {
            "too_soon_secs": 30,
            "non_binary_market_id": "non-binary",
            "disabled_order_book_market_id": "no-book",
            "excluded_condition_id": "0xnear-eligible",
            "selected_after_exclusion": selected_after_exclusion.market_id,
            "over_tight_window_error": err.code()
        },
        "candidates": candidates,
        "physical_files": files,
        "passed": true
    });
    let report_path = root.join("issue238_nearterm_selection_fsv_report.json");
    write_json(&report_path, &report);
    let readback: Value =
        serde_json::from_slice(&fs::read(&report_path).expect("read report")).expect("decode");
    assert_eq!(readback, report);
    write_blake3sums(&root);
}

fn market(id: &str, end_ts: u64, binary: bool, enable_order_book: bool) -> GammaMarketRecord {
    let outcomes = if binary {
        vec!["Yes".to_string(), "No".to_string()]
    } else {
        vec!["A".to_string(), "B".to_string(), "C".to_string()]
    };
    let outcome_prices = if binary {
        vec![0.45, 0.55]
    } else {
        vec![0.2, 0.3, 0.5]
    };
    let clob_token_ids = outcomes
        .iter()
        .enumerate()
        .map(|(index, _)| format!("{id}-token-{index}"))
        .collect::<Vec<_>>();
    GammaMarketRecord {
        market_id: id.to_string(),
        condition_id: format!("0x{id}"),
        slug: Some(format!("{id}-slug")),
        question: Some(format!("Will {id} resolve soon?")),
        event_id: Some(format!("{id}-event")),
        event_slug: Some(format!("{id}-event-slug")),
        active: true,
        closed: false,
        neg_risk: false,
        enable_order_book: Some(enable_order_book),
        outcomes: outcomes.clone(),
        outcome_prices,
        clob_token_ids: clob_token_ids.clone(),
        outcome_shape: if binary {
            GammaOutcomeShape::Binary
        } else {
            GammaOutcomeShape::NonBinary
        },
        category: Some("crypto".to_string()),
        resolution_source: Some("uma".to_string()),
        volume_24h: Some(100.0),
        liquidity: Some(1_000.0),
        best_bid: Some(0.44),
        best_ask: Some(0.46),
        spread: Some(0.02),
        last_trade_price: Some(0.45),
        end_ts: Some(end_ts),
        join_key: GammaJoinKey {
            market_id: id.to_string(),
            condition_id: format!("0x{id}"),
            token_ids: clob_token_ids,
            event_id: Some(format!("{id}-event")),
        },
    }
}

fn assert_c_drive(path: &Path) {
    support::assert_host_fsv_root(path, "FSV root");
}
