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
    CALYX_NATIVE_ARTIFACT_KIND, CALYX_NATIVE_SCHEMA_VERSION, CalyxNativeForecast,
    CalyxNativeRequest, read_calyx_native_forecast, write_calyx_native_forecast,
};
use crate::crypto_ingestor::{
    CryptoPendingResolutionRecord, ERR_CRYPTO_INGESTOR_PENDING, ingestor_error,
    register_crypto_pending,
};
use crate::forecast::{ComponentKind, ForecastComponent, logit, sigmoid};
use crate::kernel_recall::POLY_KERNEL_RECALL_MIN_RATIO;
use crate::kernel_recall_admission::produce_calyx_native_forecast_with_measured_kernel_recall;
use crate::live_calyx_native_evidence::{
    LiveCalyxNativeEvidenceStore, StoredLiveCalyxNativeEvidence,
    read_latest_live_calyx_native_evidence,
};
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
    pub panel_version: u32,
}

pub fn register_crypto_pending_for_mode<
    S: PendingForecastLedgerStore + LiveCalyxNativeEvidenceStore,
>(
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
            let forecast_at_millis = snapshot_millis(request.snapshot.snapshot_ts)?;
            let evidence = read_latest_live_calyx_native_evidence(
                store,
                request.domain,
                request.horizon_bucket,
                request.panel_version,
                forecast_at_millis,
            )?;
            let forecast = produce_live_calyx_native_forecast(
                request.snapshot,
                request.domain,
                request.horizon_bucket,
                request.panel_version,
                &evidence,
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
    panel_version: u32,
    stored_evidence: &StoredLiveCalyxNativeEvidence,
) -> Result<CalyxNativeForecast> {
    let forecast_at_millis = snapshot_millis(snapshot.snapshot_ts)?;
    stored_evidence.validate_for(domain, horizon_bucket, panel_version, forecast_at_millis)?;
    let evidence = stored_evidence.evidence();
    let baseline = market_probability(snapshot)?;
    let structural = structural_probability(snapshot, baseline)?;
    let support = structural_support(snapshot);
    let spread = snapshot
        .spread
        .filter(|value| value.is_finite() && *value >= 0.0)
        .ok_or_else(|| {
            pending_error(
                "CalyxNative structural reliability requires an observed finite non-negative spread",
            )
        })?;
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
        calibration: Some(evidence.calibration().slope.clone()),
        raw_confidence: (support as f64 / (support as f64 + 4.0)).clamp(0.2, 0.85),
        oracle_flakiness: snapshot.oracle_risk.dispute_risk.clamp(0.0, 1.0),
        oracle_validity: (1.0 - snapshot.oracle_risk.dispute_risk).clamp(0.0, 1.0),
        panel_bits: evidence.panel().panel_bits as f64,
        anchor_entropy_bits: evidence.panel().anchor_entropy_bits as f64,
        superiority_tiers: SuperiorityTiers {
            oracle_self_consistency: (1.0 - snapshot.oracle_risk.dispute_risk).clamp(0.0, 1.0),
            panel_sufficient: evidence.panel_sufficient(),
            kernel_recall_ratio: 0.0,
            min_kernel_recall_ratio: POLY_KERNEL_RECALL_MIN_RATIO,
            calibrated: evidence.calibrated(),
            goodhart_defended: evidence.goodhart_defended(),
            mistake_closed: evidence.mistake_closed(),
        },
        evidence: Some(stored_evidence.evidence_ref()),
    };
    produce_calyx_native_forecast_with_measured_kernel_recall(
        request,
        evidence.kernel_recall(),
        &FixedClock::new(forecast_at_millis),
    )
}

pub fn register_crypto_pending_from_calyx_native_artifact<
    S: PendingForecastLedgerStore + LiveCalyxNativeEvidenceStore,
