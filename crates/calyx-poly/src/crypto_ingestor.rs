use std::collections::BTreeMap;

use calyx_core::{CxId, SlotVector, VaultId, VaultStore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::clob_client::ClobOrderBook;
use crate::constellation::build_constellation;
use crate::crypto_forecast_registration::CryptoForecastRegistrationMode;
use crate::data_api_types::{DataApiTradeRecord, DataApiTradeSide};
use crate::error::{PolyError, Result};
use crate::features;
use crate::gamma_client::GammaMarketRecord;
use crate::lenses::PolyPanel;
use crate::model::{CounterpartyVolume, HolderShare, MarketSnapshot, OracleRiskEvidence};
use crate::pending_forecast_register::{
    PendingForecastEntry, PendingForecastLedgerStore, PendingForecastRegister,
    PendingForecastStatus, record_pending_forecast,
};
use crate::pipeline::ingest_snapshot;
use crate::score::ForecastSource;
use crate::ws_market_report::MarketWsCaptureReport;
use crate::ws_market_types::MarketWsClientConfig;

pub use crate::crypto_ingestor_live::{
    CryptoLiveCaptureRun, run_live_crypto_ingestion_cycle, select_crypto_capture_market,
};

pub const CRYPTO_INGESTOR_SCHEMA_VERSION: &str = "poly.crypto_ingestor.v1";
pub const ERR_CRYPTO_INGESTOR_INVALID_CONFIG: &str = "CALYX_POLY_CRYPTO_INGESTOR_INVALID_CONFIG";
pub const ERR_CRYPTO_INGESTOR_NO_MARKET: &str = "CALYX_POLY_CRYPTO_INGESTOR_NO_MARKET";
pub const ERR_CRYPTO_INGESTOR_NOT_PRE_RESOLUTION: &str =
    "CALYX_POLY_CRYPTO_INGESTOR_NOT_PRE_RESOLUTION";
pub const ERR_CRYPTO_INGESTOR_READBACK: &str = "CALYX_POLY_CRYPTO_INGESTOR_READBACK";
pub const ERR_CRYPTO_INGESTOR_PENDING: &str = "CALYX_POLY_CRYPTO_INGESTOR_PENDING";
pub const ERR_CRYPTO_INGESTOR_FORBIDDEN_DRIVE: &str = "CALYX_POLY_CRYPTO_INGESTOR_FORBIDDEN_DRIVE";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CryptoIngestorConfig {
    pub market_limit: usize,
    pub public_search_queries: Vec<String>,
    pub public_search_limit_per_type: usize,
    pub outcome_limit_per_market: usize,
    pub holder_limit: usize,
    pub trade_limit: usize,
    pub captured_ts: u64,
    pub min_secs_to_resolution: u64,
    pub max_secs_to_resolution: Option<u64>,
    #[serde(default)]
    pub excluded_condition_ids: Vec<String>,
    pub panel_version: u32,
    pub domain: String,
    pub horizon_bucket: String,
    pub region_vocab: Vec<String>,
    pub capture_ws: bool,
    pub ws_config: MarketWsClientConfig,
    #[serde(default)]
    pub forecast_mode: CryptoForecastRegistrationMode,
}

impl Default for CryptoIngestorConfig {
    fn default() -> Self {
        Self {
            market_limit: 10,
            public_search_queries: vec!["Bitcoin".to_string(), "Ethereum".to_string()],
            public_search_limit_per_type: 5,
            outcome_limit_per_market: 2,
            holder_limit: 100,
            trade_limit: 100,
            captured_ts: 0,
            min_secs_to_resolution: 60,
            max_secs_to_resolution: None,
            excluded_condition_ids: Vec::new(),
            panel_version: 38,
            domain: "crypto".to_string(),
            horizon_bucket: "pre_resolution".to_string(),
            region_vocab: vec!["global".to_string()],
            capture_ws: true,
            ws_config: MarketWsClientConfig::default(),
            forecast_mode: CryptoForecastRegistrationMode::BaselineMarket,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CryptoMarketInputs {
    pub market: GammaMarketRecord,
    pub books: Vec<ClobOrderBook>,
    pub holders: Vec<HolderShare>,
    pub trades: Vec<DataApiTradeRecord>,
    pub captured_ts: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CryptoSnapshotPutRecord {
    pub cx_id: String,
    pub token_id: String,
    pub snapshot_ts: u64,
    pub vault_snapshot_seq: u64,
    pub expected_constellation_sha256: String,
    pub stored_constellation_sha256: String,
    pub readback_equal: bool,
    pub flags_ungrounded: bool,
    pub absent_slots: Vec<u16>,
    pub scalar_keys: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CryptoPendingResolutionRecord {
    pub forecast_id: String,
    pub ledger_seq: u64,
    pub source: ForecastSource,
    pub p_model: f64,
    pub confidence: f64,
    pub provenance_hash: String,
    pub forecast_artifact_path: Option<String>,
    pub forecast_artifact_blake3: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CryptoSnapshotIngestRecord {
    pub put: CryptoSnapshotPutRecord,
    pub pending: CryptoPendingResolutionRecord,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CryptoIngestionRun {
    pub schema_version: String,
    pub domain: String,
    pub captured_ts: u64,
    pub market_id: String,
    pub condition_id: String,
    pub token_count: usize,
    pub snapshots: Vec<CryptoSnapshotIngestRecord>,
    pub ws_report: Option<MarketWsCaptureReport>,
}

pub fn build_crypto_market_snapshots(inputs: &CryptoMarketInputs) -> Result<Vec<MarketSnapshot>> {
    validate_pre_resolution(&inputs.market, inputs.captured_ts)?;
    let books = inputs
        .books
        .iter()
        .map(|book| (book.token_id.clone(), book))
        .collect::<BTreeMap<_, _>>();
    let prices = token_prices(&inputs.market, &books);
    let mut snapshots = Vec::with_capacity(inputs.market.clob_token_ids.len());
    for (outcome_index, token_id) in inputs.market.clob_token_ids.iter().enumerate() {
        let Some(book) = books.get(token_id).copied() else {
            continue;
        };
        let price = price_for(outcome_index, &inputs.market, book);
        let other = binary_other_price(outcome_index, &prices);
        snapshots.push(MarketSnapshot {
            token_id: token_id.clone(),
            condition_id: inputs.market.condition_id.clone(),
            outcome_index: outcome_index as u32,
            slug: inputs.market.slug.clone().unwrap_or_default(),
            question: inputs.market.question.clone(),
            event_id: inputs.market.event_id.clone(),
            category: Some("crypto".to_string()),
            region: Some("global".to_string()),
            tags: vec!["crypto".to_string()],
            resolution_source: inputs.market.resolution_source.clone(),
            neg_risk: inputs.market.neg_risk,
            snapshot_ts: inputs.captured_ts,
            price,
            mid: book.midpoint,
            best_bid: book.best_bid.or(inputs.market.best_bid),
            best_ask: book.best_ask.or(inputs.market.best_ask),
            spread: book.spread.or(inputs.market.spread),
            tick_size: book.tick_size,
            volume_24h: inputs.market.volume_24h,
            liquidity: inputs.market.liquidity,
            one_hour_change: None,
            one_day_change: None,
            ofi: ofi_for_token(token_id, &inputs.trades),
            yes_no_residual: price
                .zip(other)
                .map(|(p, o)| features::yes_no_residual(p, o)),
            secs_to_resolution: inputs
                .market
                .end_ts
                .map(|end| end.saturating_sub(inputs.captured_ts) as f64),
            holders: holders_for_outcome(outcome_index as u32, &inputs.holders),
            makers: Vec::new(),
            counterparty_volumes: counterparty_volumes(&inputs.trades),
            onchain_fills: Vec::new(),
            temporal_reference_ts: Some(inputs.captured_ts),
            sequence_position: None,
            sequence_total: None,
            oracle_risk: OracleRiskEvidence::default(),
            book: book.to_market_book(),
        });
    }
    Ok(snapshots)
}

pub fn put_crypto_snapshot<S: VaultStore>(
    store: &S,
    panel: &PolyPanel,
    snapshot: &MarketSnapshot,
    vault_id: VaultId,
    vault_salt: &[u8],
) -> Result<CryptoSnapshotPutRecord> {
    let expected = build_constellation(snapshot, panel, vault_id, vault_salt)?;
    let expected_id = expected.cx_id;
    let cx_id = ingest_snapshot(store, panel, snapshot, vault_id, vault_salt)?;
    if cx_id != expected_id {
        return Err(ingestor_error(
            ERR_CRYPTO_INGESTOR_READBACK,
            format!("vault returned {cx_id} but expected {expected_id}"),
        ));
    }
    let seq = store.snapshot();
    let stored = store.get(cx_id, seq)?;
    let mut expected_stored = expected;
    expected_stored.provenance = stored.provenance.clone();
    let expected_bytes = encode_json(&expected_stored)?;
    let stored_bytes = encode_json(&stored)?;
    if expected_bytes != stored_bytes {
        return Err(ingestor_error(
            ERR_CRYPTO_INGESTOR_READBACK,
            format!("stored constellation bytes differ for {cx_id}"),
        ));
    }
    Ok(CryptoSnapshotPutRecord {
        cx_id: cx_id.to_string(),
        token_id: snapshot.token_id.clone(),
        snapshot_ts: snapshot.snapshot_ts,
        vault_snapshot_seq: seq,
        expected_constellation_sha256: sha256_hex(&expected_bytes),
        stored_constellation_sha256: sha256_hex(&stored_bytes),
        readback_equal: true,
        flags_ungrounded: stored.flags.ungrounded,
        absent_slots: stored
            .slots
            .iter()
            .filter_map(|(slot, value)| {
                matches!(value, SlotVector::Absent { .. }).then_some(slot.get())
            })
            .collect(),
        scalar_keys: stored.scalars.keys().cloned().collect(),
    })
}

pub fn register_crypto_pending<S: PendingForecastLedgerStore>(
    store: &S,
    register: &mut PendingForecastRegister,
    snapshot: &MarketSnapshot,
    cx_id: CxId,
    domain: &str,
    horizon_bucket: &str,
) -> Result<CryptoPendingResolutionRecord> {
    let probability = snapshot.price.or(snapshot.mid).ok_or_else(|| {
        ingestor_error(
            ERR_CRYPTO_INGESTOR_PENDING,
            "pending resolution registration requires a source price or midpoint",
        )
    })?;
    if !(0.0..=1.0).contains(&probability) || !probability.is_finite() {
        return Err(ingestor_error(
            ERR_CRYPTO_INGESTOR_PENDING,
            format!("pending probability must be finite in [0,1], got {probability}"),
        ));
    }
    let provenance_hash = blake3::hash(&snapshot.canonical_input_bytes()?)
        .to_hex()
        .to_string();
    let entry = PendingForecastEntry {
        forecast_id: format!("crypto-snapshot-{cx_id}"),
        source: ForecastSource::BaselineMarket,
        condition_id: snapshot.condition_id.clone(),
        token_id: snapshot.token_id.clone(),
        outcome_index: snapshot.outcome_index,
        domain: domain.to_string(),
        horizon_bucket: horizon_bucket.to_string(),
        forecast_version: 1,
        p_model: probability,
        confidence: 0.5,
        forecast_ts: snapshot.snapshot_ts,
        provenance_hash: provenance_hash.clone(),
        status: PendingForecastStatus::Pending,
        registered_ledger_seq: None,
        terminal_ledger_seq: None,
        terminal_resolution_id: None,
        terminal_actual_win: None,
    };
    let ledger_ref = record_pending_forecast(store, register, entry)?;
    Ok(CryptoPendingResolutionRecord {
        forecast_id: format!("crypto-snapshot-{cx_id}"),
        ledger_seq: ledger_ref.seq,
        source: ForecastSource::BaselineMarket,
        p_model: probability,
        confidence: 0.5,
        provenance_hash,
        forecast_artifact_path: None,
        forecast_artifact_blake3: None,
    })
}

pub(crate) fn validate_pre_resolution(market: &GammaMarketRecord, captured_ts: u64) -> Result<()> {
    let Some(end_ts) = market.end_ts else {
        return Err(ingestor_error(
            ERR_CRYPTO_INGESTOR_NOT_PRE_RESOLUTION,
            format!("market {} has no Gamma end timestamp", market.market_id),
        ));
    };
    if !market.active || market.closed || end_ts <= captured_ts {
        return Err(ingestor_error(
            ERR_CRYPTO_INGESTOR_NOT_PRE_RESOLUTION,
            format!(
                "market {} is not pre-resolution at {captured_ts}",
                market.market_id
            ),
        ));
    }
    Ok(())
}

fn price_for(
    outcome_index: usize,
    market: &GammaMarketRecord,
    book: &ClobOrderBook,
) -> Option<f64> {
    book.midpoint
        .or_else(|| market.outcome_prices.get(outcome_index).copied())
        .or(book.last_trade_price)
        .filter(valid_probability)
}

fn valid_probability(price: &f64) -> bool {
    price.is_finite() && (0.0..=1.0).contains(price)
}

fn token_price_for(
    outcome_index: usize,
    market: &GammaMarketRecord,
    book: Option<&ClobOrderBook>,
) -> Option<f64> {
    book.and_then(|book| book.midpoint)
        .or_else(|| market.outcome_prices.get(outcome_index).copied())
        .or_else(|| book.and_then(|book| book.last_trade_price))
        .filter(valid_probability)
}

fn token_prices<'a>(
    market: &'a GammaMarketRecord,
    books: &BTreeMap<String, &'a ClobOrderBook>,
) -> BTreeMap<String, Option<f64>> {
    market
        .clob_token_ids
        .iter()
        .enumerate()
        .map(|(index, token)| {
            (
                token.clone(),
                token_price_for(index, market, books.get(token).copied()),
            )
        })
        .collect()
}

fn binary_other_price(index: usize, prices: &BTreeMap<String, Option<f64>>) -> Option<f64> {
    if prices.len() != 2 {
        return None;
    }
    prices.values().enumerate().find_map(
        |(candidate, price)| {
            if candidate != index { *price } else { None }
        },
    )
}

fn ofi_for_token(token_id: &str, trades: &[DataApiTradeRecord]) -> Option<f64> {
    let mut buy = 0.0;
    let mut sell = 0.0;
    for trade in trades.iter().filter(|trade| trade.asset == token_id) {
        match trade.side {
            DataApiTradeSide::Buy => buy += trade.notional_volume(),
            DataApiTradeSide::Sell => sell += trade.notional_volume(),
        }
    }
    features::order_flow_imbalance(buy, sell)
}

fn holders_for_outcome(outcome_index: u32, holders: &[HolderShare]) -> Vec<HolderShare> {
    holders
        .iter()
        .filter(|holder| holder.outcome_index == outcome_index)
        .cloned()
        .collect()
}

fn counterparty_volumes(trades: &[DataApiTradeRecord]) -> Vec<CounterpartyVolume> {
    let mut by_wallet = BTreeMap::<String, f64>::new();
    for trade in trades {
        *by_wallet.entry(trade.proxy_wallet.clone()).or_default() += trade.notional_volume();
    }
    by_wallet
        .into_iter()
        .map(|(counterparty, volume)| CounterpartyVolume {
            counterparty,
            volume,
        })
        .collect()
}

fn encode_json<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    serde_json::to_vec(value).map_err(|err| {
        ingestor_error(
            ERR_CRYPTO_INGESTOR_READBACK,
            format!("encode readback JSON bytes: {err}"),
        )
    })
}

pub(crate) fn reject_forbidden_drive(path: &std::path::Path) -> Result<()> {
    let text = path.display().to_string().replace('/', "\\");
    if text.to_ascii_lowercase().starts_with("d:\\") {
        return Err(ingestor_error(
            ERR_CRYPTO_INGESTOR_FORBIDDEN_DRIVE,
            format!("D: drive is forbidden for Poly evidence: {text}"),
        ));
    }
    Ok(())
}

pub(crate) fn ingestor_error(code: impl Into<String>, message: impl Into<String>) -> PolyError {
    PolyError::raw_source(code, message)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}
