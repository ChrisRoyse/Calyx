use serde_json::{Value, json};

use crate::raw_source_probes::Probe;

pub(crate) fn add_clob_batch_probes(probes: &mut Vec<Probe>, tokens: &[String]) {
    let requests = token_requests(tokens);
    probes.push(post_probe(
        "clob_post_books_by_tokens",
        "books",
        "https://docs.polymarket.com/api-reference/market-data/get-order-books-request-body",
        requests.clone(),
        true,
        false,
    ));
    probes.push(post_probe(
        "clob_post_prices_by_tokens",
        "prices",
        "https://docs.polymarket.com/api-reference/market-data/get-market-prices-request-body",
        price_requests(tokens),
        true,
        false,
    ));
    probes.push(post_probe(
        "clob_post_midpoints_by_tokens",
        "midpoints",
        "https://docs.polymarket.com/api-reference/market-data/get-midpoint-prices-request-body",
        requests.clone(),
        true,
        false,
    ));
    probes.push(post_probe(
        "clob_post_spreads_by_tokens",
        "spreads",
        "https://docs.polymarket.com/api-reference/market-data/get-spreads",
        requests.clone(),
        true,
        false,
    ));
    probes.push(post_probe(
        "clob_post_last_trades_by_tokens",
        "last-trades-prices",
        "https://docs.polymarket.com/api-reference/market-data/get-last-trade-prices-request-body",
        requests,
        true,
        false,
    ));
    probes.push(post_probe(
        "clob_post_batch_prices_history_by_tokens",
        "batch-prices-history",
        "https://docs.polymarket.com/api-reference/markets/get-batch-prices-history",
        json!({"markets": tokens, "interval": "1d", "fidelity": 1440}),
        true,
        false,
    ));
}

pub(crate) fn clob_batch_edge_probes(token: &str) -> Vec<Probe> {
    vec![
        post_probe(
            "edge_clob_post_books_object_payload",
            "books",
            "https://docs.polymarket.com/api-reference/market-data/get-order-books-request-body",
            json!({"token_id": token}),
            false,
            true,
        ),
        post_probe(
            "edge_clob_post_prices_object_payload",
            "prices",
            "https://docs.polymarket.com/api-reference/market-data/get-market-prices-request-body",
            json!({"token_id": token, "side": "BUY"}),
            false,
            true,
        ),
        post_probe(
            "edge_clob_post_midpoints_object_payload",
            "midpoints",
            "https://docs.polymarket.com/api-reference/market-data/get-midpoint-prices-request-body",
            json!({"token_id": token}),
            false,
            true,
        ),
        post_probe(
            "edge_clob_post_batch_history_missing_markets",
            "batch-prices-history",
            "https://docs.polymarket.com/api-reference/markets/get-batch-prices-history",
            json!({}),
            false,
            true,
        ),
    ]
}

pub(crate) fn clob_post_runtime_semantics_probes(
    token: &str,
    unique_tokens: &[String],
) -> Vec<Probe> {
    let duplicate_tokens = vec![token.to_string(); 21];
    vec![
        post_probe(
            "edge_clob_post_prices_missing_side_runtime_semantics",
            "prices",
            "https://docs.polymarket.com/api-reference/market-data/get-market-prices-request-body",
            json!([{"token_id": token}]),
            true,
            true,
        ),
        post_probe(
            "edge_clob_post_prices_invalid_side_runtime_semantics",
            "prices",
            "https://docs.polymarket.com/api-reference/market-data/get-market-prices-request-body",
            json!([{"token_id": token, "side": "HOLD"}]),
            true,
            true,
        ),
        post_probe(
            "edge_clob_post_prices_invalid_token_runtime_semantics",
            "prices",
            "https://docs.polymarket.com/api-reference/market-data/get-market-prices-request-body",
            json!([{"token_id": "not-a-real-token", "side": "BUY"}]),
            true,
            true,
        ),
        post_probe(
            "edge_clob_post_batch_history_21_duplicate_markets_runtime_semantics",
            "batch-prices-history",
            "https://docs.polymarket.com/api-reference/markets/get-batch-prices-history",
            json!({"markets": duplicate_tokens, "interval": "1d", "fidelity": 1440}),
            true,
            true,
        ),
        post_probe(
            "edge_clob_post_batch_history_21_unique_markets_runtime_semantics",
            "batch-prices-history",
            "https://docs.polymarket.com/api-reference/markets/get-batch-prices-history",
            json!({"markets": unique_tokens, "interval": "1d", "fidelity": 1440}),
            false,
            true,
        ),
    ]
}

fn post_probe(
    name: impl Into<String>,
    endpoint: impl Into<String>,
    docs_url: impl Into<String>,
    body: Value,
    expected_success: bool,
    edge_case: bool,
) -> Probe {
    let endpoint = endpoint.into();
    Probe {
        name: name.into(),
        source: "clob-post".to_string(),
        endpoint: endpoint.clone(),
        method: "POST".to_string(),
        url: format!("https://clob.polymarket.com/{endpoint}"),
        docs_url: docs_url.into(),
        request_body: Some(body),
        expected_success,
        edge_case,
        expect_json: true,
    }
}

fn token_requests(tokens: &[String]) -> Value {
    Value::Array(
        tokens
            .iter()
            .map(|token| json!({"token_id": token}))
            .collect(),
    )
}

fn price_requests(tokens: &[String]) -> Value {
    Value::Array(
        tokens
            .iter()
            .flat_map(|token| {
                [
                    json!({"token_id": token, "side": "BUY"}),
                    json!({"token_id": token, "side": "SELL"}),
                ]
            })
            .collect(),
    )
}
