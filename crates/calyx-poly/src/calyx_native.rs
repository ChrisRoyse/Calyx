//! The Calyx-native forecast producer (issue #85 keystone + #90 persistence).
//!
//! This is the producer behind [`crate::score::ForecastSource::CalyxNative`] — previously an enum
//! variant with no producer. It composes the measured components (kNN base rate #81, per-slot bits
//! vote #84, oracle #83, kernel #82, structural #89) into one `p_model` by reliability-weighted
//! logit pooling (#85), de-biases it with the domain×horizon calibration slope (#86), caps its
//! confidence by `min(raw, oracle self-consistency, DPI)` (#87), and gates admissibility on the
//! six-tier superiority predicate (#88). The full prediction — every component, the blend, the
//! calibration, the ceiling, the verdict, and a provenance hash — is persisted (#90) and read back
//! as the FSV source of truth. Fail closed at every stage; a provisional-only or non-superior
//! forecast is produced but marked non-admissible, never silently upgraded.

use calyx_assay::TrustTag;
use calyx_core::Clock;
use serde::{Deserialize, Serialize};

use crate::error::{PolyError, Result};
use crate::forecast::ForecastComponent;
use crate::forecast_blend::{BlendResult, blend_components};
use crate::forecast_calibration::{CalibrationSlope, apply_calibration};
use crate::forecast_ceiling::{
    ConfidenceCeiling, ERR_CEILING_INPUT, confidence_ceiling, dpi_ceiling_from_bits,
};
use crate::score::ForecastSource;
use crate::superiority::{SuperiorityTiers, SuperiorityVerdict, evaluate_superiority};

/// Schema tag persisted with every Calyx-native forecast.
pub const CALYX_NATIVE_SCHEMA_VERSION: &str = "poly.calyx_native_forecast.v2";
/// Artifact-kind tag.
pub const CALYX_NATIVE_ARTIFACT_KIND: &str = "poly_calyx_native_forecast";
pub const ERR_CALYX_NATIVE_PROVENANCE: &str = "CALYX_POLY_CALYX_NATIVE_PROVENANCE";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalyxNativeEvidenceRef {
    pub ledger_seq: u64,
    pub recorded_at_millis: u64,
    pub panel_version: u32,
    pub payload_blake3: String,
}

/// Everything the producer needs for one market's forecast.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CalyxNativeRequest {
    /// Domain slug.
    pub domain: String,
    /// Market condition id.
    pub condition_id: String,
    /// Outcome token id.
    pub token_id: String,
    /// Horizon bucket (from `forecast_calibration::horizon_bucket`).
    pub horizon_bucket: String,
    /// The measured components to blend.
    pub components: Vec<ForecastComponent>,
    /// The domain×horizon calibration slope to de-bias with (if fitted).
    pub calibration: Option<CalibrationSlope>,
    /// Support-driven raw confidence (`n/(n+1)` style, `< 1`).
    pub raw_confidence: f64,
    /// Oracle flakiness for the self-consistency ceiling.
    pub oracle_flakiness: f64,
    /// Oracle validity for the self-consistency ceiling.
    pub oracle_validity: f64,
    /// Measured panel bits about the outcome (for the DPI ceiling).
    pub panel_bits: f64,
    /// Outcome entropy in bits (for the DPI ceiling and sufficiency).
    pub anchor_entropy_bits: f64,
    /// The six superiority tiers for the market/domain.
    pub superiority_tiers: SuperiorityTiers,
    /// Aster Ledger row that supplied measured live-admission evidence, when applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<CalyxNativeEvidenceRef>,
}

