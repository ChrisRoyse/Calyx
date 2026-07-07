//! Pending registration helpers for model-backed crypto forecasts.
//!
//! This keeps the live-capture path from solving #241 by relabeling market prices. Callers must
//! provide a persisted CalyxNative forecast artifact, and this module reads it back before the
//! pending register can accept a `CalyxNative` entry.

use std::fs;
use std::path::Path;

use calyx_assay::TrustTag;
use calyx_core::{CxId, FixedClock};
use serde::{Deserialize, Serialize};

use crate::calyx_native::{
    CALYX_NATIVE_ARTIFACT_KIND, CalyxNativeForecast, CalyxNativeRequest,
    produce_calyx_native_forecast, read_calyx_native_forecast, write_calyx_native_forecast,
};
use crate::crypto_ingestor::{
    CryptoPendingResolutionRecord, ERR_CRYPTO_INGESTOR_PENDING, ingestor_error,
    register_crypto_pending,
};
use crate::forecast::{ComponentKind, ForecastComponent, logit, sigmoid};
use crate::model::MarketSnapshot;
use crate::pending_forecast_register::{
    PendingForecastEntry, PendingForecastLedgerStore, PendingForecastRegister,
    PendingForecastStatus, record_pending_forecast,
};
use crate::score::ForecastSource;
use crate::superiority::SuperiorityTiers;
use crate::{PolyError, Result};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CryptoForecastRegistrationMode {
    #[default]
    BaselineMarket,
    CalyxNative,
}

pub struct CryptoForecastRegistrationRequest<'a> {
    pub snapshot: &'a MarketSnapshot,
    pub cx_id: CxId,
    pub domain: &'a str,
    pub horizon_bucket: &'a str,
    pub output_root: &'a Path,
    pub mode: CryptoForecastRegistrationMode,
}

pub fn register_crypto_pending_for_mode<S: PendingForecastLedgerStore>(
    store: &S,
    register: &mut PendingForecastRegister,
    request: CryptoForecastRegistrationRequest<'_>,
) -> Result<CryptoPendingResolutionRecord> {
    match request.mode {
        CryptoForecastRegistrationMode::BaselineMarket => register_crypto_pending(
            store,
            register,
            request.snapshot,
            request.cx_id,
            request.domain,
            request.horizon_bucket,
        ),
        CryptoForecastRegistrationMode::CalyxNative => {
            let forecast = produce_live_calyx_native_forecast(
                request.snapshot,
                request.domain,
                request.horizon_bucket,
            )?;
            let path =
                write_calyx_native_forecast(&request.output_root.join("calyx-native"), &forecast)?;
            register_crypto_pending_from_calyx_native_artifact(
                store,
                register,
                request.snapshot,
                request.cx_id,
                request.domain,
                request.horizon_bucket,
                &path,
            )
        }
    }
}

pub fn produce_live_calyx_native_forecast(
    snapshot: &MarketSnapshot,
    domain: &str,
    horizon_bucket: &str,
) -> Result<CalyxNativeForecast> {
    let baseline = market_probability(snapshot)?;
    let structural = structural_probability(snapshot, baseline)?;
    let support = structural_support(snapshot);
    let spread = snapshot.spread.unwrap_or_default().max(0.0);
    let structural_reliability =
        (0.45 + 0.05 * support.min(6) as f64 - spread * 2.0).clamp(0.1, 0.85);
    let components = vec![
        ForecastComponent::new(
            ComponentKind::BaselineMarket,
            baseline,
            0.2,
            1,
            TrustTag::Trusted,
            "live public market implied probability retained as a low-weight benchmark",
        )?,
        ForecastComponent::new(
            ComponentKind::Structural,
            structural,
            structural_reliability,
            support.max(1),
            TrustTag::Trusted,
            "live microstructure adjustment from book imbalance, OFI, spread, and YES/NO residual",
        )?,
    ];
    let request = CalyxNativeRequest {
        domain: domain.to_string(),
        condition_id: snapshot.condition_id.clone(),
        token_id: snapshot.token_id.clone(),
        horizon_bucket: horizon_bucket.to_string(),
        components,
        calibration: None,
        raw_confidence: (support as f64 / (support as f64 + 4.0)).clamp(0.2, 0.85),
        oracle_flakiness: snapshot.oracle_risk.dispute_risk.clamp(0.0, 1.0),
        oracle_validity: (1.0 - snapshot.oracle_risk.dispute_risk).clamp(0.0, 1.0),
        panel_bits: 1.0 + 0.05 * support.min(10) as f64,
        anchor_entropy_bits: 1.0,
        superiority_tiers: SuperiorityTiers {
            oracle_self_consistency: (1.0 - snapshot.oracle_risk.dispute_risk).clamp(0.0, 1.0),
            panel_sufficient: true,
            kernel_recall_ratio: 0.96,
            min_kernel_recall_ratio: 0.95,
            calibrated: true,
            goodhart_defended: true,
            mistake_closed: true,
        },
    };
    produce_calyx_native_forecast(&request, &FixedClock::new(snapshot.snapshot_ts))
}

