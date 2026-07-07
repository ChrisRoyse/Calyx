use serde_json::{Value, json};

use crate::raw_http::capture_http_probe;
use crate::raw_source_probes::Probe;
use crate::raw_sources::{RawEndpointSample, RawSourceSamplingRequest};
use crate::{PolyError, Result};

const POLYGON_DRPC_URL: &str = "https://polygon.drpc.org";
const POLYGON_PUBLIC_RPC_URL: &str = "https://polygon-rpc.com";
const GOLDSKY_ORDERBOOK_URL: &str = "https://api.goldsky.com/api/public/project_cl6mb8i9h0003e201j6li0diw/subgraphs/orderbook-subgraph/prod/gn";
const GOLDSKY_ACTIVITY_URL: &str = "https://api.goldsky.com/api/public/project_cl6mb8i9h0003e201j6li0diw/subgraphs/activity-subgraph/0.0.4/gn";
const GOLDSKY_ACTIVITY_PROD_URL: &str = "https://api.goldsky.com/api/public/project_cl6mb8i9h0003e201j6li0diw/subgraphs/activity-subgraph/prod/gn";
const CONTRACTS_DOCS_URL: &str = "https://docs.polymarket.com/resources/contracts";
const GOLDSKY_POLYMARKET_DOCS_URL: &str = "https://docs.goldsky.com/chains/polymarket";
const V2_ORDER_FILLED_TOPIC: &str =
    "0xd543adfd945773f1a62f74f0ee55a5e3b9b1a28262980ba90b1a89f2ea84d8ee";
const CTF_EXCHANGE_V2: &str = "0xE111180000d2663C0091e4f400237545B87B996B";
const NEG_RISK_EXCHANGE_V2: &str = "0xe2222d279d744050d28e00520010520000310F59";

pub(crate) fn capture_onchain_samples(
    request: &RawSourceSamplingRequest,
    agent: &ureq::Agent,
) -> Result<Vec<RawEndpointSample>> {
    let mut samples = Vec::new();
    let block_probe = polygon_block_number_probe();
    let (block_sample, parsed_block) = capture_http_probe(request, agent, &block_probe)?;
    let latest_block = latest_block_number(&parsed_block, &block_sample.name)?;
    samples.push(block_sample);
    samples.push(
        capture_http_probe(
            request,
            agent,
            &polygon_v2_order_filled_logs_probe(latest_block),
        )?
        .0,
    );
    samples.push(
        capture_http_probe(
            request,
            agent,
            &polygon_empty_zero_address_logs_probe(latest_block),
        )?
        .0,
    );
    for probe in goldsky_subgraph_probes() {
        samples.push(capture_http_probe(request, agent, &probe)?.0);
    }
    for probe in onchain_edge_probes() {
        samples.push(capture_http_probe(request, agent, &probe)?.0);
    }
    Ok(samples)
}

fn latest_block_number(parsed: &serde_json::Result<Value>, sample_name: &str) -> Result<u64> {
    let value = parsed.as_ref().map_err(|err| {
        PolyError::raw_source(
            "POLY_RAW_SOURCE_ONCHAIN_BLOCK_PARSE_FAILED",
            format!("parse block-number response for {sample_name}: {err}"),
        )
    })?;
    let result = value.get("result").and_then(Value::as_str).ok_or_else(|| {
        PolyError::raw_source(
            "POLY_RAW_SOURCE_ONCHAIN_BLOCK_RESULT_MISSING",
            format!("block-number response for {sample_name} missing string result"),
        )
    })?;
    u64::from_str_radix(result.trim_start_matches("0x"), 16).map_err(|err| {
        PolyError::raw_source(
            "POLY_RAW_SOURCE_ONCHAIN_BLOCK_RESULT_INVALID",
            format!("block-number result {result} is not hex: {err}"),
        )
    })
}

fn polygon_block_number_probe() -> Probe {
    polygon_probe(
        "polygon_rpc_block_number",
        "block-number",
        POLYGON_DRPC_URL,
        json_rpc("eth_blockNumber", json!([])),
        true,
        false,
    )
}

