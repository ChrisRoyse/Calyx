//! Issue #24 - Gamma API client FSV.
//!
//! Source of truth for live FSV: persisted Gamma HTTP body bytes read back from disk and parsed
//! into market records with condition/token/event join keys.

#[path = "fsv_support.rs"]
mod support;

use std::fs;
use std::path::Path;

use calyx_poly::{
    ERR_GAMMA_JSON, ERR_GAMMA_MARKET_INVALID, ERR_GAMMA_METADATA_INVALID,
    ERR_GAMMA_REQUEST_INVALID, GAMMA_CRYPTO_TAG_ID, GammaClient, GammaClientConfig,
    GammaEventsPage, GammaEventsRequest, GammaMarketsPage, GammaMarketsRequest, GammaOutcomeShape,
    GammaSeriesPage, GammaSeriesRequest, GammaTagsPage, GammaTagsRequest, parse_gamma_events_value,
    parse_gamma_market, parse_gamma_markets_value, parse_gamma_series_value,
    parse_gamma_tags_value,
};
use serde_json::{Value, json};

use support::{collect_files, named_fsv_root, reset_dir, write_blake3sums, write_json};

#[test]
fn issue024_gamma_parser_known_truth_edges() {
    let empty = parse_gamma_markets_value(&json!([])).expect("empty page parses");
    assert!(empty.is_empty(), "zero-limit/empty result is not an error");
    assert!(
        parse_gamma_markets_value(&json!({"data": []}))
            .expect("explicit empty rows parse")
            .is_empty()
    );
    for malformed in [json!({}), json!({"markets": {}})] {
        let error = parse_gamma_markets_value(&malformed)
            .expect_err("object without a rows array fails closed");
        assert_eq!(error.code(), ERR_GAMMA_JSON);
    }
    let metadata_error = parse_gamma_events_value(&json!({"status": "ok"}))
        .expect_err("metadata object without rows fails closed");
    assert_eq!(metadata_error.code(), ERR_GAMMA_METADATA_INVALID);

    let binary = parse_gamma_market(&binary_market()).expect("binary market parses");
    assert_eq!(binary.outcome_shape, GammaOutcomeShape::Binary);
    assert_eq!(binary.condition_id, "0x24binary");
    assert_eq!(binary.clob_token_ids, vec!["tok_yes", "tok_no"]);
    assert_eq!(binary.join_key.event_id.as_deref(), Some("evt24"));
    assert_eq!(binary.volume_24h, Some(123.45));
    assert_eq!(binary.liquidity, Some(678.9));

    let non_binary = parse_gamma_market(&json!({
        "id": "m24multi",
        "conditionId": "0x24multi",
        "slug": "multi-outcome-known-truth",
        "active": true,
        "closed": false,
        "outcomes": ["A", "B", "C"],
        "outcomePrices": [0.2, 0.3, 0.5],
        "clobTokenIds": ["tok_a", "tok_b", "tok_c"]
    }))
    .expect("non-binary market parses");
    assert_eq!(non_binary.outcome_shape, GammaOutcomeShape::NonBinary);
    assert_eq!(
        non_binary.clob_token_ids,
        vec!["tok_a", "tok_b", "tok_c"],
        "non-binary rows are classified, not coerced to YES/NO"
    );

    let err = parse_gamma_market(&json!({
        "id": "m24bad",
        "conditionId": "0x24bad",
        "active": true,
        "closed": false,
        "outcomes": "[\"Yes\", \"No\"]",
        "outcomePrices": "[\"0.4\", \"0.6\"]",
        "clobTokenIds": "not-json"
    }))
    .expect_err("malformed JSON-string field must fail closed");
    assert_eq!(err.code(), ERR_GAMMA_MARKET_INVALID);

    let client = GammaClient::new(GammaClientConfig::default()).expect("client config");
    let err = client
        .fetch_markets(&GammaMarketsRequest::crypto_active(501))
        .expect_err("oversized request rejected before network");
    assert_eq!(err.code(), ERR_GAMMA_REQUEST_INVALID);
}

