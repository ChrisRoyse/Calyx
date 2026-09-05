use std::collections::BTreeSet;

use serde_json::{Value, json};

use crate::raw_large_corpus_profile::CorpusRecord;
use crate::raw_source_support::string_field;

#[derive(Debug, Clone)]
pub(crate) struct ClobTarget {
    pub(crate) condition_id: String,
    pub(crate) token_id: String,
}

#[derive(Debug, Clone)]
pub(crate) struct ClobCapturePlan {
    pub(crate) dataset: &'static str,
    pub(crate) endpoint: &'static str,
    pub(crate) docs_url: &'static str,
    pub(crate) method: &'static str,
    pub(crate) url: String,
    pub(crate) request_body: Option<Value>,
    pub(crate) stop_reason: &'static str,
}

pub(crate) fn derive_clob_targets(records: &[CorpusRecord], limit: usize) -> Vec<ClobTarget> {
    let mut seen = BTreeSet::new();
    let mut targets = Vec::new();
    for record in records {
        collect_targets(&record.value, &mut seen, &mut targets, limit);
        if targets.len() >= limit {
            break;
        }
    }
    targets
}

pub(crate) fn get_plans(target: &ClobTarget) -> Vec<ClobCapturePlan> {
    vec![
        get_plan(
            "clob_get_book_large",
            "book",
            "https://docs.polymarket.com/api-reference/market-data/get-order-book",
            format!(
                "https://clob.polymarket.com/book?token_id={}",
                target.token_id
            ),
        ),
        get_plan(
            "clob_get_price_buy_large",
            "price",
            "https://docs.polymarket.com/api-reference/market-data/get-market-price",
            format!(
                "https://clob.polymarket.com/price?token_id={}&side=BUY",
                target.token_id
            ),
        ),
        get_plan(
            "clob_get_price_sell_large",
            "price",
            "https://docs.polymarket.com/api-reference/market-data/get-market-price",
            format!(
                "https://clob.polymarket.com/price?token_id={}&side=SELL",
                target.token_id
            ),
        ),
        get_plan(
            "clob_get_midpoint_large",
            "midpoint",
            "https://docs.polymarket.com/api-reference/market-data/get-midpoint-price",
            format!(
                "https://clob.polymarket.com/midpoint?token_id={}",
                target.token_id
            ),
        ),
        get_plan(
            "clob_get_spread_large",
            "spread",
            "https://docs.polymarket.com/api-reference/market-data/get-spread",
            format!(
                "https://clob.polymarket.com/spread?token_id={}",
                target.token_id
            ),
        ),
        get_plan(
            "clob_get_last_trade_price_large",
            "last-trade-price",
            "https://docs.polymarket.com/api-reference/market-data/get-last-trade-price",
            format!(
                "https://clob.polymarket.com/last-trade-price?token_id={}",
                target.token_id
            ),
        ),
        get_plan(
            "clob_get_tick_size_large",
            "tick-size",
            "https://docs.polymarket.com/api-reference/market-data/get-tick-size",
            format!(
                "https://clob.polymarket.com/tick-size?token_id={}",
                target.token_id
            ),
        ),
        get_plan(
            "clob_get_prices_history_large",
            "prices-history",
            "https://docs.polymarket.com/api-reference/markets/get-prices-history",
            format!(
                "https://clob.polymarket.com/prices-history?market={}&interval=1d&fidelity=1440",
                target.token_id
            ),
        ),
        get_plan(
            "clob_get_market_info_large",
            "clob-markets",
            "https://docs.polymarket.com/api-reference/markets/get-clob-market-info",
            format!(
                "https://clob.polymarket.com/clob-markets/{}",
                target.condition_id
            ),
        ),
    ]
}