/// The persisted Calyx-native forecast — the FSV source of truth on disk.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CalyxNativeForecast {
    /// Schema tag.
    pub schema_version: String,
    /// Artifact-kind tag.
    pub artifact_kind: String,
    /// Forecast source — always `calyx_native`.
    pub source: String,
    /// Domain slug.
    pub domain: String,
    /// Market condition id.
    pub condition_id: String,
    /// Outcome token id.
    pub token_id: String,
    /// Horizon bucket.
    pub horizon_bucket: String,
    /// Blended probability before calibration.
    pub p_raw: f64,
    /// De-biased model probability (after calibration; equals `p_raw` if no slope).
    pub p_model: f64,
    /// Final confidence (`< 1`), the ceiling's minimum.
    pub confidence: f64,
    /// The full confidence-ceiling breakdown.
    pub confidence_ceiling: ConfidenceCeiling,
    /// The measured components.
    pub components: Vec<ForecastComponent>,
    /// The blend result.
    pub blend: BlendResult,
    /// The calibration slope applied (if any).
    pub calibration: Option<CalibrationSlope>,
    /// The six-tier superiority verdict.
    pub superiority: SuperiorityVerdict,
    /// Aster Ledger evidence row bound into this forecast's provenance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<CalyxNativeEvidenceRef>,
    /// Admissible iff superiority passes **and** the blend is Trusted.
    pub admissible: bool,
    /// Reason the forecast is non-admissible (empty if admissible).
    pub refusal_reason: String,
    /// Record trust (Trusted only if every contributing component was Trusted).
    pub trust: TrustTag,
    /// blake3 of the canonical payload.
    pub provenance_hash: String,
    /// Wall-clock at computation.
    pub computed_at: u64,
}

/// Produces a Calyx-native forecast for one market.
pub fn produce_calyx_native_forecast(
    req: &CalyxNativeRequest,
    clock: &dyn Clock,
) -> Result<CalyxNativeForecast> {
    let blend = blend_components(&req.components)?;
    let p_raw = blend.p_model;
    let p_model = match &req.calibration {
        Some(slope) => apply_calibration(slope, p_raw),
        None => p_raw,
    };

    let measured_panel_sufficient =
        measured_panel_sufficient(req.panel_bits, req.anchor_entropy_bits)?;
    let dpi = dpi_ceiling_from_bits(req.panel_bits, req.anchor_entropy_bits);
    let ceiling = confidence_ceiling(
        req.raw_confidence,
        req.oracle_flakiness,
        req.oracle_validity,
        dpi,
    )?;
    let mut tiers = req.superiority_tiers.clone();
    tiers.panel_sufficient = tiers.panel_sufficient && measured_panel_sufficient;
    let superiority = evaluate_superiority(&tiers)?;

    let trust = blend.trust;
    let mut refusal = String::new();
    if !superiority.pass {
        refusal = format!(
            "superiority tiers failed: {}",
            superiority.failing_tiers.join(", ")
        );
    } else if trust != TrustTag::Trusted {
        refusal = "load-bearing evidence is provisional-only (proxy-grounded)".to_string();
    }
    let admissible = refusal.is_empty();

    let provenance_hash = provenance_hash(req, p_raw, p_model, ceiling.confidence, admissible)?;

    Ok(CalyxNativeForecast {
        schema_version: CALYX_NATIVE_SCHEMA_VERSION.to_string(),
        artifact_kind: CALYX_NATIVE_ARTIFACT_KIND.to_string(),
        source: ForecastSource::CalyxNative.as_str().to_string(),
        domain: req.domain.clone(),
        condition_id: req.condition_id.clone(),
        token_id: req.token_id.clone(),
        horizon_bucket: req.horizon_bucket.clone(),
        p_raw,
        p_model,
        confidence: ceiling.confidence,
        confidence_ceiling: ceiling,
        components: req.components.clone(),
        blend,
        calibration: req.calibration.clone(),
        superiority,
        evidence: req.evidence.clone(),
        admissible,
        refusal_reason: refusal,
        trust,
        provenance_hash,
        computed_at: clock.now(),
    })
}

fn measured_panel_sufficient(panel_bits: f64, anchor_entropy_bits: f64) -> Result<bool> {
    if !panel_bits.is_finite() || panel_bits < 0.0 {
        return Err(PolyError::diagnostics(
            ERR_CEILING_INPUT,
            format!("panel_bits={panel_bits} must be finite and non-negative"),
        ));
    }
    if !anchor_entropy_bits.is_finite() || anchor_entropy_bits < 0.0 {
        return Err(PolyError::diagnostics(
            ERR_CEILING_INPUT,
            format!("anchor_entropy_bits={anchor_entropy_bits} must be finite and non-negative"),
        ));
    }
    Ok(panel_bits >= anchor_entropy_bits)
}

