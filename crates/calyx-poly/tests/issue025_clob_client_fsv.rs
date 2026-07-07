//! Issue #25 - CLOB API typed client FSV.
//!
//! Source of truth for live FSV: persisted public CLOB HTTP body bytes read back from disk and
//! parsed into normalized book, scalar, and history records for one Gamma-derived token.

#[path = "fsv_support.rs"]
mod support;

use std::fs;
use std::path::Path;

use calyx_poly::{
    ClobBookPage, ClobBookStatus, ClobClient, ClobClientConfig, ClobJsonPage,
    ClobPriceBatchRequest, ClobScalarKind, ClobSide, ERR_CLOB_BOOK_CROSSED, ERR_CLOB_BOOK_INVALID,
    ERR_CLOB_HTTP, GammaClient, GammaClientConfig, GammaMarketRecord, GammaMarketsRequest,
    parse_clob_batch_history_value, parse_clob_books_value, parse_clob_history_value,
    parse_clob_last_trades_value, parse_clob_order_book, parse_clob_price_map_value,
    parse_clob_scalar_map_value, parse_clob_scalar_value,
};
use serde_json::{Value, json};

use support::{collect_files, named_fsv_root, reset_dir, write_blake3sums, write_json};

#[test]
fn issue025_clob_parser_known_truth_edges() {
    let book = parse_clob_order_book(&happy_book()).expect("happy book parses");
    assert_eq!(book.status, ClobBookStatus::Ready);
    assert_eq!(book.best_bid, Some(0.42));
    assert_eq!(book.best_ask, Some(0.44));
    assert_eq!(book.midpoint, Some(0.43));
    assert_eq!(book.spread, Some(0.02));
    assert_eq!(book.bids[0].price, 0.42, "bids are normalized best first");
    assert_eq!(book.asks[0].price, 0.44, "asks are normalized best first");
    assert_eq!(book.to_market_book().bids[0].price, 0.42);

    let thin = parse_clob_order_book(&json!({
        "market": "0xthin",
        "asset_id": "tok_thin",
        "timestamp": "1783407000000",
        "bids": [],
        "asks": [{"price": "0.52", "size": "10"}]
    }))
    .expect("thin one-sided book is absent/degraded, not fabricated");
    assert_eq!(thin.status, ClobBookStatus::ThinOrEmpty);
    assert_eq!(thin.best_bid, None);
    assert_eq!(thin.midpoint, None);

    let crossed = parse_clob_order_book(&json!({
        "market": "0xcrossed",
        "asset_id": "tok_crossed",
        "timestamp": "1783407000000",
        "bids": [{"price": "0.50", "size": "10"}],
        "asks": [{"price": "0.49", "size": "10"}]
    }))
    .expect_err("crossed/locked book fails closed");
    assert_eq!(crossed.code(), ERR_CLOB_BOOK_CROSSED);

    let invalid = parse_clob_order_book(&json!({
        "market": "0xbad",
        "asset_id": "tok_bad",
        "timestamp": "1783407000000",
        "bids": [{"price": "1.20", "size": "10"}],
        "asks": [{"price": "0.80", "size": "10"}]
    }))
    .expect_err("invalid level fails closed");
    assert_eq!(invalid.code(), ERR_CLOB_BOOK_INVALID);

    let empty_prices = parse_clob_price_map_value(&json!({})).expect("empty price map parses");
    assert!(empty_prices.is_empty());
    let missing_side = parse_clob_price_map_value(&json!({"tok": {"SELL": "0.44"}}))
        .expect("missing-side live SELL default parses");
    assert_eq!(missing_side[0].buy, None);
    assert_eq!(missing_side[0].sell, Some(0.44));
}

