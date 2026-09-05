use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use super::*;
use crate::resolved_market_corpus::build_resolved_market_corpus;

#[test]
fn issue223_loader_admits_known_truth_pre_resolution_gamma_row() {
    let root = test_root("admit");
    let input = root.join("gamma_markets_closed_large");
    fs::create_dir_all(&input).expect("input dir");
    fs::write(
        input.join("page-000000.json"),
        serde_json::to_vec_pretty(&json!({
            "data": [
                gamma_market(GammaMarketFixture {
                    condition_id: "0xclean",
                    outcomes: &["No", "Yes"],
                    outcome_prices: &["0.00", "1.00"],
                    tokens: &["no-token", "yes-token"],
                    last_trade_price: "0.61",
                    spread: "0.08",
                    volume_24h: "500.0",
                    liquidity: "2000.0",
                    created_at: "2026-01-01T00:00:00Z",
                    closed_time: "2026-01-02T00:00:00Z",
                })
            ]
        }))
        .expect("encode page"),
    )
    .expect("write page");
    fs::write(input.join("page-000000.metadata.json"), "{}").expect("write ignored metadata");

    let loaded = load_admissible_markets(&input).expect("load");
    assert_eq!(
        loaded.census.files_read, 1,
        "metadata sidecar is not an input page"
    );
    assert_eq!(loaded.census.markets_seen, 1);
    assert_eq!(loaded.census.admitted, 1);
    assert_eq!(loaded.census.rejected_terminal_degenerate, 0);
    assert_eq!(loaded.census.rejected_lookahead, 0);
    assert_eq!(loaded.census.unresolved_no_clean_winner, 0);
    assert_eq!(loaded.census.malformed_page_objects, 0);

    let (snapshot, resolution) = &loaded.markets[0];
    assert_eq!(snapshot.condition_id, "0xclean");
    assert_eq!(snapshot.token_id, "no-token");
    assert_eq!(snapshot.price, Some(0.61));
    assert_eq!(snapshot.spread, Some(0.08));
    assert_eq!(snapshot.volume_24h, Some(500.0));
    assert_eq!(snapshot.liquidity, Some(2000.0));
    assert_eq!(snapshot.secs_to_resolution, Some(86_400.0));
    assert_eq!(resolution.condition_id, "0xclean");
    assert_eq!(resolution.winning_outcome_index, 1);
    assert_eq!(resolution.winning_label, "Yes");

    let inputs = loaded.inputs();
    let corpus =
        build_resolved_market_corpus(&inputs, 1, b"issue223-loader-test", 0.99).expect("corpus");
    assert_eq!(corpus.recall_queries.len(), 1);
    assert_eq!(corpus.exemplars.len(), 1);
    assert!(!corpus.exemplars[0].outcome_yes);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn issue223_loader_rejects_degenerate_lookahead_and_unresolved_rows_loud() {
    let root = test_root("reject");
    let input = root.join("gamma_markets_closed_large");
    fs::create_dir_all(&input).expect("input dir");
    fs::write(
        input.join("page-000000.json"),
        serde_json::to_vec_pretty(&json!([
            gamma_market(GammaMarketFixture {
                condition_id: "0xdegenerate",
                outcomes: &["No", "Yes"],
                outcome_prices: &["0.00", "1.00"],
                tokens: &["no-token", "yes-token"],
                last_trade_price: "1.00",
                spread: "1.00",
                volume_24h: "0",
                liquidity: "0",
                created_at: "2026-01-01T00:00:00Z",
                closed_time: "2026-01-02T00:00:00Z",
            }),
            gamma_market(GammaMarketFixture {
                condition_id: "0xlookahead",
                outcomes: &["No", "Yes"],
                outcome_prices: &["0.00", "1.00"],
                tokens: &["no-token", "yes-token"],
                last_trade_price: "0.61",
                spread: "0.08",
                volume_24h: "500.0",
                liquidity: "2000.0",
                created_at: "2026-01-03T00:00:00Z",
                closed_time: "2026-01-02T00:00:00Z",
            }),
            gamma_market(GammaMarketFixture {
                condition_id: "0xunresolved",
                outcomes: &["No", "Yes"],
                outcome_prices: &["0.40", "0.60"],
                tokens: &["no-token", "yes-token"],
                last_trade_price: "0.61",
                spread: "0.08",
                volume_24h: "500.0",
                liquidity: "2000.0",
                created_at: "2026-01-01T00:00:00Z",
                closed_time: "2026-01-02T00:00:00Z",
            })
        ]))
        .expect("encode page"),
    )
    .expect("write page");

    let loaded = load_admissible_markets(&input).expect("load");
    assert!(loaded.markets.is_empty());
    assert_eq!(loaded.census.files_read, 1);
    assert_eq!(loaded.census.markets_seen, 3);
    assert_eq!(loaded.census.admitted, 0);
    assert_eq!(loaded.census.rejected_terminal_degenerate, 1);
    assert_eq!(loaded.census.rejected_lookahead, 1);
    assert_eq!(loaded.census.unresolved_no_clean_winner, 1);
    assert_eq!(loaded.census.skipped_not_binary_or_ids, 0);
    assert_eq!(loaded.census.malformed_page_objects, 0);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn issue1321_loader_uses_shared_gamma_timestamp_parser() {
    let root = test_root("shared-time");
    let input = root.join("gamma_markets_closed_large");
    fs::create_dir_all(&input).expect("input dir");
    let mut date_only_end = gamma_market(GammaMarketFixture {
        condition_id: "0xdateonly",
        outcomes: &["No", "Yes"],
        outcome_prices: &["0.00", "1.00"],
        tokens: &["no-token", "yes-token"],
        last_trade_price: "0.61",
        spread: "0.08",
        volume_24h: "500.0",
        liquidity: "2000.0",
        created_at: "2026-01-01T00:00:00Z",
        closed_time: "2026-01-02T00:00:00Z",
    });
    let object = date_only_end.as_object_mut().expect("market object");
    object.remove("closedTime");
    object.insert("endDate".to_string(), json!("2026-01-02"));
    fs::write(
        input.join("page-000000.json"),
        serde_json::to_vec_pretty(&json!({
            "data": [
                date_only_end,
                gamma_market(GammaMarketFixture {
                    condition_id: "0xbadtime",
                    outcomes: &["No", "Yes"],
                    outcome_prices: &["0.00", "1.00"],
                    tokens: &["no-token", "yes-token"],
                    last_trade_price: "0.61",
                    spread: "0.08",
                    volume_24h: "500.0",
                    liquidity: "2000.0",
                    created_at: "2026-01-01T00:00:00Z",
                    closed_time: "2026-01-02T99:99:99Z",
                })
            ]
        }))
        .expect("encode page"),
    )
    .expect("write page");

    let loaded = load_admissible_markets(&input).expect("load");
    assert_eq!(loaded.census.markets_seen, 2);
    assert_eq!(loaded.census.admitted, 1);
    assert_eq!(loaded.census.rejected_lookahead, 1);
    assert_eq!(loaded.markets[0].0.condition_id, "0xdateonly");
    assert_eq!(loaded.markets[0].0.secs_to_resolution, Some(86_400.0));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn issue1321_loader_counts_object_pages_without_market_arrays() {
    let root = test_root("malformed-page");
    let input = root.join("gamma_markets_closed_large");
    fs::create_dir_all(&input).expect("input dir");
    fs::write(
        input.join("page-000000.json"),
        serde_json::to_vec_pretty(&json!({"cursor": "next-page"})).expect("encode page"),
    )
    .expect("write page");

    let loaded = load_admissible_markets(&input).expect("load");
    assert_eq!(loaded.census.files_read, 1);
    assert_eq!(loaded.census.markets_seen, 0);
    assert_eq!(loaded.census.malformed_page_objects, 1);
    assert_eq!(loaded.census.admitted, 0);

    let _ = fs::remove_dir_all(root);
}

struct GammaMarketFixture<'a> {
    condition_id: &'a str,
    outcomes: &'a [&'a str],
    outcome_prices: &'a [&'a str],
    tokens: &'a [&'a str],
    last_trade_price: &'a str,
    spread: &'a str,
    volume_24h: &'a str,
    liquidity: &'a str,
    created_at: &'a str,
    closed_time: &'a str,
}

fn gamma_market(fixture: GammaMarketFixture<'_>) -> Value {
    json!({
        "conditionId": fixture.condition_id,
        "slug": format!("known-truth-{}", fixture.condition_id),
        "category": "Crypto",
        "outcomes": serde_json::to_string(&fixture.outcomes).expect("outcomes"),
        "outcomePrices": serde_json::to_string(&fixture.outcome_prices).expect("prices"),
        "clobTokenIds": serde_json::to_string(&fixture.tokens).expect("tokens"),
        "spread": fixture.spread,
        "volume24hr": fixture.volume_24h,
        "liquidityNum": fixture.liquidity,
        "bestBid": "0.57",
        "bestAsk": "0.65",
        "lastTradePrice": fixture.last_trade_price,
        "createdAt": fixture.created_at,
        "closedTime": fixture.closed_time,
    })
}

fn test_root(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "calyx-poly-issue223-gamma-loader-{name}-{}-{nanos}",
        std::process::id()
    ))
}
