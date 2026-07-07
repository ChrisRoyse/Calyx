use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::book_liquidity::PublicBookLevel;
use crate::clob_types::ClobSide;
use crate::{PolyError, Result};

pub const MARKET_WS_URL: &str = "wss://ws-subscriptions-clob.polymarket.com/ws/market";
pub const MARKET_WS_DOCS_URL: &str =
    "https://docs.polymarket.com/market-data/websocket/market-channel";
pub const MARKET_WS_ARTIFACT_KIND: &str = "poly.market_websocket.capture.v1";
pub const MARKET_WS_SCHEMA_VERSION: &str = "poly.market_websocket.v1";
pub const MARKET_WS_REPORT_FILE: &str = "market-ws-capture-report.json";

pub const ERR_WS_MARKET_CONNECT: &str = "CALYX_POLY_WS_MARKET_CONNECT";
pub const ERR_WS_MARKET_REQUEST_INVALID: &str = "CALYX_POLY_WS_MARKET_REQUEST_INVALID";
pub const ERR_WS_MARKET_JSON: &str = "CALYX_POLY_WS_MARKET_JSON";
pub const ERR_WS_MARKET_EVENT_INVALID: &str = "CALYX_POLY_WS_MARKET_EVENT_INVALID";
pub const ERR_WS_MARKET_SEND: &str = "CALYX_POLY_WS_MARKET_SEND";
pub const ERR_WS_MARKET_READ: &str = "CALYX_POLY_WS_MARKET_READ";
pub const ERR_WS_MARKET_BODY_LIMIT: &str = "CALYX_POLY_WS_MARKET_BODY_LIMIT";
pub const ERR_WS_MARKET_NO_PAYLOAD_WINDOW: &str = "CALYX_POLY_WS_MARKET_NO_PAYLOAD_WINDOW";
pub const ERR_WS_MARKET_SESSION_INCOMPLETE: &str = "CALYX_POLY_WS_MARKET_SESSION_INCOMPLETE";
pub const ERR_WS_MARKET_READBACK_MISMATCH: &str = "CALYX_POLY_WS_MARKET_READBACK_MISMATCH";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarketWsClientConfig {
    pub url: String,
    pub timeout_secs: u64,
    pub max_frames: usize,
    pub max_body_bytes: usize,
    pub heartbeat_secs: u64,
    pub min_data_events: usize,
    pub require_pong: bool,
}

