use serde_json::{Value, json};

use crate::raw_large_corpus_onchain_specs::{
    CONTRACTS_DOCS_URL, CTF_EXCHANGE_V2, GOLDSKY_ACTIVITY_URL, GOLDSKY_DOCS_URL,
    GOLDSKY_ORDERBOOK_URL, NEG_RISK_EXCHANGE_V2, POLYGON_DRPC_URL, V2_ORDER_FILLED_TOPIC,
};

pub(crate) fn polygon_block_number_plan() -> PostPlan {
    polygon_plan(
        "polygon_rpc_block_number_large",
        "block-number",
        json_rpc("eth_blockNumber", json!([])),
        "sampled_json_rpc",
    )
}

pub(crate) fn onchain_plans(latest_block: u64) -> Vec<PostPlan> {
    vec![
        polygon_logs_plan(
            "polygon_rpc_ctf_exchange_v2_order_filled_logs_large",
            "ctf-exchange-v2-order-filled-logs",
            CTF_EXCHANGE_V2,
            latest_block,
            latest_block,
        ),
        polygon_logs_plan(
            "polygon_rpc_neg_risk_exchange_v2_order_filled_logs_large",
            "neg-risk-exchange-v2-order-filled-logs",
            NEG_RISK_EXCHANGE_V2,
            latest_block,
            latest_block,
        ),
        goldsky_plan(
            "goldsky_orderbook_order_filled_events_large",
            "orderbook-order-filled-events",
            GOLDSKY_ORDERBOOK_URL,
            json!({
                "query": "{ orderFilledEvents(first: 100, orderBy: timestamp, orderDirection: desc) { id timestamp transactionHash maker taker makerAssetId takerAssetId makerAmountFilled takerAmountFilled fee } }"
            }),
            "sampled_graphql_events",
        ),
        goldsky_plan(
            "goldsky_activity_redemptions_large",
            "activity-redemptions",
            GOLDSKY_ACTIVITY_URL,
            json!({
                "query": "{ redemptions(first: 100, orderBy: timestamp, orderDirection: desc) { id timestamp redeemer condition payout } }"
            }),
            "sampled_graphql_redemptions",
        ),
    ]
}

pub(crate) fn onchain_edge_plans() -> Vec<PostPlan> {
    vec![
        polygon_plan(
            "edge_polygon_rpc_invalid_method_large",
            "invalid-json-rpc-method",
            json_rpc("poly_fsv_invalidMethod", json!([])),
            "expected_json_rpc_error",
        ),
        goldsky_plan(
            "edge_goldsky_orderbook_malformed_query_large",
            "malformed-graphql-query",
            GOLDSKY_ORDERBOOK_URL,
            json!({ "query": "{ orderFilledEvents(first: 1) { id " }),
            "expected_graphql_error",
        ),
        polygon_plan(
            "edge_polygon_rpc_invalid_logs_address_large",
            "invalid-logs-address",
            json_rpc(
                "eth_getLogs",
                json!([{
                    "fromBlock": "0x0",
                    "toBlock": "0x0",
                    "address": "not-an-address",
                    "topics": [V2_ORDER_FILLED_TOPIC]
                }]),
            ),
            "expected_json_rpc_error",
        ),
    ]
}

fn polygon_logs_plan(
    dataset: &str,
    endpoint: &str,
    address: &str,
    from_block: u64,
    to_block: u64,
) -> PostPlan {
    polygon_plan(
        dataset,
        endpoint,
        json_rpc(
            "eth_getLogs",
            json!([{
                "fromBlock": hex_block(from_block),
                "toBlock": hex_block(to_block),
                "address": address,
                "topics": [V2_ORDER_FILLED_TOPIC]
            }]),
        ),
        "sampled_json_rpc_single_block_logs",
    )
}

fn polygon_plan(
    dataset: &str,
    endpoint: &str,
    request_body: Value,
    expected_semantics: &str,
) -> PostPlan {
    post_plan(
        dataset,
        "polygon-rpc",
        endpoint,
        POLYGON_DRPC_URL,
        CONTRACTS_DOCS_URL,
        request_body,
        expected_semantics,
    )
}

fn goldsky_plan(
    dataset: &str,
    endpoint: &str,
    url: &str,
    request_body: Value,
    expected_semantics: &str,
) -> PostPlan {
    post_plan(
        dataset,
        "goldsky-subgraph",
        endpoint,
        url,
        GOLDSKY_DOCS_URL,
        request_body,
        expected_semantics,
    )
}

fn post_plan(
    dataset: &str,
    source: &str,
    endpoint: &str,
    url: &str,
    docs_url: &str,
    request_body: Value,
    expected_semantics: &str,
) -> PostPlan {
    PostPlan {
        dataset: dataset.to_string(),
        source: source.to_string(),
        endpoint: endpoint.to_string(),
        url: url.to_string(),
        docs_url: docs_url.to_string(),
        request_body,
        expected_semantics: expected_semantics.to_string(),
    }
}

fn json_rpc(method: &str, params: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params
    })
}

fn hex_block(block: u64) -> String {
    format!("0x{block:x}")
}

#[derive(Debug, Clone)]
pub(crate) struct PostPlan {
    pub(crate) dataset: String,
    pub(crate) source: String,
    pub(crate) endpoint: String,
    pub(crate) url: String,
    pub(crate) docs_url: String,
    pub(crate) request_body: Value,
    pub(crate) expected_semantics: String,
}
