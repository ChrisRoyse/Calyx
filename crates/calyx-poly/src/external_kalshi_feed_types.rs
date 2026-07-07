use serde::{Deserialize, Serialize};

pub const KALSHI_EXTERNAL_API_BASE_URL: &str = "https://external-api.kalshi.com/trade-api/v2";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KalshiFeedClientConfig {
    pub base_url: String,
    pub timeout_secs: u64,
    pub max_body_bytes: usize,
}

impl Default for KalshiFeedClientConfig {
    fn default() -> Self {
        Self {
            base_url: KALSHI_EXTERNAL_API_BASE_URL.to_string(),
            timeout_secs: 20,
            max_body_bytes: 5 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KalshiMarketsRequest {
    pub status: Option<String>,
    pub limit: usize,
}

impl KalshiMarketsRequest {
    pub fn with_status(status: impl Into<String>, limit: usize) -> Self {
        Self {
            status: Some(status.into()),
            limit,
        }
    }

    pub fn open(limit: usize) -> Self {
        Self::with_status("open", limit)
    }

    pub fn settled(limit: usize) -> Self {
        Self::with_status("settled", limit)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KalshiMarketsPage {
    pub url: String,
    pub status_code: u16,
    pub body_bytes: u64,
    pub body_sha256: String,
    #[serde(skip, default)]
    pub raw_body: Vec<u8>,
    pub markets: Vec<KalshiMarketRecord>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KalshiMarketRecord {
    pub ticker: String,
    pub event_ticker: Option<String>,
    pub title: String,
    pub subtitle: Option<String>,
    pub status: String,
    pub market_type: Option<String>,
    pub close_time: Option<String>,
    pub expiration_time: Option<String>,
    pub settlement_ts: Option<String>,
    pub result: Option<String>,
    pub expiration_value: Option<String>,
    pub yes_bid_dollars: Option<f64>,
    pub yes_ask_dollars: Option<f64>,
    pub no_bid_dollars: Option<f64>,
    pub no_ask_dollars: Option<f64>,
    pub last_price_dollars: Option<f64>,
    pub previous_price_dollars: Option<f64>,
    pub settlement_value_dollars: Option<f64>,
    pub liquidity_dollars: Option<f64>,
    pub volume_fp: Option<f64>,
    pub volume_24h_fp: Option<f64>,
    pub open_interest_fp: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KalshiEncodedSignal {
    pub source: String,
    pub ticker: String,
    pub feature_names: Vec<String>,
    pub values: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KalshiPersistedFeedReport {
    pub schema_version: String,
    pub artifact_kind: String,
    pub source: String,
    pub url: String,
    pub status_code: u16,
    pub body_bytes: u64,
    pub body_sha256: String,
    pub body_blake3: String,
    pub raw_path: String,
    pub parsed_path: String,
    pub summary_path: String,
    pub market_count: usize,
    pub tickers: Vec<String>,
    pub raw_readback_equal: bool,
    pub parsed_readback_equal: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExternalSignalOutcomeObservation {
    pub signal_value: f32,
    pub outcome: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExternalSignalAdmissionReport {
    pub schema_version: String,
    pub artifact_kind: String,
    pub source: String,
    pub signal_name: String,
    pub estimator: String,
    pub n_samples: usize,
    pub positive_count: usize,
    pub negative_count: usize,
    pub bits: f32,
    pub ci_low_bits: f32,
    pub ci_high_bits: f32,
    pub threshold_bits: f32,
    pub admitted: bool,
    pub code: String,
    pub reason: String,
    pub computed_at: u64,
}
