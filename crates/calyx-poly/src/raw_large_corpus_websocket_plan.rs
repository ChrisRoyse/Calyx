use serde_json::{Value, json};

use crate::raw_large_corpus_clob_plan::ClobTarget;
use crate::{PolyError, Result};

pub(crate) const MARKET_WS_URL: &str = "wss://ws-subscriptions-clob.polymarket.com/ws/market";
pub(crate) const MARKET_WS_DOCS_URL: &str =
    "https://docs.polymarket.com/market-data/websocket/market-channel";
pub(crate) const SPORTS_WS_URL: &str = "wss://sports-api.polymarket.com/ws";
pub(crate) const SPORTS_DOCS_URL: &str = "https://docs.polymarket.com/market-data/websocket/sports";
pub(crate) const RTDS_WS_URL: &str = "wss://ws-live-data.polymarket.com";
pub(crate) const RTDS_DOCS_URL: &str = "https://docs.polymarket.com/market-data/websocket/rtds";

pub(crate) fn websocket_plans(targets: &[ClobTarget]) -> Result<Vec<WebSocketCapturePlan>> {
    let asset_ids = target_asset_ids(targets)?;
    Ok(vec![
        WebSocketCapturePlan::market(
            "websocket_market_books_large",
            json!({
                "assets_ids": asset_ids,
                "type": "market",
                "initial_dump": true,
                "level": 2,
                "custom_feature_enabled": false
            }),
            WebSocketExpectation::DataEvent,
        ),
        WebSocketCapturePlan::market(
            "websocket_market_custom_window_large",
            json!({
                "assets_ids": target_asset_ids(targets)?,
                "type": "market",
                "initial_dump": true,
                "level": 2,
                "custom_feature_enabled": true
            }),
            WebSocketExpectation::DataEvent,
        ),
        WebSocketCapturePlan::rtds(
            "websocket_rtds_crypto_prices_large",
            "crypto_prices",
            json!({
                "action": "subscribe",
                "subscriptions": [{"topic": "crypto_prices", "type": "update"}]
            }),
            WebSocketExpectation::DataEvent,
        ),
        WebSocketCapturePlan::rtds(
            "websocket_rtds_crypto_chainlink_large",
            "crypto_prices_chainlink",
            json!({
                "action": "subscribe",
                "subscriptions": [{"topic": "crypto_prices_chainlink", "type": "*", "filters": ""}]
            }),
            WebSocketExpectation::DataEvent,
        ),
        WebSocketCapturePlan::sports(
            "websocket_sports_window_large",
            WebSocketExpectation::HandshakeWindow,
        ),
        WebSocketCapturePlan::rtds(
            "websocket_rtds_comments_window_large",
            "comments",
            json!({
                "action": "subscribe",
                "subscriptions": [{"topic": "comments", "type": "*"}]
            }),
            WebSocketExpectation::HandshakeWindow,
        ),
    ])
}

pub(crate) fn websocket_edge_plans(targets: &[ClobTarget]) -> Result<Vec<WebSocketCapturePlan>> {
    let first_asset = target_asset_ids(targets)?
        .into_iter()
        .next()
        .ok_or_else(|| {
            PolyError::raw_source(
                "POLY_LARGE_CORPUS_WS_EDGE_TARGET_MISSING",
                "no token ID exists for WebSocket edge plans",
            )
        })?;
    Ok(vec![
        WebSocketCapturePlan::market(
            "edge_ws_market_invalid_token_no_custom_large",
            json!({
                "assets_ids": ["not-a-real-token"],
                "type": "market",
                "custom_feature_enabled": false
            }),
            WebSocketExpectation::NoDataEvent,
        ),
        WebSocketCapturePlan::market(
            "edge_ws_market_unsubscribe_first_message_data_large",
            json!({
                "assets_ids": [first_asset],
                "operation": "unsubscribe"
            }),
            WebSocketExpectation::DataEvent,
        ),
        WebSocketCapturePlan::rtds(
            "edge_ws_rtds_unknown_topic_large",
            "not_a_topic",
            json!({
                "action": "subscribe",
                "subscriptions": [{"topic": "not_a_topic", "type": "update"}]
            }),
            WebSocketExpectation::ErrorFrame,
        ),
        WebSocketCapturePlan::rtds_text(
            "edge_ws_rtds_malformed_subscription_no_payload_large",
            "malformed-subscription",
            "{not-json",
            WebSocketExpectation::NoDataEvent,
        ),
        WebSocketCapturePlan::rtds(
            "edge_ws_rtds_equity_aapl_blocked_runtime_large",
            "equity_prices",
            json!({
                "action": "subscribe",
                "subscriptions": [
                    {"topic": "equity_prices", "type": "*", "filters": "{\"symbol\":\"AAPL\"}"}
                ]
            }),
            WebSocketExpectation::NoDataEvent,
        ),
    ])
}