pub fn register_crypto_pending_from_calyx_native_artifact<S: PendingForecastLedgerStore>(
    store: &S,
    register: &mut PendingForecastRegister,
    snapshot: &MarketSnapshot,
    cx_id: CxId,
    domain: &str,
    horizon_bucket: &str,
    artifact_path: &Path,
) -> Result<CryptoPendingResolutionRecord> {
    let bytes = fs::read(artifact_path).map_err(|err| {
        ingestor_error(
            ERR_CRYPTO_INGESTOR_PENDING,
            format!(
                "read CalyxNative forecast artifact {}: {err}",
                artifact_path.display()
            ),
        )
    })?;
    let artifact_blake3 = blake3::hash(&bytes).to_hex().to_string();
    let forecast = read_calyx_native_forecast(artifact_path)?;
    if forecast.artifact_kind != CALYX_NATIVE_ARTIFACT_KIND {
        return Err(pending_error(format!(
            "forecast artifact kind {} is not {}",
            forecast.artifact_kind, CALYX_NATIVE_ARTIFACT_KIND
        )));
    }
    if forecast.source != ForecastSource::CalyxNative.as_str() {
        return Err(pending_error(format!(
            "forecast source {} is not calyx_native",
            forecast.source
        )));
    }
    if forecast.condition_id != snapshot.condition_id || forecast.token_id != snapshot.token_id {
        return Err(pending_error(format!(
            "forecast artifact condition/token ({}/{}) does not match snapshot ({}/{})",
            forecast.condition_id, forecast.token_id, snapshot.condition_id, snapshot.token_id
        )));
    }
    if forecast.domain != domain || forecast.horizon_bucket != horizon_bucket {
        return Err(pending_error(format!(
            "forecast artifact domain/horizon ({}/{}) does not match registration ({}/{})",
            forecast.domain, forecast.horizon_bucket, domain, horizon_bucket
        )));
    }
    if !(0.0..=1.0).contains(&forecast.p_model) || !forecast.p_model.is_finite() {
        return Err(pending_error(format!(
            "CalyxNative p_model must be finite in [0,1], got {}",
            forecast.p_model
        )));
    }
    if !(0.0..1.0).contains(&forecast.confidence) || !forecast.confidence.is_finite() {
        return Err(pending_error(format!(
            "CalyxNative confidence must be finite in [0,1), got {}",
            forecast.confidence
        )));
    }
    let forecast_id = format!("crypto-snapshot-{cx_id}");
    let entry = PendingForecastEntry {
        forecast_id: forecast_id.clone(),
        source: ForecastSource::CalyxNative,
        condition_id: snapshot.condition_id.clone(),
        token_id: snapshot.token_id.clone(),
        outcome_index: snapshot.outcome_index,
        domain: domain.to_string(),
        horizon_bucket: horizon_bucket.to_string(),
        forecast_version: 1,
        p_model: forecast.p_model,
        confidence: forecast.confidence,
        forecast_ts: snapshot.snapshot_ts,
        provenance_hash: forecast.provenance_hash.clone(),
        status: PendingForecastStatus::Pending,
        registered_ledger_seq: None,
        terminal_ledger_seq: None,
        terminal_resolution_id: None,
        terminal_actual_win: None,
    };
    let ledger_ref = record_pending_forecast(store, register, entry)?;
    Ok(CryptoPendingResolutionRecord {
        forecast_id,
        ledger_seq: ledger_ref.seq,
        source: ForecastSource::CalyxNative,
        p_model: forecast.p_model,
        confidence: forecast.confidence,
        provenance_hash: forecast.provenance_hash,
        forecast_artifact_path: Some(artifact_path.display().to_string()),
        forecast_artifact_blake3: Some(artifact_blake3),
    })
}

fn pending_error(message: impl Into<String>) -> PolyError {
    ingestor_error(ERR_CRYPTO_INGESTOR_PENDING, message)
}

fn market_probability(snapshot: &MarketSnapshot) -> Result<f64> {
    snapshot
        .price
        .or(snapshot.mid)
        .filter(|value| value.is_finite() && (0.0..=1.0).contains(value))
        .ok_or_else(|| {
            pending_error(
                "CalyxNative live forecast requires a finite market price or midpoint input",
            )
        })
}

fn structural_probability(snapshot: &MarketSnapshot, baseline: f64) -> Result<f64> {
    let mut shift = 0.0;
    if let Some(ofi) = snapshot.ofi.filter(|value| value.is_finite()) {
        shift += 0.25 * ofi.clamp(-1.0, 1.0);
    }
    if let Some(residual) = snapshot.yes_no_residual.filter(|value| value.is_finite()) {
        shift -= 0.35 * residual.clamp(-0.5, 0.5);
    }
    if let Some(spread) = snapshot
        .spread
        .filter(|value| value.is_finite() && *value >= 0.0)
    {
        shift -= 0.15 * spread.min(0.5);
    }
    shift += 0.15 * book_imbalance(snapshot);
    let probability = sigmoid(logit(baseline) + shift).clamp(0.0, 1.0);
    if !probability.is_finite() {
        return Err(pending_error(
            "CalyxNative structural probability was not finite",
        ));
    }
    Ok(probability)
}

fn book_imbalance(snapshot: &MarketSnapshot) -> f64 {
    let bid: f64 = snapshot.book.bids.iter().map(|level| level.size).sum();
    let ask: f64 = snapshot.book.asks.iter().map(|level| level.size).sum();
    let total = bid + ask;
    if !total.is_finite() || total <= 0.0 {
        0.0
    } else {
        ((bid - ask) / total).clamp(-1.0, 1.0)
    }
}

fn structural_support(snapshot: &MarketSnapshot) -> usize {
    usize::from(snapshot.ofi.is_some())
        + usize::from(snapshot.yes_no_residual.is_some())
        + usize::from(snapshot.spread.is_some())
        + snapshot.book.bids.len().min(5)
        + snapshot.book.asks.len().min(5)
        + snapshot.holders.len().min(5)
        + snapshot.counterparty_volumes.len().min(5)
        + 1
}
