use serde_json::{Value, json};

const SPORTS_WS_URL: &str = "wss://sports-api.polymarket.com/ws";
const SPORTS_DOCS_URL: &str = "https://docs.polymarket.com/market-data/websocket/sports";
const RTDS_WS_URL: &str = "wss://ws-live-data.polymarket.com";
const RTDS_DOCS_URL: &str = "https://docs.polymarket.com/market-data/websocket/rtds";
const SPORTS_WAIT_SECS: u64 = 15;
const RTDS_EVENT_WAIT_SECS: u64 = 15;
const RTDS_CONTROL_WAIT_SECS: u64 = 10;

#[derive(Debug, Clone)]
pub(crate) struct PublicWebSocketProbe {
    pub(crate) name: String,
    pub(crate) source: String,
    pub(crate) endpoint: String,
    pub(crate) url: String,
    pub(crate) docs_url: String,
    pub(crate) outbound_messages: Vec<String>,
    pub(crate) expected_success: bool,
    pub(crate) edge_case: bool,
    pub(crate) min_event_frames: usize,
    pub(crate) require_event_frame: bool,
    pub(crate) respond_to_text_ping: bool,
    pub(crate) send_client_ping: bool,
    pub(crate) max_wait_secs: u64,
}

pub(crate) fn public_websocket_probes() -> Vec<PublicWebSocketProbe> {
    vec![
        PublicWebSocketProbe::sports_result(),
        PublicWebSocketProbe::rtds(
            "ws_rtds_crypto_prices",
            "crypto_prices",
            json!({
                "action": "subscribe",
                "subscriptions": [{"topic": "crypto_prices", "type": "update"}]
            }),
            true,
            false,
        ),
        PublicWebSocketProbe::rtds(
            "ws_rtds_crypto_prices_chainlink",
            "crypto_prices_chainlink",
            json!({
                "action": "subscribe",
                "subscriptions": [{"topic": "crypto_prices_chainlink", "type": "*", "filters": ""}]
            }),
            true,
            false,
        ),
        PublicWebSocketProbe::rtds_accepts_subscription(
            "ws_rtds_comments_created",
            "comments",
            json!({
                "action": "subscribe",
                "subscriptions": [{"topic": "comments", "type": "*"}]
            }),
            true,
            false,
        ),
        PublicWebSocketProbe::rtds(
            "edge_ws_rtds_unknown_topic",
            "not_a_topic",
            json!({
                "action": "subscribe",
                "subscriptions": [{"topic": "not_a_topic", "type": "update"}]
            }),
            false,
            true,
        ),
        PublicWebSocketProbe::rtds_text(
            "edge_ws_rtds_malformed_subscription",
            "malformed-subscription",
            "{not-json",
            false,
            true,
        ),
        PublicWebSocketProbe::rtds(
            "ws_rtds_equity_aapl_snapshot",
            "equity_prices",
            json!({
                "action": "subscribe",
                "subscriptions": [
                    {"topic": "equity_prices", "type": "*", "filters": "{\"symbol\":\"AAPL\"}"}
                ]
            }),
            true,
            false,
        ),
        PublicWebSocketProbe::rtds(
            "edge_ws_rtds_equity_invalid_symbol_quiet",
            "equity_prices",
            json!({
                "action": "subscribe",
                "subscriptions": [
                    {"topic": "equity_prices", "type": "update", "filters": "{\"symbol\":\"NOTAREALPOLY\"}"}
                ]
            }),
            false,
            true,
        ),
        PublicWebSocketProbe::rtds(
            "edge_ws_rtds_equity_malformed_filter_quiet",
            "equity_prices",
            json!({
                "action": "subscribe",
                "subscriptions": [
                    {"topic": "equity_prices", "type": "update", "filters": "{not-json"}
                ]
            }),
            false,
            true,
        ),
    ]
}

impl PublicWebSocketProbe {
    fn sports_result() -> Self {
        Self {
            name: "ws_sports_results".to_string(),
            source: "websocket-sports".to_string(),
            endpoint: "sports".to_string(),
            url: SPORTS_WS_URL.to_string(),
            docs_url: SPORTS_DOCS_URL.to_string(),
            outbound_messages: Vec::new(),
            expected_success: true,
            edge_case: false,
            min_event_frames: 1,
            require_event_frame: true,
            respond_to_text_ping: true,
            send_client_ping: false,
            max_wait_secs: SPORTS_WAIT_SECS,
        }
    }

    fn rtds(
        name: impl Into<String>,
        endpoint: impl Into<String>,
        subscription: Value,
        expected_success: bool,
        edge_case: bool,
    ) -> Self {
        Self::rtds_text(
            name,
            endpoint,
            subscription.to_string(),
            expected_success,
            edge_case,
        )
    }

    fn rtds_text(
        name: impl Into<String>,
        endpoint: impl Into<String>,
        subscription: impl Into<String>,
        expected_success: bool,
        edge_case: bool,
    ) -> Self {
        Self {
            name: name.into(),
            source: "websocket-rtds".to_string(),
            endpoint: endpoint.into(),
            url: RTDS_WS_URL.to_string(),
            docs_url: RTDS_DOCS_URL.to_string(),
            outbound_messages: vec![subscription.into()],
            expected_success,
            edge_case,
            min_event_frames: 1,
            require_event_frame: true,
            respond_to_text_ping: false,
            send_client_ping: true,
            max_wait_secs: if expected_success {
                RTDS_EVENT_WAIT_SECS
            } else {
                RTDS_CONTROL_WAIT_SECS
            },
        }
    }

    fn rtds_accepts_subscription(
        name: impl Into<String>,
        endpoint: impl Into<String>,
        subscription: Value,
        expected_success: bool,
        edge_case: bool,
    ) -> Self {
        let mut probe = Self::rtds(name, endpoint, subscription, expected_success, edge_case);
        probe.require_event_frame = false;
        probe.max_wait_secs = RTDS_CONTROL_WAIT_SECS;
        probe
    }
}
