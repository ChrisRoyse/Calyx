//! Issue #239 - maker-address concentration source semantics.
//!
//! Source of truth for deterministic FSV: persisted report JSON read back from disk after running
//! the real market-integrity screen and Data API projection code. Source of truth for live FSV:
//! one real public CLOB book body read back from disk before checking whether the public shape
//! exposes maker identity fields.

// calyx-shared-module: path=fsv_support.rs alias=__calyx_shared_fsv_support_rs local=support visibility=private
use crate::__calyx_shared_fsv_support_rs as support;

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use calyx_poly::risk::{
    MARKET_INTEGRITY_INVALID_EVIDENCE, MARKET_INTEGRITY_MISSING_MAKER_EVIDENCE, MARKET_INTEGRITY_OK,
};
use calyx_poly::{
    Book, ClobBookPage, ClobClient, ClobClientConfig, DataApiClient, DataApiClientConfig,
    DataApiEvidenceStatus, ERR_DATA_API_MAKER_EVIDENCE_UNAVAILABLE, GammaClient, GammaClientConfig,
    GammaMarketRecord, GammaMarketsRequest, HolderShare, Level, MakerShare,
    MakerShareEvidenceSource, MarketIntegrityParams, MarketSnapshot,
    build_data_api_concentration_inputs, screen_market_integrity,
};
use serde_json::{Value, json};

use support::{collect_files, named_fsv_root, reset_dir, write_blake3sums, write_json};

#[test]
fn issue239_rejects_false_maker_sources_fsv() {
    let (root, keep_root) = named_fsv_root("POLY_ISSUE239_FSV_ROOT", "issue239-maker-source");
    reset_dir(&root);

    let true_maker = screen_market_integrity(
        &snapshot_with_makers(MakerShareEvidenceSource::RestingClobOrderBook),
        &MarketIntegrityParams::default(),
    );
    assert_eq!(true_maker.code, MARKET_INTEGRITY_OK);
    assert!(true_maker.ok);

    let holder_as_maker = false_maker_edge(
        &root,
        "holder-position-wallet-as-maker",
        MakerShareEvidenceSource::HolderOrPositionWallet,
    );
    let trader_as_maker = false_maker_edge(
        &root,
        "trader-counterparty-wallet-as-maker",
        MakerShareEvidenceSource::TraderOrCounterpartyWallet,
    );
    let unknown_as_maker = false_maker_edge(
        &root,
        "unknown-legacy-maker-row",
        MakerShareEvidenceSource::Unknown,
    );

    let data_api = DataApiClient::new(DataApiClientConfig::default()).expect("Data API client");
    let unavailable = data_api
        .require_true_maker_evidence()
        .expect_err("Data API cannot supply true maker evidence");
    assert_eq!(unavailable.code(), ERR_DATA_API_MAKER_EVIDENCE_UNAVAILABLE);

    let concentration = build_data_api_concentration_inputs(
        "0x1111111111111111111111111111111111111111111111111111111111111111",
        holders(10, 100.0),
        vec![],
    );
    assert_eq!(
        concentration.maker_evidence_status,
        DataApiEvidenceStatus::Absent
    );
    assert!(concentration.maker_shares.is_empty());
    let data_api_screen = screen_market_integrity(
        &snapshot_without_makers(),
        &MarketIntegrityParams::default(),
    );
    assert_eq!(
        data_api_screen.code,
        MARKET_INTEGRITY_MISSING_MAKER_EVIDENCE
    );
    assert!(!data_api_screen.ok);

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let report = json!({
        "issue": 239,
        "proof_claim": "Poly distinguishes true resting CLOB maker-address size from holder/position/trader wallet concentration, accepts only true maker provenance for maker screens, and fails closed when Data API maker evidence is unavailable.",
        "minimum_sufficient_proof_corpus": {
            "known_truth_true_maker_snapshots": 1,
            "false_maker_source_edges": 3,
            "data_api_projection_cases": 1,
            "why_this_is_sufficient": "One true-maker snapshot proves the accepted path. Three false-source rows cover holder/position wallets, trader/counterparty wallets, and legacy unknown provenance. One Data API projection proves holder rows remain wallet concentration while maker rows remain unavailable.",
            "why_smaller_is_insufficient": "Without the true-maker path the acceptance contract is unproven. Without all three false-source edges a common bad projection could still pass. Without the Data API projection #239's original failure mode is not covered.",
            "why_larger_is_wasteful": "More markets or larger row counts would repeat the same provenance gate, aggregation, and fail-closed screen paths without proving another invariant."
        },
        "source_of_truth": "report JSON read back from disk after executing DataApiClient::require_true_maker_evidence, build_data_api_concentration_inputs, and screen_market_integrity",
        "true_maker": true_maker,
        "false_source_edges": {
            "holder_position_wallet": holder_as_maker,
            "trader_counterparty_wallet": trader_as_maker,
            "unknown_legacy": unknown_as_maker
        },
        "data_api_projection": {
            "holder_count": concentration.holder_shares.len(),
            "counterparty_count": concentration.counterparty_volumes.len(),
            "maker_count": concentration.maker_shares.len(),
            "maker_evidence_status": concentration.maker_evidence_status,
            "maker_evidence_reason": concentration.maker_evidence_reason,
            "require_true_maker_evidence_code": unavailable.code(),
            "downstream_screen": data_api_screen
        },
        "official_docs_checked": [
            "https://docs.polymarket.com/api-reference/market-data/get-order-book",
            "https://docs.polymarket.com/market-data/overview",
            "https://docs.polymarket.com/market-data/websocket/overview",
            "https://docs.polymarket.com/api-reference/authentication"
        ],
        "physical_files": files
    });
    let report_path = root.join("issue239-maker-source-fsv-report.json");
    write_json(&report_path, &report);
    let readback: Value =
        serde_json::from_slice(&fs::read(&report_path).expect("read report")).expect("decode");
    assert_eq!(readback, report);
    write_blake3sums(&root);
    if keep_root {
        println!("poly_issue239_fsv_root={}", root.display());
    }
    println!("ISSUE239_MAKER_SOURCE_FSV={}", report_path.display());
}