pub(crate) fn post_plans(targets: &[ClobTarget]) -> Vec<ClobCapturePlan> {
    let tokens = targets
        .iter()
        .map(|target| target.token_id.clone())
        .collect::<Vec<_>>();
    let token_requests = Value::Array(
        tokens
            .iter()
            .map(|token| json!({"token_id": token}))
            .collect(),
    );
    vec![
        post_plan(
            "clob_post_books_large",
            "books",
            "https://docs.polymarket.com/api-reference/market-data/get-order-books-request-body",
            token_requests.clone(),
        ),
        post_plan(
            "clob_post_prices_large",
            "prices",
            "https://docs.polymarket.com/api-reference/market-data/get-market-prices-request-body",
            price_requests(&tokens),
        ),
        post_plan(
            "clob_post_midpoints_large",
            "midpoints",
            "https://docs.polymarket.com/api-reference/market-data/get-midpoint-prices-request-body",
            token_requests.clone(),
        ),
        post_plan(
            "clob_post_spreads_large",
            "spreads",
            "https://docs.polymarket.com/api-reference/market-data/get-spreads",
            token_requests.clone(),
        ),
        post_plan(
            "clob_post_last_trades_large",
            "last-trades-prices",
            "https://docs.polymarket.com/api-reference/market-data/get-last-trade-prices-request-body",
            token_requests,
        ),
        post_plan(
            "clob_post_batch_prices_history_large",
            "batch-prices-history",
            "https://docs.polymarket.com/api-reference/markets/get-batch-prices-history",
            json!({"markets": tokens, "interval": "1d", "fidelity": 1440}),
        ),
    ]
}

pub(crate) fn clob_edge(
    name: &'static str,
    method: &'static str,
    url: String,
    request_body: Option<Value>,
    stop_reason: &'static str,
) -> ClobCapturePlan {
    ClobCapturePlan {
        dataset: name,
        endpoint: name,
        docs_url: "https://docs.polymarket.com/api-reference/introduction",
        method,
        url,
        request_body,
        stop_reason,
    }
}

fn collect_targets(
    value: &Value,
    seen: &mut BTreeSet<String>,
    targets: &mut Vec<ClobTarget>,
    limit: usize,
) {
    if targets.len() >= limit {
        return;
    }
    match value {
        Value::Object(map) => {
            let enabled = map
                .get("enableOrderBook")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            if enabled && let Some(condition_id) = string_field(value, "conditionId") {
                for token_id in clob_token_ids(value) {
                    if seen.insert(token_id.clone()) {
                        targets.push(ClobTarget {
                            condition_id: condition_id.clone(),
                            token_id,
                        });
                        if targets.len() >= limit {
                            return;
                        }
                    }
                }
            }
            for value in map.values() {
                collect_targets(value, seen, targets, limit);
                if targets.len() >= limit {
                    return;
                }
            }
        }
        Value::Array(items) => {
            for value in items {
                collect_targets(value, seen, targets, limit);
                if targets.len() >= limit {
                    return;
                }
            }
        }
        _ => {}
    }
}

fn clob_token_ids(value: &Value) -> Vec<String> {
    let Some(raw) = value.get("clobTokenIds") else {
        return Vec::new();
    };
    if let Some(items) = raw.as_array() {
        return items
            .iter()
            .filter_map(Value::as_str)
            .map(ToString::to_string)
            .collect();
    }
    raw.as_str()
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
        .and_then(|parsed| parsed.as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|item| item.as_str().map(ToString::to_string))
        .collect()
}

fn get_plan(
    dataset: &'static str,
    endpoint: &'static str,
    docs_url: &'static str,
    url: String,
) -> ClobCapturePlan {
    ClobCapturePlan {
        dataset,
        endpoint,
        docs_url,
        method: "GET",
        url,
        request_body: None,
        stop_reason: "target_token",
    }
}

fn post_plan(
    dataset: &'static str,
    endpoint: &'static str,
    docs_url: &'static str,
    body: Value,
) -> ClobCapturePlan {
    ClobCapturePlan {
        dataset,
        endpoint,
        docs_url,
        method: "POST",
        url: format!("https://clob.polymarket.com/{endpoint}"),
        request_body: Some(body),
        stop_reason: "batch_request",
    }
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