#[test]
#[ignore = "requires live public Gamma and CLOB APIs"]
fn issue025_clob_client_live_fsv() {
    let (root, _keep) = named_fsv_root("POLY_ISSUE025_FSV_ROOT", "issue025-clob-client");
    reset_dir(&root);
    let gamma = GammaClient::new(GammaClientConfig::default()).expect("Gamma client");
    let clob = ClobClient::new(ClobClientConfig::default()).expect("CLOB client");
    let (market, token, book) = select_live_token(&gamma, &clob);
    let token_ids = vec![token.clone()];

    let buy = clob
        .fetch_price(&token, ClobSide::Buy)
        .expect("fetch BUY price");
    let sell = clob
        .fetch_price(&token, ClobSide::Sell)
        .expect("fetch SELL price");
    let midpoint = clob.fetch_midpoint(&token).expect("fetch midpoint");
    let spread = clob.fetch_spread(&token).expect("fetch spread");
    let tick_size = clob.fetch_tick_size(&token).expect("fetch tick size");
    let history = clob
        .fetch_prices_history(&token, "1d", 1440)
        .expect("fetch prices-history");
    assert!(!history.history.points.is_empty());

    let post_books = clob.post_books(&token_ids).expect("POST /books");
    let post_prices = clob
        .post_prices(&[
            ClobPriceBatchRequest::side(&token, ClobSide::Buy),
            ClobPriceBatchRequest::side(&token, ClobSide::Sell),
        ])
        .expect("POST /prices");
    let post_missing_side = clob
        .post_prices(&[ClobPriceBatchRequest::missing_side(&token)])
        .expect("POST /prices missing side runtime semantics");
    let post_invalid_side = clob
        .post_prices(&[ClobPriceBatchRequest::raw_side(&token, "HOLD")])
        .expect("POST /prices invalid side runtime semantics");
    let post_invalid_token = clob
        .post_prices(&[ClobPriceBatchRequest::side(
            "not-a-real-token",
            ClobSide::Buy,
        )])
        .expect("POST /prices invalid token runtime semantics");
    let post_midpoints = clob.post_midpoints(&token_ids).expect("POST /midpoints");
    let post_spreads = clob.post_spreads(&token_ids).expect("POST /spreads");
    let post_last_trades = clob
        .post_last_trades(&token_ids)
        .expect("POST /last-trades-prices");
    let post_history = clob
        .post_batch_prices_history(&token_ids, "1d", 1440)
        .expect("POST /batch-prices-history");
    let duplicate_tokens = vec![token.clone(); 21];
    let post_duplicate_history = clob
        .post_batch_prices_history(&duplicate_tokens, "1d", 1440)
        .expect("21 duplicate markets are a live success semantics");
    let invalid_book = clob
        .fetch_book("not-a-real-token")
        .expect_err("invalid token GET /book fails closed");
    assert_eq!(invalid_book.code(), ERR_CLOB_HTTP);
    assert!(
        post_missing_side
            .prices
            .iter()
            .any(|row| row.token_id == token && row.sell.is_some() && row.buy.is_none())
    );
    assert!(post_invalid_side.prices.is_empty());
    assert!(post_invalid_token.prices.is_empty());
    assert!(!post_history.histories[0].points.is_empty());
    assert!(!post_duplicate_history.histories[0].points.is_empty());

    let persisted = json!({
        "book": persist_book(&root, "get-book", &book),
        "price_buy": persist_scalar(&root, "get-price-buy", &buy.http, &token, ClobScalarKind::BuyPrice, "price"),
        "price_sell": persist_scalar(&root, "get-price-sell", &sell.http, &token, ClobScalarKind::SellPrice, "price"),
        "midpoint": persist_scalar(&root, "get-midpoint", &midpoint.http, &token, ClobScalarKind::Midpoint, "mid"),
        "spread": persist_scalar(&root, "get-spread", &spread.http, &token, ClobScalarKind::Spread, "spread"),
        "tick_size": persist_scalar(&root, "get-tick-size", &tick_size.http, &token, ClobScalarKind::TickSize, "minimum_tick_size"),
        "history": persist_history(&root, "get-prices-history", &history.http, &token),
        "post_books": persist_books(&root, "post-books", &post_books.http),
        "post_prices": persist_price_map(&root, "post-prices", &post_prices.http),
        "post_prices_missing_side": persist_price_map(&root, "post-prices-missing-side", &post_missing_side.http),
        "post_prices_invalid_side": persist_price_map(&root, "post-prices-invalid-side", &post_invalid_side.http),
        "post_prices_invalid_token": persist_price_map(&root, "post-prices-invalid-token", &post_invalid_token.http),
        "post_midpoints": persist_scalar_map(&root, "post-midpoints", &post_midpoints.http, ClobScalarKind::Midpoint),
        "post_spreads": persist_scalar_map(&root, "post-spreads", &post_spreads.http, ClobScalarKind::Spread),
        "post_last_trades": persist_last_trades(&root, "post-last-trades-prices", &post_last_trades.http),
        "post_history": persist_batch_history(&root, "post-batch-prices-history", &post_history.http),
        "post_duplicate_history": persist_batch_history(&root, "post-batch-prices-history-21-duplicates", &post_duplicate_history.http)
    });
    let synthetic_edges = json!({
        "thin_or_empty": {"status": format!("{:?}", ClobBookStatus::ThinOrEmpty)},
        "crossed_or_locked": {"code": ERR_CLOB_BOOK_CROSSED},
        "invalid_level": {"code": ERR_CLOB_BOOK_INVALID},
        "invalid_get_book_token": {"code": invalid_book.code()}
    });

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let report = json!({
        "issue": 25,
        "proof_claim": "The read-only CLOB client fetches /book, /price BUY/SELL, /midpoint, /spread, /tick-size, /prices-history, and POST batch variants for one Gamma-derived live token, persists exact raw response bytes, reads them back, and parses them into normalized fail-closed book/scalar/history records.",
        "selected_market": {
            "market_id": market.market_id,
            "condition_id": market.condition_id,
            "event_id": market.event_id,
            "token_id": token,
            "outcome_count": market.clob_token_ids.len()
        },
        "minimum_sufficient_proof_corpus": {
            "live_gamma_derived_tokens": 1,
            "live_get_endpoints": 7,
            "live_post_endpoint_cases": 10,
            "synthetic_parser_edge_cases": 4,
            "why_this_is_sufficient": "One live Gamma-derived token exercises every #25 CLOB HTTP path, join key, raw-body readback, and normalized parser surface. The synthetic parser edges prove the states that cannot be reliably forced from the live API: empty/thin book, crossed/locked book, invalid level, and invalid GET token fail-closed behavior.",
            "why_smaller_is_insufficient": "Dropping the live token would not prove actual CLOB wire shapes. Dropping synthetic edges would not prove fail-closed and Absent semantics. Dropping POST runtime cases would miss the #178 live semantics where invalid side/token returns HTTP 200 empty objects.",
            "why_larger_is_wasteful": "More tokens would repeat the same endpoint, readback, scalar-map, and book-normalization paths without adding a #25 invariant; 100,000 or 1,000,000 rows would be a corpus/backfill test, not a client correctness proof."
        },
        "source_of_truth": "live public CLOB HTTP response bodies persisted under this FSV root and parsed only after disk readback",
        "persisted_readbacks": persisted,
        "edge_cases": synthetic_edges,
        "physical_files": files,
        "passed": true
    });
    let report_path = root.join("issue025_clob_client_fsv_report.json");
    write_json(&report_path, &report);
    let readback: Value =
        serde_json::from_slice(&fs::read(&report_path).expect("read report")).expect("decode");
    assert_eq!(readback, report);
    write_blake3sums(&root);
    println!("ISSUE025_CLOB_CLIENT_FSV={}", report_path.display());
}