fn target_asset_ids(targets: &[ClobTarget]) -> Result<Vec<String>> {
    let ids = targets
        .iter()
        .take(6)
        .map(|target| target.token_id.clone())
        .collect::<Vec<_>>();
    if ids.is_empty() {
        return Err(PolyError::raw_source(
            "POLY_LARGE_CORPUS_WS_TARGETS_EMPTY",
            "no token IDs were derived for public WebSocket capture",
        ));
    }
    Ok(ids)
}

#[derive(Debug, Clone)]
pub(crate) struct WebSocketCapturePlan {
    pub(crate) dataset: String,
    pub(crate) source: String,
    pub(crate) endpoint: String,
    pub(crate) docs_url: String,
    pub(crate) url: String,
    pub(crate) outbound_messages: Vec<String>,
    pub(crate) shape: WebSocketShape,
    pub(crate) expectation: WebSocketExpectation,
    pub(crate) min_data_events: usize,
    pub(crate) max_frames: usize,
    pub(crate) max_wait_secs: u64,
    pub(crate) heartbeat_message: Option<&'static str>,
    pub(crate) heartbeat_interval_secs: u64,
    pub(crate) send_initial_heartbeat: bool,
    pub(crate) respond_to_text_ping: bool,
}

impl WebSocketCapturePlan {
    fn market(dataset: &str, subscription: Value, expectation: WebSocketExpectation) -> Self {
        Self {
            dataset: dataset.to_string(),
            source: "websocket-market".to_string(),
            endpoint: "market".to_string(),
            docs_url: MARKET_WS_DOCS_URL.to_string(),
            url: MARKET_WS_URL.to_string(),
            outbound_messages: vec![subscription.to_string()],
            shape: WebSocketShape::Market,
            expectation,
            min_data_events: 1,
            max_frames: 30,
            max_wait_secs: 15,
            heartbeat_message: Some("PING"),
            heartbeat_interval_secs: 10,
            send_initial_heartbeat: true,
            respond_to_text_ping: false,
        }
    }

    fn sports(dataset: &str, expectation: WebSocketExpectation) -> Self {
        Self {
            dataset: dataset.to_string(),
            source: "websocket-sports".to_string(),
            endpoint: "sports".to_string(),
            docs_url: SPORTS_DOCS_URL.to_string(),
            url: SPORTS_WS_URL.to_string(),
            outbound_messages: Vec::new(),
            shape: WebSocketShape::Public,
            expectation,
            min_data_events: 1,
            max_frames: 20,
            max_wait_secs: 20,
            heartbeat_message: None,
            heartbeat_interval_secs: 0,
            send_initial_heartbeat: false,
            respond_to_text_ping: true,
        }
    }

    fn rtds(
        dataset: &str,
        endpoint: &str,
        subscription: Value,
        expectation: WebSocketExpectation,
    ) -> Self {
        Self::rtds_text(dataset, endpoint, subscription.to_string(), expectation)
    }

    fn rtds_text(
        dataset: &str,
        endpoint: &str,
        subscription: impl Into<String>,
        expectation: WebSocketExpectation,
    ) -> Self {
        Self {
            dataset: dataset.to_string(),
            source: "websocket-rtds".to_string(),
            endpoint: endpoint.to_string(),
            docs_url: RTDS_DOCS_URL.to_string(),
            url: RTDS_WS_URL.to_string(),
            outbound_messages: vec![subscription.into()],
            shape: WebSocketShape::Public,
            expectation,
            min_data_events: 1,
            max_frames: 30,
            max_wait_secs: 15,
            heartbeat_message: Some("ping"),
            heartbeat_interval_secs: 5,
            send_initial_heartbeat: true,
            respond_to_text_ping: false,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum WebSocketShape {
    Market,
    Public,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum WebSocketExpectation {
    DataEvent,
    HandshakeWindow,
    ErrorFrame,
    NoDataEvent,
}

impl WebSocketExpectation {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::DataEvent => "data_event_window",
            Self::HandshakeWindow => "handshake_window",
            Self::ErrorFrame => "expected_error_frame",
            Self::NoDataEvent => "expected_no_data_event",
        }
    }
}