#[test]
#[ignore = "requires live public Gamma API"]
fn issue024_gamma_client_live_fsv() {
    let (root, _keep) = named_fsv_root("POLY_ISSUE024_FSV_ROOT", "issue024-gamma-client");
    reset_dir(&root);
    let client = GammaClient::new(GammaClientConfig::default()).expect("Gamma client");

    let active = client
        .fetch_markets(&GammaMarketsRequest::crypto_active(5))
        .expect("fetch live active crypto Gamma markets");
    let closed = client
        .fetch_markets(&GammaMarketsRequest::crypto_closed(5))
        .expect("fetch live closed crypto Gamma markets");
    let events = client
        .fetch_events(&GammaEventsRequest::crypto_active(5))
        .expect("fetch live active crypto Gamma events");
    let series = client
        .fetch_series(&GammaSeriesRequest::with_limit(5))
        .expect("fetch live Gamma series metadata");
    let tags = client
        .fetch_tags(&GammaTagsRequest::with_limit(5))
        .expect("fetch live Gamma tag metadata");

    assert_live_page(&active, Some(true), Some(false));
    assert_live_page(&closed, None, Some(true));
    assert_eq!(events.events.len(), 5);
    assert!(
        events
            .events
            .iter()
            .all(|event| event.active && !event.closed)
    );
    assert_eq!(series.series.len(), 5);
    assert_eq!(tags.tags.len(), 5);
    let active_readback = persist_page(&root, "active-crypto", &active);
    let closed_readback = persist_page(&root, "closed-crypto", &closed);
    let events_readback = persist_events_page(&root, "events-crypto-active", &events);
    let series_readback = persist_series_page(&root, "series-metadata", &series);
    let tags_readback = persist_tags_page(&root, "tags-metadata", &tags);

    let empty = parse_gamma_markets_value(&json!([])).expect("empty edge parses");
    let malformed = parse_gamma_market(&json!({
        "id": "m24livebad",
        "conditionId": "0x24livebad",
        "active": true,
        "closed": false,
        "outcomes": "[\"Yes\", \"No\"]",
        "outcomePrices": "[\"0.4\", \"0.6\"]",
        "clobTokenIds": "not-json"
    }))
    .expect_err("malformed edge fails");
    let multi = parse_gamma_market(&json!({
        "id": "m24livemulti",
        "conditionId": "0x24livemulti",
        "active": true,
        "closed": false,
        "outcomes": ["Up", "Flat", "Down"],
        "outcomePrices": ["0.2", "0.3", "0.5"],
        "clobTokenIds": ["tok_up", "tok_flat", "tok_down"]
    }))
    .expect("multi edge parses");

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let report = json!({
        "issue": 24,
        "proof_claim": "The Gamma client fetches active and closed crypto-tagged market rows plus Gamma event, series, and tag metadata, persists and reads back the raw body bytes, decodes Gamma JSON-string fields for outcomes/prices/token IDs, and surfaces condition/token/event join keys without binary coercion.",
        "minimum_sufficient_proof_corpus": {
            "live_active_crypto_rows": active.markets.len(),
            "live_closed_crypto_rows": closed.markets.len(),
            "live_active_crypto_events": events.events.len(),
            "live_series_rows": series.series.len(),
            "live_tag_rows": tags.tags.len(),
            "synthetic_edge_cases": 3,
            "tag_id": GAMMA_CRYPTO_TAG_ID,
            "why_this_is_sufficient": "Five active and five closed crypto-tagged live market rows prove both required Gamma discovery modes and multiple real JSON-string market rows while keeping network and disk use tiny. Five active crypto-tagged events plus five series rows and five tag rows prove the remaining Gamma metadata endpoints. Three known-truth edges prove empty/zero-limit semantics, malformed fail-closed behavior, and non-binary classification.",
            "why_smaller_is_insufficient": "A single live page would not prove both active and closed discovery. Removing the synthetic edges would not prove the required fail-closed and non-binary behavior.",
            "why_larger_is_wasteful": "Larger pages repeat the same HTTP, readback, JSON-string decoding, and join-key extraction paths without adding a #24 invariant."
        },
        "source_of_truth": "live public Gamma HTTP response bodies persisted under this FSV root and parsed only after disk readback",
        "active": active_readback,
        "closed": closed_readback,
        "events": events_readback,
        "series": series_readback,
        "tags": tags_readback,
        "edge_cases": {
            "empty_zero_limit": {"parsed_count": empty.len()},
            "malformed_market": {"code": malformed.code()},
            "non_binary_classified": {
                "shape": format!("{:?}", multi.outcome_shape),
                "token_count": multi.clob_token_ids.len()
            }
        },
        "physical_files": files,
        "passed": true
    });
    let report_path = root.join("issue024_gamma_client_fsv_report.json");
    write_json(&report_path, &report);
    let readback: Value =
        serde_json::from_slice(&fs::read(&report_path).expect("read report")).expect("decode");
    assert_eq!(readback, report);
    write_blake3sums(&root);
    println!("ISSUE024_GAMMA_CLIENT_FSV={}", report_path.display());
}

