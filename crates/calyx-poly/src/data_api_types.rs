use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::{CounterpartyVolume, HolderShare, MakerShare};

pub const DATA_API_BASE_URL: &str = "https://data-api.polymarket.com";
pub const DATA_API_TRADES_OFFSET_CAP: usize = 10_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataApiClientConfig {
    pub base_url: String,
    pub timeout_secs: u64,
    pub max_body_bytes: usize,
}

impl Default for DataApiClientConfig {
    fn default() -> Self {
        Self {
            base_url: DATA_API_BASE_URL.to_string(),
            timeout_secs: 20,
            max_body_bytes: 5 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DataApiJsonPage {
    pub method: String,
    pub url: String,
    pub status_code: u16,
    pub body_bytes: u64,
    pub body_sha256: String,
    #[serde(skip, default)]
    pub raw_body: Vec<u8>,
    pub value: Value,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataApiEvidenceStatus {
    Ready,
    Absent,
    Bounded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum DataApiTradeSide {
    Buy,
    Sell,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DataApiTradesPage {
    pub http: DataApiJsonPage,
    pub trades: Vec<DataApiTradeRecord>,
    pub bounded_window: bool,
}

impl DataApiTradesPage {
    pub fn counterparty_volumes(&self) -> Vec<CounterpartyVolume> {
        trade_counterparty_volumes(&self.trades)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DataApiHoldersPage {
    pub http: DataApiJsonPage,
    pub groups: Vec<DataApiHolderGroup>,
    pub status: DataApiEvidenceStatus,
}

impl DataApiHoldersPage {
    pub fn holder_shares(&self) -> Vec<HolderShare> {
        self.groups
            .iter()
            .flat_map(DataApiHolderGroup::holder_shares)
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DataApiMarketPositionsPage {
    pub http: DataApiJsonPage,
    pub groups: Vec<DataApiMarketPositionGroup>,
    pub status: DataApiEvidenceStatus,
}

impl DataApiMarketPositionsPage {
    pub fn holder_shares(&self) -> Vec<HolderShare> {
        self.groups
            .iter()
            .flat_map(DataApiMarketPositionGroup::holder_shares)
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DataApiPositionsPage {
    pub http: DataApiJsonPage,
    pub positions: Vec<DataApiPositionRecord>,
    pub status: DataApiEvidenceStatus,
}

impl DataApiPositionsPage {
    pub fn holder_shares(&self) -> Vec<HolderShare> {
        self.positions
            .iter()
            .filter_map(DataApiPositionRecord::holder_share)
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DataApiActivityPage {
    pub http: DataApiJsonPage,
    pub activity: Vec<DataApiActivityRecord>,
    pub status: DataApiEvidenceStatus,
}

impl DataApiActivityPage {
    pub fn counterparty_volumes(&self) -> Vec<CounterpartyVolume> {
        activity_counterparty_volumes(&self.activity)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DataApiOpenInterestPage {
    pub http: DataApiJsonPage,
    pub rows: Vec<DataApiOpenInterestRecord>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DataApiBoundedWindowPage {
    pub http: DataApiJsonPage,
    pub endpoint: String,
    pub bounded: bool,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DataApiConcentrationInputs {
    pub condition_id: String,
    pub holder_shares: Vec<HolderShare>,
    pub counterparty_volumes: Vec<CounterpartyVolume>,
    pub maker_shares: Vec<MakerShare>,
    pub maker_evidence_status: DataApiEvidenceStatus,
    pub maker_evidence_reason: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DataApiHolderGroup {
    pub token_id: String,
    pub holders: Vec<DataApiHolderRecord>,
}

impl DataApiHolderGroup {
    fn holder_shares(&self) -> Vec<HolderShare> {
        self.holders
            .iter()
            .map(|holder| HolderShare {
                wallet: holder.proxy_wallet.clone(),
                amount: holder.amount,
                outcome_index: holder.outcome_index,
            })
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DataApiHolderRecord {
    pub proxy_wallet: String,
    pub asset: String,
    pub amount: f64,
    pub outcome_index: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DataApiMarketPositionGroup {
    pub token_id: String,
    pub positions: Vec<DataApiPositionRecord>,
}

impl DataApiMarketPositionGroup {
    fn holder_shares(&self) -> Vec<HolderShare> {
        self.positions
            .iter()
            .filter_map(DataApiPositionRecord::holder_share)
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DataApiPositionRecord {
    pub proxy_wallet: String,
    pub asset: String,
    pub condition_id: String,
    pub size: f64,
    pub current_price: Option<f64>,
    pub current_value: Option<f64>,
    pub outcome: Option<String>,
    pub outcome_index: u32,
}

impl DataApiPositionRecord {
    fn holder_share(&self) -> Option<HolderShare> {
        (self.size > 0.0).then(|| HolderShare {
            wallet: self.proxy_wallet.clone(),
            amount: self.size,
            outcome_index: self.outcome_index,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DataApiTradeRecord {
    pub proxy_wallet: String,
    pub side: DataApiTradeSide,
    pub asset: String,
    pub condition_id: String,
    pub size: f64,
    pub price: f64,
    pub timestamp: u64,
    pub outcome_index: u32,
    pub transaction_hash: Option<String>,
}

impl DataApiTradeRecord {
    pub fn notional_volume(&self) -> f64 {
        round12(self.size * self.price)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DataApiActivityRecord {
    pub proxy_wallet: String,
    pub activity_type: String,
    pub condition_id: String,
    pub asset: Option<String>,
    pub side: Option<DataApiTradeSide>,
    pub size: Option<f64>,
    pub usdc_size: Option<f64>,
    pub price: Option<f64>,
    pub timestamp: u64,
    pub transaction_hash: Option<String>,
    pub outcome_index: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DataApiOpenInterestRecord {
    pub market: String,
    pub value: f64,
}

pub fn build_data_api_concentration_inputs(
    condition_id: impl Into<String>,
    holder_shares: Vec<HolderShare>,
    counterparty_volumes: Vec<CounterpartyVolume>,
) -> DataApiConcentrationInputs {
    DataApiConcentrationInputs {
        condition_id: condition_id.into(),
        holder_shares,
        counterparty_volumes,
        maker_shares: Vec::new(),
        maker_evidence_status: DataApiEvidenceStatus::Absent,
        maker_evidence_reason: "Data API does not expose true resting CLOB maker-address size"
            .to_string(),
    }
}

fn trade_counterparty_volumes(trades: &[DataApiTradeRecord]) -> Vec<CounterpartyVolume> {
    let mut by_wallet = BTreeMap::new();
    for trade in trades {
        *by_wallet.entry(trade.proxy_wallet.clone()).or_insert(0.0) += trade.notional_volume();
    }
    by_wallet
        .into_iter()
        .map(|(counterparty, volume)| CounterpartyVolume {
            counterparty,
            volume: round12(volume),
        })
        .collect()
}

fn activity_counterparty_volumes(activity: &[DataApiActivityRecord]) -> Vec<CounterpartyVolume> {
    let mut by_wallet = BTreeMap::new();
    for row in activity {
        let Some(volume) = row
            .usdc_size
            .or_else(|| row.size.zip(row.price).map(|(s, p)| s * p))
        else {
            continue;
        };
        *by_wallet.entry(row.proxy_wallet.clone()).or_insert(0.0) += volume;
    }
    by_wallet
        .into_iter()
        .map(|(counterparty, volume)| CounterpartyVolume {
            counterparty,
            volume: round12(volume),
        })
        .collect()
}

fn round12(value: f64) -> f64 {
    const SCALE: f64 = 1_000_000_000_000.0;
    let rounded = (value * SCALE).round() / SCALE;
    if rounded == 0.0 { 0.0 } else { rounded }
}