fn select_live_token(
    gamma: &GammaClient,
    clob: &ClobClient,
) -> (GammaMarketRecord, String, ClobBookPage) {
    let page = gamma
        .fetch_markets(&GammaMarketsRequest::crypto_active(5))
        .expect("fetch active crypto Gamma markets");
    for market in page.markets {
        for token in &market.clob_token_ids {
            if let Ok(book) = clob.fetch_book(token) {
                return (market.clone(), token.clone(), book);
            }
        }
    }
    panic!("no active crypto Gamma token produced a readable CLOB book");
}

fn persist_book(root: &Path, name: &str, page: &ClobBookPage) -> Value {
    persist_case(root, name, &page.http, |value| {
        Ok(serde_json::to_value(parse_clob_order_book(value)?).expect("book JSON"))
    })
}

fn persist_books(root: &Path, name: &str, page: &ClobJsonPage) -> Value {
    persist_case(root, name, page, |value| {
        Ok(serde_json::to_value(parse_clob_books_value(value)?).expect("books JSON"))
    })
}

fn persist_scalar(
    root: &Path,
    name: &str,
    page: &ClobJsonPage,
    token: &str,
    kind: ClobScalarKind,
    field: &str,
) -> Value {
    persist_case(root, name, page, |value| {
        Ok(
            serde_json::to_value(parse_clob_scalar_value(token, kind, field, value)?)
                .expect("scalar JSON"),
        )
    })
}