impl Default for MarketWsClientConfig {
    fn default() -> Self {
        Self {
            url: MARKET_WS_URL.to_string(),
            timeout_secs: 10,
            max_frames: 24,
            max_body_bytes: 1024 * 1024,
            heartbeat_secs: 10,
            min_data_events: 1,
            require_pong: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarketWsSubscription {
    pub asset_ids: Vec<String>,
    pub custom_feature_enabled: bool,
    pub initial_dump: bool,
    pub level: Option<u8>,
}

impl MarketWsSubscription {
    pub fn new(asset_ids: Vec<String>) -> Self {
        Self {
            asset_ids,
            custom_feature_enabled: true,
            initial_dump: true,
            level: Some(2),
        }
    }

    pub fn to_wire_value(&self) -> Value {
        let mut value = serde_json::json!({
            "assets_ids": self.asset_ids,
            "type": "market",
            "initial_dump": self.initial_dump,
            "custom_feature_enabled": self.custom_feature_enabled
        });
        if let Some(level) = self.level {
            value["level"] = serde_json::json!(level);
        }
        value
    }

    pub fn to_wire_text(&self) -> Result<String> {
        validate_subscription(self)?;
        Ok(self.to_wire_value().to_string())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarketWsControlMessage {
    Pong,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MarketWsTextEnvelope {
    pub control: Option<MarketWsControlMessage>,
    pub events: Vec<MarketWsParsedEvent>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MarketWsParsedEvent {
    Book(MarketWsBook),
    PriceChange(MarketWsPriceChange),
    LastTradePrice(MarketWsLastTradePrice),
    BestBidAsk(MarketWsBestBidAsk),
    TickSizeChange(MarketWsTickSizeChange),
    Lifecycle(MarketWsLifecycleEvent),
    Unknown(MarketWsUnknownEvent),
}

impl MarketWsParsedEvent {
    pub fn event_type(&self) -> &str {
        match self {
            Self::Book(_) => "book",
            Self::PriceChange(_) => "price_change",
            Self::LastTradePrice(_) => "last_trade_price",
            Self::BestBidAsk(_) => "best_bid_ask",
            Self::TickSizeChange(_) => "tick_size_change",
            Self::Lifecycle(event) => &event.event_type,
            Self::Unknown(event) => &event.event_type,
        }
    }

    pub fn is_market_data(&self) -> bool {
        matches!(
            self,
            Self::Book(_)
                | Self::PriceChange(_)
                | Self::LastTradePrice(_)
                | Self::BestBidAsk(_)
                | Self::TickSizeChange(_)
        )
    }

    pub fn is_lifecycle(&self) -> bool {
        matches!(self, Self::Lifecycle(_))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MarketWsBook {
    pub asset_id: String,
    pub market: Option<String>,
    pub timestamp_ms: Option<u64>,
    pub hash: Option<String>,
    pub bids: Vec<PublicBookLevel>,
    pub asks: Vec<PublicBookLevel>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MarketWsPriceChange {
    pub market: Option<String>,
    pub timestamp_ms: Option<u64>,
    pub changes: Vec<MarketWsPriceChangeLevel>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MarketWsPriceChangeLevel {
    pub asset_id: String,
    pub side: ClobSide,
    pub price: f64,
    pub size: f64,
    pub removes_level: bool,
    pub hash: Option<String>,
    pub best_bid: Option<f64>,
    pub best_ask: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MarketWsLastTradePrice {
    pub asset_id: String,
    pub market: Option<String>,
    pub price: f64,
    pub size: Option<f64>,
    pub side: Option<ClobSide>,
    pub fee_rate_bps: Option<f64>,
    pub timestamp_ms: Option<u64>,
    pub transaction_hash: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MarketWsBestBidAsk {
    pub asset_id: String,
    pub market: Option<String>,
    pub best_bid: Option<f64>,
    pub best_ask: Option<f64>,
    pub timestamp_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MarketWsTickSizeChange {
    pub asset_id: Option<String>,
    pub market: Option<String>,
    pub old_tick_size: Option<f64>,
    pub new_tick_size: f64,
    pub timestamp_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MarketWsLifecycleEvent {
    pub event_type: String,
    pub market: Option<String>,
    pub asset_id: Option<String>,
    pub condition_id: Option<String>,
    pub raw: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MarketWsUnknownEvent {
    pub event_type: String,
    pub raw: Value,
}

pub fn validate_market_ws_config(config: &MarketWsClientConfig) -> Result<()> {
    if config.url.trim().is_empty()
        || config.timeout_secs == 0
        || config.max_frames == 0
        || config.max_body_bytes == 0
    {
        return Err(ws_market_error(
            ERR_WS_MARKET_REQUEST_INVALID,
            "Market WebSocket url, timeout, max_frames, and max_body_bytes must be non-empty",
        ));
    }
    Ok(())
}

pub fn validate_subscription(subscription: &MarketWsSubscription) -> Result<()> {
    if subscription.asset_ids.is_empty()
        || subscription
            .asset_ids
            .iter()
            .any(|asset_id| asset_id.trim().is_empty())
    {
        return Err(ws_market_error(
            ERR_WS_MARKET_REQUEST_INVALID,
            "Market WebSocket subscription requires at least one non-empty asset id",
        ));
    }
    Ok(())
}

pub(crate) fn ws_market_error(code: impl Into<String>, message: impl Into<String>) -> PolyError {
    PolyError::raw_source(code, message)
}
