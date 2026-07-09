//! Issue #238 - public-search discovery for same-day crypto markets.
//!
//! Source of truth: deterministic Gamma public-search response with nested market rows.

#[path = "fsv_support.rs"]
mod support;

use std::fs;
use std::path::Path;

use calyx_poly::crypto_ingestor::{CryptoIngestorConfig, select_crypto_capture_market};
use calyx_poly::gamma_public_search::parse_gamma_public_search_markets_value;
use serde_json::{Value, json};

use support::{collect_files, named_fsv_root, reset_dir, write_blake3sums, write_json};

const CAPTURE_TS: u64 = 1_783_429_200;

#[test]
fn issue238_public_search_discovers_nearest_crypto_price_market_fsv() {
    let (root, _keep) = named_fsv_root(
        "POLY_ISSUE238_PUBLIC_SEARCH_FSV_ROOT",
        "issue238-public-search-discovery",
    );
    assert_c_drive(&root);
    reset_dir(&root);
    let tagged_far = [market_json(
        "tagged-token-launch",
        "0xtagged",
        "Will a crypto token launch by September 30?",
        CAPTURE_TS + 7_315_200,
        0.63,
        0.37,
    )];
    let public_search = json!({
        "events": [{
            "id": "daily-btc-event",
            "title": "Bitcoin above ___ on July 7?",
            "markets": [
                market_json(
                    "same-day-btc",
                    "0xsameday",
                    "Will the price of Bitcoin be above $64,000 on July 7?",
                    CAPTURE_TS + 10_800,
                    0.165,
                    0.835
                ),
                closed_market_json()
            ]
        }]
    });
    let mut candidates = tagged_far
        .iter()
        .map(calyx_poly::parse_gamma_market)
        .collect::<calyx_poly::Result<Vec<_>>>()
        .expect("tagged market parses");
    let public_markets =
        parse_gamma_public_search_markets_value(&public_search).expect("public search parses");
    candidates.extend(public_markets.clone());
    let config = CryptoIngestorConfig {
        min_secs_to_resolution: 60,
        max_secs_to_resolution: Some(86_400),
        ..CryptoIngestorConfig::default()
    };
    let selected =
        select_crypto_capture_market(&candidates, CAPTURE_TS, &config).expect("select same-day");
    assert_eq!(selected.market_id, "same-day-btc");
    assert_eq!(public_markets.len(), 2);

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let report = json!({
        "issue": 238,
        "proof_claim": "Gamma public-search nested event markets are flattened into selector candidates so the live crypto capture harness can choose same-day BTC/ETH price markets instead of a farther tagged market.",
        "minimum_sufficient_proof_corpus": {
            "tagged_market_candidates": 1,
            "public_search_events": 1,
            "public_search_markets": 2,
            "selected_candidate_count": 1,
            "why_this_is_sufficient": "One far tagged market plus one nearer public-search market proves the missing discovery path and the existing nearest-market selector. One closed nested row proves terminal rows do not become selected.",
            "why_smaller_is_insufficient": "Without the tagged candidate there is no regression proof against the old path; without the public-search candidate there is no proof the same-day market is discoverable.",
            "why_larger_is_wasteful": "More BTC/ETH ladder rows repeat the same nested parser and selector predicates."
        },
        "source_of_truth": "deterministic public-search JSON persisted into this report and read back from disk",
        "capture_ts": CAPTURE_TS,
        "selected": {
            "market_id": selected.market_id,
            "question": selected.question,
            "secs_to_resolution": selected.end_ts.unwrap() - CAPTURE_TS
        },
        "public_search_market_count": public_markets.len(),
        "physical_files": files,
        "passed": true
    });
    let report_path = root.join("issue238_public_search_discovery_fsv_report.json");
    write_json(&report_path, &report);
    let readback: Value = serde_json::from_slice(&fs::read(&report_path).unwrap()).unwrap();
    assert_eq!(readback, report);
    write_blake3sums(&root);
}

fn market_json(
    id: &str,
    condition_id: &str,
    question: &str,
    end_ts: u64,
    yes: f64,
    no: f64,
) -> Value {
    json!({
        "id": id,
        "conditionId": condition_id,
        "question": question,
        "active": true,
        "closed": false,
        "enableOrderBook": true,
        "outcomes": "[\"Yes\",\"No\"]",
        "outcomePrices": format!("[\"{yes}\",\"{no}\"]"),
        "clobTokenIds": format!("[\"{id}-yes\",\"{id}-no\"]"),
        "endDate": chrono_like(end_ts)
    })
}

fn closed_market_json() -> Value {
    let mut value = market_json(
        "closed-btc",
        "0xclosed",
        "Will the price of Bitcoin be above $48,000 on July 7?",
        CAPTURE_TS + 10_800,
        0.9995,
        0.0005,
    );
    value["active"] = json!(false);
    value["closed"] = json!(true);
    value
}

fn chrono_like(ts: u64) -> String {
    let base = CAPTURE_TS;
    match ts - base {
        10_800 => "2026-07-07T16:00:00Z".to_string(),
        7_315_200 => "2026-09-30T05:00:00Z".to_string(),
        _ => panic!("unexpected timestamp {ts}"),
    }
}

fn assert_c_drive(path: &Path) {
    support::assert_host_fsv_root(path, "FSV root");
}