fn persist_events_page(root: &Path, name: &str, page: &GammaEventsPage) -> Value {
    persist_metadata_page(
        MetadataPersistInput {
            root,
            name,
            raw_body: &page.raw_body,
            body_sha256: &page.body_sha256,
            body_bytes: page.body_bytes,
            status_code: page.status_code,
            url: &page.url,
            record_key: "events",
        },
        |value| Ok(serde_json::to_value(parse_gamma_events_value(value)?).expect("events JSON")),
    )
}

fn persist_series_page(root: &Path, name: &str, page: &GammaSeriesPage) -> Value {
    persist_metadata_page(
        MetadataPersistInput {
            root,
            name,
            raw_body: &page.raw_body,
            body_sha256: &page.body_sha256,
            body_bytes: page.body_bytes,
            status_code: page.status_code,
            url: &page.url,
            record_key: "series",
        },
        |value| Ok(serde_json::to_value(parse_gamma_series_value(value)?).expect("series JSON")),
    )
}

fn persist_tags_page(root: &Path, name: &str, page: &GammaTagsPage) -> Value {
    persist_metadata_page(
        MetadataPersistInput {
            root,
            name,
            raw_body: &page.raw_body,
            body_sha256: &page.body_sha256,
            body_bytes: page.body_bytes,
            status_code: page.status_code,
            url: &page.url,
            record_key: "tags",
        },
        |value| Ok(serde_json::to_value(parse_gamma_tags_value(value)?).expect("tags JSON")),
    )
}

struct MetadataPersistInput<'a> {
    root: &'a Path,
    name: &'a str,
    raw_body: &'a [u8],
    body_sha256: &'a str,
    body_bytes: u64,
    status_code: u16,
    url: &'a str,
    record_key: &'a str,
}

