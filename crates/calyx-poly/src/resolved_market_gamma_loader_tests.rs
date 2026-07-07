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
                gamma_market(
                    "0xclean",
                    &["No", "Yes"],
                    &["0.00", "1.00"],
                    &["no-token", "yes-token"],
                    "0.61",
                    "0.08",
                    "500.0",
                    "2000.0",
                    "2026-01-01T00:00:00Z",
                    "2026-01-02T00:00:00Z",
                )
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
            gamma_market(
                "0xdegenerate",
                &["No", "Yes"],
                &["0.00", "1.00"],
                &["no-token", "yes-token"],
                "1.00",
                "1.00",
                "0",
                "0",
                "2026-01-01T00:00:00Z",
                "2026-01-02T00:00:00Z",
            ),
            gamma_market(
                "0xlookahead",
                &["No", "Yes"],
                &["0.00", "1.00"],
                &["no-token", "yes-token"],
                "0.61",
                "0.08",
                "500.0",
                "2000.0",
                "2026-01-03T00:00:00Z",
                "2026-01-02T00:00:00Z",
            ),
            gamma_market(
                "0xunresolved",
                &["No", "Yes"],
                &["0.40", "0.60"],
                &["no-token", "yes-token"],
                "0.61",
                "0.08",
                "500.0",
                "2000.0",
                "2026-01-01T00:00:00Z",
                "2026-01-02T00:00:00Z",
            )
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

    let _ = fs::remove_dir_all(root);
}

fn gamma_market(
    condition_id: &str,
    outcomes: &[&str],
    outcome_prices: &[&str],
    tokens: &[&str],
    last_trade_price: &str,
    spread: &str,
    volume_24h: &str,
    liquidity: &str,
    created_at: &str,
    closed_time: &str,
) -> Value {
    json!({
        "conditionId": condition_id,
        "slug": format!("known-truth-{condition_id}"),
        "category": "Crypto",
        "outcomes": serde_json::to_string(&outcomes).expect("outcomes"),
        "outcomePrices": serde_json::to_string(&outcome_prices).expect("prices"),
        "clobTokenIds": serde_json::to_string(&tokens).expect("tokens"),
        "spread": spread,
        "volume24hr": volume_24h,
        "liquidityNum": liquidity,
        "bestBid": "0.57",
        "bestAsk": "0.65",
        "lastTradePrice": last_trade_price,
        "createdAt": created_at,
        "closedTime": closed_time,
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