#[test]
#[ignore = "requires live public Gamma and CLOB APIs"]
fn issue239_live_public_clob_book_has_no_maker_identity_fsv() {
    let (root, keep_root) =
        named_fsv_root("POLY_ISSUE239_LIVE_FSV_ROOT", "issue239-live-maker-source");
    reset_dir(&root);
    let gamma = GammaClient::new(GammaClientConfig::default()).expect("Gamma client");
    let clob = ClobClient::new(ClobClientConfig::default()).expect("CLOB client");
    let (market, token, book) = select_live_book(&gamma, &clob);

    let case_root = root.join("live-clob-book");
    fs::create_dir_all(&case_root).expect("create live case root");
    let body_path = case_root.join("body.json");
    fs::write(&body_path, &book.http.raw_body).expect("write live CLOB body");
    let body_readback = fs::read(&body_path).expect("read live CLOB body");
    assert_eq!(body_readback, book.http.raw_body);
    let value: Value = serde_json::from_slice(&body_readback).expect("decode live CLOB body");
    let top_level_keys = object_keys(&value);
    let level_keys = first_level_keys(&value);
    assert!(
        !level_keys.is_empty(),
        "live book must expose at least one bid/ask level to prove level shape"
    );
    assert_no_maker_identity_fields(&top_level_keys);
    assert_no_maker_identity_fields(&level_keys);

    let report = json!({
        "issue": 239,
        "proof_claim": "One real public CLOB /book response for an active Gamma-derived crypto token exposes public price/size levels but no maker identity field, so Poly must not derive MakerShare rows from this source.",
        "minimum_sufficient_proof_corpus": {
            "live_gamma_active_markets": 1,
            "live_clob_books": 1,
            "why_this_is_sufficient": "One active Gamma-derived token proves the public /book wire shape and disk readback path for the source family that would need maker identity. Additional tokens repeat the same public schema.",
            "why_smaller_is_insufficient": "Without one live book, source availability would be inferred from docs rather than source bytes. Without disk readback, the FSV source of truth would be only a return value.",
            "why_larger_is_wasteful": "More books would not add proof unless the claim were coverage or scale; #239 is a schema/provenance claim."
        },
        "source_of_truth": "live public CLOB /book body persisted under this FSV root and read back from disk before schema checks",
        "selected_market": {
            "market_id": market.market_id,
            "condition_id": market.condition_id,
            "token_id": token,
            "slug": market.slug
        },
        "book_http": {
            "method": book.http.method,
            "url": book.http.url,
            "status_code": book.http.status_code,
            "body_bytes": book.http.body_bytes,
            "body_sha256": book.http.body_sha256
        },
        "top_level_keys": top_level_keys,
        "first_level_keys": level_keys,
        "maker_identity_fields_present": false,
        "body_path": body_path.display().to_string()
    });
    let report_path = root.join("issue239-live-clob-book-fsv-report.json");
    write_json(&report_path, &report);
    let readback: Value =
        serde_json::from_slice(&fs::read(&report_path).expect("read report")).expect("decode");
    assert_eq!(readback, report);
    write_blake3sums(&root);
    if keep_root {
        println!("poly_issue239_live_fsv_root={}", root.display());
    }
    println!("ISSUE239_LIVE_MAKER_SOURCE_FSV={}", report_path.display());
}