fn persist_metadata_page<F>(input: MetadataPersistInput<'_>, parse: F) -> Value
where
    F: FnOnce(&Value) -> Result<Value, calyx_poly::PolyError>,
{
    let case_root = input.root.join(input.name);
    fs::create_dir_all(&case_root).expect("create case root");
    let body_path = case_root.join("body.json");
    let parsed_path = case_root.join("parsed-records.json");
    let summary_path = case_root.join("summary.json");
    fs::write(&body_path, input.raw_body).expect("write raw metadata body");
    let raw_readback = fs::read(&body_path).expect("read raw metadata body");
    assert_eq!(
        raw_readback, input.raw_body,
        "raw metadata readback is exact"
    );
    let value: Value = serde_json::from_slice(&raw_readback).expect("decode raw metadata");
    let parsed = parse(&value).expect("parse metadata readback");
    let record_count = parsed.as_array().map(Vec::len).unwrap_or_default();
    write_json(&parsed_path, &parsed);
    let summary = json!({
        "url": input.url,
        "status_code": input.status_code,
        "body_path": body_path.display().to_string(),
        "body_bytes": input.body_bytes,
        "body_sha256": input.body_sha256,
        "parsed_path": parsed_path.display().to_string(),
        "record_key": input.record_key,
        "parsed_count": record_count,
        "readback_equal": true
    });
    write_json(&summary_path, &summary);
    let summary_readback: Value =
        serde_json::from_slice(&fs::read(&summary_path).expect("read summary")).expect("decode");
    assert_eq!(summary_readback, summary);
    summary_readback
}

fn assert_live_page(
    page: &GammaMarketsPage,
    expect_active: Option<bool>,
    expect_closed: Option<bool>,
) {
    assert_eq!(page.status_code, 200);
    assert_eq!(page.markets.len(), 5);
    assert!(page.body_bytes > 0);
    assert_eq!(page.body_sha256.len(), 64);
    for market in &page.markets {
        if let Some(active) = expect_active {
            assert_eq!(market.active, active);
        }
        if let Some(closed) = expect_closed {
            assert_eq!(market.closed, closed);
        }
        assert!(!market.condition_id.trim().is_empty());
        assert_eq!(market.outcomes.len(), market.outcome_prices.len());
        assert_eq!(market.outcomes.len(), market.clob_token_ids.len());
        assert!(!market.join_key.token_ids.is_empty());
    }
}

fn persist_page(root: &Path, name: &str, page: &GammaMarketsPage) -> Value {
    let case_root = root.join(name);
    fs::create_dir_all(&case_root).expect("create case root");
    let body_path = case_root.join("body.json");
    let parsed_path = case_root.join("parsed-markets.json");
    let summary_path = case_root.join("summary.json");
    fs::write(&body_path, &page.raw_body).expect("write raw body");
    let raw_readback = fs::read(&body_path).expect("read raw body");
    assert_eq!(raw_readback, page.raw_body, "raw body readback is exact");
    let parsed_value: Value = serde_json::from_slice(&raw_readback).expect("decode raw readback");
    let parsed_readback =
        parse_gamma_markets_value(&parsed_value).expect("parse raw readback into markets");
    assert_eq!(parsed_readback, page.markets);
    write_json(
        &parsed_path,
        &serde_json::to_value(&parsed_readback).expect("parsed JSON"),
    );
    let summary = json!({
        "url": page.url,
        "status_code": page.status_code,
        "body_path": body_path.display().to_string(),
        "body_bytes": page.body_bytes,
        "body_sha256": page.body_sha256,
        "parsed_path": parsed_path.display().to_string(),
        "parsed_count": parsed_readback.len(),
        "first_join_key": parsed_readback.first().map(|market| &market.join_key),
        "readback_equal": true
    });
    write_json(&summary_path, &summary);
    let summary_readback: Value =
        serde_json::from_slice(&fs::read(&summary_path).expect("read summary")).expect("decode");
    assert_eq!(summary_readback, summary);
    summary_readback
}

fn binary_market() -> Value {
    json!({
        "id": "m24binary",
        "conditionId": "0x24binary",
        "slug": "bitcoin-known-truth",
        "question": "Will Bitcoin close above the threshold?",
        "active": true,
        "closed": false,
        "negRisk": false,
        "enableOrderBook": true,
        "events": [{"id": "evt24", "slug": "bitcoin-event"}],
        "outcomes": "[\"Yes\", \"No\"]",
        "outcomePrices": "[\"0.42\", \"0.58\"]",
        "clobTokenIds": "[\"tok_yes\", \"tok_no\"]",
        "volume24hr": "123.45",
        "liquidityNum": "678.9",
        "bestBid": 0.41,
        "bestAsk": 0.43,
        "spread": 0.02,
        "lastTradePrice": 0.42
    })
}