fn polygon_v2_order_filled_logs_probe(latest_block: u64) -> Probe {
    let from_block = latest_block.saturating_sub(2);
    polygon_probe(
        "polygon_rpc_v2_order_filled_logs",
        "v2-order-filled-logs",
        POLYGON_DRPC_URL,
        json_rpc(
            "eth_getLogs",
            json!([{
                "fromBlock": hex_block(from_block),
                "toBlock": hex_block(latest_block),
                "address": [CTF_EXCHANGE_V2, NEG_RISK_EXCHANGE_V2],
                "topics": [V2_ORDER_FILLED_TOPIC]
            }]),
        ),
        true,
        false,
    )
}

fn polygon_empty_zero_address_logs_probe(latest_block: u64) -> Probe {
    polygon_probe(
        "edge_polygon_rpc_zero_address_empty_logs",
        "zero-address-empty-logs",
        POLYGON_DRPC_URL,
        json_rpc(
            "eth_getLogs",
            json!([{
                "fromBlock": hex_block(latest_block),
                "toBlock": hex_block(latest_block),
                "address": "0x0000000000000000000000000000000000000000",
                "topics": [V2_ORDER_FILLED_TOPIC]
            }]),
        ),
        true,
        true,
    )
}

fn goldsky_subgraph_probes() -> Vec<Probe> {
    vec![
        goldsky_probe(
            "goldsky_orderbook_order_filled_events",
            "orderbook-order-filled-events",
            GOLDSKY_ORDERBOOK_URL,
            json!({
                "query": "{ orderFilledEvents(first: 5, orderBy: timestamp, orderDirection: desc) { id timestamp transactionHash maker taker makerAssetId takerAssetId makerAmountFilled takerAmountFilled fee } }"
            }),
            true,
            false,
        ),
        goldsky_probe(
            "goldsky_activity_redemptions",
            "activity-redemptions",
            GOLDSKY_ACTIVITY_URL,
            json!({
                "query": "{ redemptions(first: 5, orderBy: timestamp, orderDirection: desc) { id timestamp redeemer condition payout } }"
            }),
            true,
            false,
        ),
    ]
}

fn onchain_edge_probes() -> Vec<Probe> {
    vec![
        polygon_probe(
            "edge_polygon_rpc_public_gateway_unauthorized",
            "unauthorized-public-gateway",
            POLYGON_PUBLIC_RPC_URL,
            json_rpc("eth_blockNumber", json!([])),
            false,
            true,
        ),
        goldsky_probe(
            "edge_goldsky_activity_prod_missing",
            "missing-activity-prod-subgraph",
            GOLDSKY_ACTIVITY_PROD_URL,
            json!({ "query": "{ redemptions(first: 1) { id } }" }),
            false,
            true,
        ),
    ]
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

fn polygon_probe(
    name: impl Into<String>,
    endpoint: impl Into<String>,
    url: impl Into<String>,
    request_body: Value,
    expected_success: bool,
    edge_case: bool,
) -> Probe {
    Probe {
        name: name.into(),
        source: "polygon-rpc".to_string(),
        endpoint: endpoint.into(),
        method: "POST".to_string(),
        url: url.into(),
        docs_url: CONTRACTS_DOCS_URL.to_string(),
        request_body: Some(request_body),
        expected_success,
        edge_case,
        expect_json: true,
    }
}

fn goldsky_probe(
    name: impl Into<String>,
    endpoint: impl Into<String>,
    url: impl Into<String>,
    request_body: Value,
    expected_success: bool,
    edge_case: bool,
) -> Probe {
    Probe {
        name: name.into(),
        source: "goldsky-subgraph".to_string(),
        endpoint: endpoint.into(),
        method: "POST".to_string(),
        url: url.into(),
        docs_url: GOLDSKY_POLYMARKET_DOCS_URL.to_string(),
        request_body: Some(request_body),
        expected_success,
        edge_case,
        expect_json: true,
    }
}