fn false_maker_edge(root: &Path, name: &str, evidence_source: MakerShareEvidenceSource) -> Value {
    let screen = screen_market_integrity(
        &snapshot_with_makers(evidence_source),
        &MarketIntegrityParams::default(),
    );
    assert_eq!(screen.code, MARKET_INTEGRITY_INVALID_EVIDENCE);
    assert!(!screen.ok);
    let evidence = json!({
        "source_kind": evidence_source,
        "screen": screen
    });
    write_json(&root.join(format!("{name}.json")), &evidence);
    let readback: Value =
        serde_json::from_slice(&fs::read(root.join(format!("{name}.json"))).expect("read edge"))
            .expect("decode edge");
    assert_eq!(readback, evidence);
    readback
}

fn snapshot_without_makers() -> MarketSnapshot {
    snapshot(Vec::new())
}

fn snapshot_with_makers(evidence_source: MakerShareEvidenceSource) -> MarketSnapshot {
    snapshot(
        (0..4)
            .map(|idx| MakerShare {
                maker: format!("0xmaker{idx:02}"),
                size: 250.0,
                evidence_source,
            })
            .collect(),
    )
}

fn snapshot(makers: Vec<MakerShare>) -> MarketSnapshot {
    MarketSnapshot {
        token_id: "issue239-token".to_string(),
        condition_id: "0x1111111111111111111111111111111111111111111111111111111111111111"
            .to_string(),
        outcome_index: 0,
        slug: "issue239-maker-source".to_string(),
        question: Some("Issue 239 maker source?".to_string()),
        event_id: Some("issue239-event".to_string()),
        category: Some("crypto".to_string()),
        region: Some("global".to_string()),
        tags: vec!["issue239".to_string()],
        resolution_source: Some("uma".to_string()),
        neg_risk: false,
        snapshot_ts: 1_785_500_239,
        price: Some(0.55),
        mid: Some(0.55),
        best_bid: Some(0.54),
        best_ask: Some(0.56),
        spread: Some(0.02),
        tick_size: Some(0.01),
        volume_24h: Some(10_000.0),
        liquidity: Some(5_000.0),
        one_hour_change: Some(0.01),
        one_day_change: Some(-0.01),
        ofi: Some(0.1),
        yes_no_residual: Some(0.0),
        secs_to_resolution: Some(3_600.0),
        holders: holders(10, 100.0),
        makers,
        counterparty_volumes: vec![],
        onchain_fills: vec![],
        temporal_reference_ts: None,
        sequence_position: None,
        sequence_total: None,
        oracle_risk: Default::default(),
        book: Book {
            bids: vec![Level {
                price: 0.54,
                size: 100.0,
            }],
            asks: vec![Level {
                price: 0.56,
                size: 100.0,
            }],
        },
    }
}

fn holders(count: usize, amount: f64) -> Vec<HolderShare> {
    (0..count)
        .map(|idx| HolderShare {
            wallet: format!("0xholder{idx:02}"),
            amount,
            outcome_index: 0,
        })
        .collect()
}

fn select_live_book(
    gamma: &GammaClient,
    clob: &ClobClient,
) -> (GammaMarketRecord, String, ClobBookPage) {
    let page = gamma
        .fetch_markets(&GammaMarketsRequest::crypto_active(20))
        .expect("fetch active crypto Gamma markets");
    for market in page.markets {
        for token in &market.clob_token_ids {
            if let Ok(book) = clob.fetch_book(token)
                && (!book.book.bids.is_empty() || !book.book.asks.is_empty())
            {
                return (market.clone(), token.clone(), book);
            }
        }
    }
    panic!("no active crypto token produced a public CLOB book with visible levels");
}

fn object_keys(value: &Value) -> Vec<String> {
    value
        .as_object()
        .map(|map| map.keys().cloned().collect())
        .unwrap_or_default()
}

fn first_level_keys(value: &Value) -> Vec<String> {
    for side in ["bids", "asks"] {
        if let Some(keys) = value
            .get(side)
            .and_then(Value::as_array)
            .and_then(|rows| rows.first())
            .and_then(Value::as_object)
            .map(|map| map.keys().cloned().collect())
        {
            return keys;
        }
    }
    Vec::new()
}

fn assert_no_maker_identity_fields(keys: &[String]) {
    let forbidden: BTreeSet<&str> = [
        "maker",
        "maker_address",
        "makerAddress",
        "proxy_wallet",
        "proxyWallet",
        "owner",
        "user",
        "address",
    ]
    .into_iter()
    .collect();
    for key in keys {
        assert!(
            !forbidden.contains(key.as_str()),
            "public CLOB book unexpectedly exposed maker identity field {key}"
        );
    }
}