/// Persists a Calyx-native forecast as JSON under `dir` and returns its path.
pub fn write_calyx_native_forecast(
    dir: &std::path::Path,
    forecast: &CalyxNativeForecast,
) -> Result<std::path::PathBuf> {
    let file_name = format!(
        "calyx_native_forecast_{}_{}_{}.json",
        sanitize(&forecast.domain),
        sanitize(&forecast.condition_id),
        token_suffix(&forecast.token_id)
    );
    crate::diagnostics_store::write_json(dir, &file_name, forecast)
}

/// Reads a persisted Calyx-native forecast back from disk.
pub fn read_calyx_native_forecast(path: &std::path::Path) -> Result<CalyxNativeForecast> {
    crate::diagnostics_store::read_json(path)
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

fn token_suffix(token_id: &str) -> String {
    blake3::hash(token_id.as_bytes()).to_hex().to_string()[..16].to_string()
}

fn provenance_hash(
    req: &CalyxNativeRequest,
    p_raw: f64,
    p_model: f64,
    confidence: f64,
    admissible: bool,
) -> Result<String> {
    let mut hasher = blake3::Hasher::new();
    let request_bytes = serde_json::to_vec(req).map_err(|error| {
        PolyError::diagnostics(
            ERR_CALYX_NATIVE_PROVENANCE,
            format!("encode CalyxNative request for provenance: {error}"),
        )
    })?;
    hasher.update(&request_bytes);
    hasher.update(&p_raw.to_le_bytes());
    hasher.update(&p_model.to_le_bytes());
    hasher.update(&confidence.to_le_bytes());
    hasher.update(&[u8::from(admissible)]);
    Ok(hasher.finalize().to_hex().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forecast::ComponentKind;
    use crate::superiority::SuperiorityTiers;
    use calyx_core::FixedClock;

    fn comp(kind: ComponentKind, p: f64, r: f64, trust: TrustTag) -> ForecastComponent {
        ForecastComponent::new(kind, p, r, 100, trust, "t").unwrap()
    }

    fn strong_tiers() -> SuperiorityTiers {
        SuperiorityTiers {
            oracle_self_consistency: 0.9,
            panel_sufficient: true,
            kernel_recall_ratio: 0.97,
            min_kernel_recall_ratio: 0.95,
            calibrated: true,
            goodhart_defended: true,
            mistake_closed: true,
        }
    }

    fn base_request() -> CalyxNativeRequest {
        CalyxNativeRequest {
            domain: "crypto".into(),
            condition_id: "0xcond".into(),
            token_id: "tok".into(),
            horizon_bucket: "1h_24h".into(),
            components: vec![
                comp(ComponentKind::KnnBaseRate, 0.7, 0.8, TrustTag::Trusted),
                comp(ComponentKind::BitsVote, 0.75, 0.9, TrustTag::Trusted),
            ],
            calibration: None,
            raw_confidence: 0.95,
            oracle_flakiness: 0.05,
            oracle_validity: 0.98,
            panel_bits: 1.0,
            anchor_entropy_bits: 1.0,
            superiority_tiers: strong_tiers(),
            evidence: None,
        }
    }

    #[test]
    fn strong_forecast_is_admissible() {
        let f = produce_calyx_native_forecast(&base_request(), &FixedClock::new(1)).unwrap();
        assert_eq!(f.source, "calyx_native");
        assert!(f.p_model > 0.7 && f.p_model < 0.75);
        assert!(f.confidence < 1.0);
        assert!(
            f.admissible,
            "strong forecast must be admissible: {}",
            f.refusal_reason
        );
        assert_eq!(f.trust, TrustTag::Trusted);
    }

    #[test]
    fn failing_tier_refuses() {
        let mut req = base_request();
        req.superiority_tiers.kernel_recall_ratio = 0.5;
        let f = produce_calyx_native_forecast(&req, &FixedClock::new(1)).unwrap();
        assert!(!f.admissible);
        assert!(f.refusal_reason.contains("kernel"));
    }

    #[test]
    fn provisional_component_refuses_even_when_superior() {
        let mut req = base_request();
        req.components[0] = comp(ComponentKind::KnnBaseRate, 0.7, 0.8, TrustTag::Provisional);
        let f = produce_calyx_native_forecast(&req, &FixedClock::new(1)).unwrap();
        assert_eq!(f.trust, TrustTag::Provisional);
        assert!(!f.admissible);
        assert!(f.refusal_reason.contains("provisional"));
    }
}
