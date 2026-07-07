use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::book_liquidity::{
    BOOK_LIQUIDITY_SCHEMA_VERSION, PUBLIC_BOOK_SNAPSHOT_ARTIFACT_KIND, PublicBookLevel,
    PublicBookSnapshot,
};
use crate::model::{Book, Level};

pub const CLOB_BASE_URL: &str = "https://clob.polymarket.com";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClobClientConfig {
    pub base_url: String,
    pub timeout_secs: u64,
    pub max_body_bytes: usize,
}

impl Default for ClobClientConfig {
    fn default() -> Self {
        Self {
            base_url: CLOB_BASE_URL.to_string(),
            timeout_secs: 20,
            max_body_bytes: 5 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ClobSide {
    Buy,
    Sell,
}

impl ClobSide {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Buy => "BUY",
            Self::Sell => "SELL",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClobScalarKind {
    BuyPrice,
    SellPrice,
    Midpoint,
    Spread,
    TickSize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClobBookStatus {
    Ready,
    ThinOrEmpty,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClobJsonPage {
    pub method: String,
    pub url: String,
    pub status_code: u16,
    pub body_bytes: u64,
    pub body_sha256: String,
    #[serde(skip, default)]
    pub raw_body: Vec<u8>,
    pub value: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClobBookPage {
    pub http: ClobJsonPage,
    pub book: ClobOrderBook,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClobScalarPage {
    pub http: ClobJsonPage,
    pub quote: ClobScalarQuote,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClobHistoryPage {
    pub http: ClobJsonPage,
    pub history: ClobPriceHistory,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClobBatchBooksPage {
    pub http: ClobJsonPage,
    pub books: Vec<ClobOrderBook>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClobBatchPricesPage {
    pub http: ClobJsonPage,
    pub prices: Vec<ClobTokenPrices>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClobBatchScalarsPage {
    pub http: ClobJsonPage,
    pub kind: ClobScalarKind,
    pub quotes: Vec<ClobScalarQuote>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClobLastTradesPage {
    pub http: ClobJsonPage,
    pub trades: Vec<ClobLastTrade>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClobBatchHistoryPage {
    pub http: ClobJsonPage,
    pub histories: Vec<ClobPriceHistory>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClobOrderBook {
    pub condition_id: String,
    pub token_id: String,
    pub timestamp_ms: u64,
    pub hash: Option<String>,
    pub bids: Vec<PublicBookLevel>,
    pub asks: Vec<PublicBookLevel>,
    pub min_order_size: Option<f64>,
    pub tick_size: Option<f64>,
    pub neg_risk: Option<bool>,
    pub last_trade_price: Option<f64>,
    pub best_bid: Option<f64>,
    pub best_ask: Option<f64>,
    pub midpoint: Option<f64>,
    pub spread: Option<f64>,
    pub status: ClobBookStatus,
}

impl ClobOrderBook {
    pub fn to_market_book(&self) -> Book {
        Book {
            bids: self
                .bids
                .iter()
                .map(|level| Level {
                    price: level.price,
                    size: level.size,
                })
                .collect(),
            asks: self
                .asks
                .iter()
                .map(|level| Level {
                    price: level.price,
                    size: level.size,
                })
                .collect(),
        }
    }

    pub fn to_public_book_snapshot(
        &self,
        source_url: impl Into<String>,
        captured_ts: u64,
    ) -> PublicBookSnapshot {
        PublicBookSnapshot {
            schema_version: BOOK_LIQUIDITY_SCHEMA_VERSION.to_string(),
            artifact_kind: PUBLIC_BOOK_SNAPSHOT_ARTIFACT_KIND.to_string(),
            source_kind: "clob_book".to_string(),
            source_url: source_url.into(),
            condition_id: self.condition_id.clone(),
            token_id: self.token_id.clone(),
            snapshot_ts: self.timestamp_ms / 1000,
            captured_ts,
            bids: self.bids.clone(),
            asks: self.asks.clone(),
            volume_24h: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClobScalarQuote {
    pub token_id: String,
    pub kind: ClobScalarKind,
    pub value: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClobTokenPrices {
    pub token_id: String,
    pub buy: Option<f64>,
    pub sell: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClobPriceBatchRequest {
    pub token_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub side: Option<String>,
}

impl ClobPriceBatchRequest {
    pub fn side(token_id: impl Into<String>, side: ClobSide) -> Self {
        Self {
            token_id: token_id.into(),
            side: Some(side.as_str().to_string()),
        }
    }

    pub fn missing_side(token_id: impl Into<String>) -> Self {
        Self {
            token_id: token_id.into(),
            side: None,
        }
    }

    pub fn raw_side(token_id: impl Into<String>, side: impl Into<String>) -> Self {
        Self {
            token_id: token_id.into(),
            side: Some(side.into()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClobHistoryPoint {
    pub t: u64,
    pub p: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClobPriceHistory {
    pub token_id: String,
    pub points: Vec<ClobHistoryPoint>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClobLastTrade {
    pub token_id: String,
    pub side: ClobSide,
    pub price: f64,
}