>(
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
    if forecast.schema_version != CALYX_NATIVE_SCHEMA_VERSION {
        return Err(pending_error(format!(
            "forecast schema {} is not {}",
            forecast.schema_version, CALYX_NATIVE_SCHEMA_VERSION
        )));
    }
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
    let forecast_at_millis = snapshot_millis(snapshot.snapshot_ts)?;
    let artifact_evidence_ref = forecast.evidence.as_ref().ok_or_else(|| {
        pending_error("CalyxNative forecast artifact has no Aster evidence reference")
    })?;
    let evidence = read_latest_live_calyx_native_evidence(
        store,
        domain,
        horizon_bucket,
        artifact_evidence_ref.panel_version,
        forecast_at_millis,
    )?;
    let evidence_ref = evidence.evidence_ref();
    evidence.validate_for(
        domain,
        horizon_bucket,
        evidence_ref.panel_version,
        forecast_at_millis,
    )?;
    if forecast.evidence.as_ref() != Some(&evidence_ref) {
        return Err(pending_error(
            "forecast artifact is not bound to the expected Aster evidence row",
        ));
    }
    if forecast.calibration.as_ref() != Some(&evidence.evidence().calibration().slope) {
        return Err(pending_error(
            "forecast artifact calibration does not match measured Aster evidence",
        ));
    }
    let expected_forecast = produce_live_calyx_native_forecast(
        snapshot,
        domain,
        horizon_bucket,
        evidence_ref.panel_version,
        &evidence,
    )?;
    if forecast.p_raw != expected_forecast.p_raw {
        return Err(pending_error(format!(
            "CalyxNative forecast artifact field p_raw does not reproduce from the snapshot and Aster evidence: actual={} ({:016x}), expected={} ({:016x})",
            forecast.p_raw,
            forecast.p_raw.to_bits(),
            expected_forecast.p_raw,
            expected_forecast.p_raw.to_bits()
        )));
    }
    if let Some(field) = forecast_reproduction_mismatch(&forecast, &expected_forecast) {
        return Err(pending_error(format!(
            "CalyxNative forecast artifact field {field} does not reproduce from the snapshot and Aster evidence"
        )));
    }
    if !forecast.admissible || !forecast.superiority.pass {
        return Err(pending_error(format!(
            "CalyxNative forecast is non-admissible: {}",
            forecast.refusal_reason
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

fn snapshot_millis(snapshot_ts: u64) -> Result<u64> {
    snapshot_ts
        .checked_mul(1_000)
        .ok_or_else(|| pending_error("snapshot timestamp overflows milliseconds"))
}

fn forecast_reproduction_mismatch(
    actual: &CalyxNativeForecast,
    expected: &CalyxNativeForecast,
) -> Option<&'static str> {
    [
        (
            actual.schema_version != expected.schema_version,
            "schema_version",
        ),
        (
            actual.artifact_kind != expected.artifact_kind,
            "artifact_kind",
        ),
        (actual.source != expected.source, "source"),
        (actual.domain != expected.domain, "domain"),
        (actual.condition_id != expected.condition_id, "condition_id"),
        (actual.token_id != expected.token_id, "token_id"),
        (
            actual.horizon_bucket != expected.horizon_bucket,
            "horizon_bucket",
        ),
        (actual.p_model != expected.p_model, "p_model"),
        (actual.confidence != expected.confidence, "confidence"),
        (
            actual.confidence_ceiling != expected.confidence_ceiling,
            "confidence_ceiling",
        ),
        (actual.components != expected.components, "components"),
        (actual.blend != expected.blend, "blend"),
        (actual.calibration != expected.calibration, "calibration"),
        (actual.superiority != expected.superiority, "superiority"),
        (actual.evidence != expected.evidence, "evidence"),
        (actual.admissible != expected.admissible, "admissible"),
        (
            actual.refusal_reason != expected.refusal_reason,
            "refusal_reason",
        ),
        (actual.trust != expected.trust, "trust"),
        (
            actual.provenance_hash != expected.provenance_hash,
            "provenance_hash",
        ),
        (actual.computed_at != expected.computed_at, "computed_at"),
    ]
    .into_iter()
    .find_map(|(mismatched, field)| mismatched.then_some(field))
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