fn persist_scalar_map(root: &Path, name: &str, page: &ClobJsonPage, kind: ClobScalarKind) -> Value {
    persist_case(root, name, page, |value| {
        Ok(serde_json::to_value(parse_clob_scalar_map_value(kind, value)?).expect("map JSON"))
    })
}

fn persist_price_map(root: &Path, name: &str, page: &ClobJsonPage) -> Value {
    persist_case(root, name, page, |value| {
        Ok(serde_json::to_value(parse_clob_price_map_value(value)?).expect("price map JSON"))
    })
}

fn persist_history(root: &Path, name: &str, page: &ClobJsonPage, token: &str) -> Value {
    persist_case(root, name, page, |value| {
        Ok(serde_json::to_value(parse_clob_history_value(token, value)?).expect("history JSON"))
    })
}

fn persist_batch_history(root: &Path, name: &str, page: &ClobJsonPage) -> Value {
    persist_case(root, name, page, |value| {
        Ok(serde_json::to_value(parse_clob_batch_history_value(value)?).expect("history JSON"))
    })
}

fn persist_last_trades(root: &Path, name: &str, page: &ClobJsonPage) -> Value {
    persist_case(root, name, page, |value| {
        Ok(serde_json::to_value(parse_clob_last_trades_value(value)?).expect("trades JSON"))
    })
}

fn persist_case<F>(root: &Path, name: &str, page: &ClobJsonPage, parse: F) -> Value
where
    F: FnOnce(&Value) -> calyx_poly::Result<Value>,
{
    let case_root = root.join(name);
    fs::create_dir_all(&case_root).expect("create case root");
    let body_path = case_root.join("body.json");
    let parsed_path = case_root.join("parsed.json");
    let summary_path = case_root.join("summary.json");
    fs::write(&body_path, &page.raw_body).expect("write raw body");
    let raw_readback = fs::read(&body_path).expect("read raw body");
    assert_eq!(raw_readback, page.raw_body, "raw body readback is exact");
    let value: Value = serde_json::from_slice(&raw_readback).expect("decode raw readback");
    let parsed = parse(&value).expect("parse raw readback");
    write_json(&parsed_path, &parsed);
    let parsed_readback: Value =
        serde_json::from_slice(&fs::read(&parsed_path).expect("read parsed")).expect("decode");
    assert_eq!(parsed_readback, parsed);
    let summary = json!({
        "method": page.method,
        "url": page.url,
        "status_code": page.status_code,
        "body_path": body_path.display().to_string(),
        "body_bytes": page.body_bytes,
        "body_sha256": page.body_sha256,
        "parsed_path": parsed_path.display().to_string(),
        "readback_equal": true
    });
    write_json(&summary_path, &summary);
    let summary_readback: Value =
        serde_json::from_slice(&fs::read(&summary_path).expect("read summary")).expect("decode");
    assert_eq!(summary_readback, summary);
    summary_readback
}

fn happy_book() -> Value {
    json!({
        "market": "0xhappy",
        "asset_id": "tok_happy",
        "timestamp": "1783407000000",
        "hash": "hash25",
        "bids": [
            {"price": "0.40", "size": "12"},
            {"price": "0.42", "size": "10"}
        ],
        "asks": [
            {"price": "0.45", "size": "8"},
            {"price": "0.44", "size": "9"}
        ],
        "min_order_size": "5",
        "tick_size": "0.01",
        "neg_risk": false,
        "last_trade_price": "0.43"
    })
}
