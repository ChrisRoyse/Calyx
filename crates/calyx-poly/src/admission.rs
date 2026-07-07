//! Forecast admission for local Poly forecasts — a forecast-**quality** gate, not bet sizing.
//!
//! A forecast is admitted only when every forecast-quality screen passes: panel sufficiency /
//! honesty gate, confidence within the never-reaches-1 ceiling, a conformally-calibrated ward guard
//! that accepts the candidate, sufficient grounding anchors, non-stale / non-circular / source-
//! derived evidence, the domain super-intelligence (forecast-superiority) predicate, and the
//! market-integrity / oracle-risk / wash-trade data screens. The default is refusal.
//!
//! This module NEVER computes bet size, market edge (`p_win - ask`), expected value, trading fees,
//! or bankroll/exposure caps. Poly is local-forecast-only: it does not trade, and admission must not
//! optimize for betting PnL (doctrine #5 / direction #159 / no-trade runtime #162). The prior
//! implementation gated on the Kelly criterion, per-share EV from the market ask, a taker-fee model,
//! and daily/domain/active bankroll caps; all of that betting economics is removed (issue #180).

use std::cmp::Ordering;

use calyx_assay::TrustTag;
use serde::{Deserialize, Serialize};

use crate::admission_checks::risk_screen_refusal;
use crate::oracle::OracleRiskScreen;
use crate::risk::MarketIntegrityScreen;
use crate::wash::WashTradeScreen;

/// Tunable forecast-admission parameters. Forecast-quality thresholds only — no bet economics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdmissionParams {
    /// Minimum model probability for a forecast to be strong enough to admit.
    pub min_p_win: f64,
    /// Target guard false-accept rate (conformal calibration target).
    pub target_far: f64,
    /// Guard calibration significance level.
    pub alpha: f64,
    /// Required grounded outcome anchors before a domain leaves the provisional vault.
    pub min_grounding_anchors: u32,
    /// Minimum count of source-derived (non-self-referential) evidence references.
    pub min_source_derived_evidence: u32,
    /// Forecast mistake-closure circuit breaker: stop admitting once the recent forecast error
    /// score reaches this cap. This is a *quality* guard on a mis-calibrated domain — it measures
    /// forecast error, not bankroll, and never sizes or restricts a bet.
    pub max_daily_error_score: f64,
}

impl Default for AdmissionParams {
    fn default() -> Self {
        Self {
            min_p_win: 0.90,
            target_far: 0.10,
            alpha: 0.05,
            min_grounding_anchors: 50,
            min_source_derived_evidence: 1,
            max_daily_error_score: 10.0,
        }
    }
}

/// Everything forecast admission needs for one local forecast candidate — quality signals only.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdmissionInputs {
    /// Model probability for the forecast outcome.
    pub p_win: f64,
    /// Forecast confidence. Must respect the never-reaches-1 ceiling (`< 1`): grounded confidence
    /// is `n/(n+1)` and the ceiling is `min(raw, self-consistency, DPI)`, all strictly below 1.
    pub confidence: f64,
    /// Panel sufficiency / honesty gate result (`I(panel;anchor) >= H(anchor)`).
    pub sufficiency_ok: bool,
    /// Total local evidence references supplied for the forecast.
    pub evidence_count: u32,
    /// Count of source-derived (non-self-referential) evidence references.
    pub source_derived_evidence_count: u32,
    /// Count of stale evidence references.
    pub stale_evidence_count: u32,
    /// Count of circular / self-referential evidence references.
    pub circular_evidence_count: u32,
    /// Domain super-intelligence / forecast-superiority predicate.
    pub super_intel_pass: bool,
    /// Ward guard is conformally calibrated for the domain.
    pub guard_calibrated: bool,
    /// Grounded outcome anchor count for the domain.
    pub grounding_anchor_count: u32,
    /// Ward guard accepted the candidate (in-distribution).
    pub guard_pass: bool,
    /// Public-market liquidity is sufficient for the observed price to be a meaningful signal
    /// (a data-quality screen — Poly reads the price, it never trades against the book).
    pub liquidity_ok: bool,
    /// Market-integrity (holder/maker concentration) data screen.
    pub market_integrity: MarketIntegrityScreen,
    /// Oracle/dispute-risk data screen.
    pub oracle_risk: OracleRiskScreen,
    /// Wash-trade (distinct-counterparty) data screen.
    pub wash_trade: WashTradeScreen,
    /// Global forecast kill switch.
    pub kill_switch_active: bool,
    /// Recent forecast error score (mistake-closure circuit-breaker input).
    pub daily_error_score: f64,
}

/// The admission verdict for one forecast candidate. Carries no bet-economics fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdmissionDecision {
    /// Whether the forecast cleared every quality screen.
    pub admitted: bool,
    /// Stable machine-readable admission/refusal code.
    pub code: String,
    /// Human-readable reason.
    pub reason: String,
}

impl AdmissionDecision {
    fn refuse(code: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            admitted: false,
            code: code.into(),
            reason: reason.into(),
        }
    }
}

/// Refusal code: the forecast's load-bearing evidence is grounded only on proxy anchors.
pub const REFUSE_PROVISIONAL_ONLY: &str = "CALYX_POLY_ADMISSION_PROVISIONAL_ONLY_EVIDENCE";
/// Refusal code: no trust-tagged load-bearing evidence was supplied at all.
pub const REFUSE_MISSING_TRUST_EVIDENCE: &str = "CALYX_POLY_ADMISSION_MISSING_TRUST_EVIDENCE";

/// Refuses a forecast whose load-bearing evidence is entirely [`TrustTag::Provisional`] (issue #209
/// §3). This is distinct from #143 (false sufficiency): here the supporting bits/associations exist
/// but are grounded only on proxy anchors on still-open markets, not on a resolved outcome. A
/// Provisional record is promoted to Trusted only through a real resolution backfill (#77), never by
/// assumption — so a forecast leaning entirely on proxy-grounded evidence must be refused exactly as
/// it is on insufficient panel.
///
/// Returns `None` when at least one Trusted record backs the forecast; `Some(refusal)` when every
/// load-bearing record is Provisional; and a fail-closed refusal when no evidence is supplied.
pub fn refuse_if_provisional_only(load_bearing: &[TrustTag]) -> Option<AdmissionDecision> {
    if load_bearing.is_empty() {
        return Some(AdmissionDecision::refuse(
            REFUSE_MISSING_TRUST_EVIDENCE,
            "no load-bearing trust-tagged evidence supplied for the forecast",
        ));
    }
    if load_bearing.iter().all(|t| *t == TrustTag::Provisional) {
        return Some(AdmissionDecision::refuse(
            REFUSE_PROVISIONAL_ONLY,
            "load-bearing evidence is provisional-only; a forecast requires at least one record \
             grounded on a resolved outcome (proxy anchors are estimates until backfilled)",
        ));
    }
    None
}

/// Admits a forecast only if its load-bearing evidence is not provisional-only **and** every
/// forecast-quality screen in [`evaluate_admission`] passes. The provisional-only guard runs first
/// so a forecast leaning entirely on proxy-grounded bits is refused before any quality check.
pub fn admit_forecast(
    params: &AdmissionParams,
    inputs: &AdmissionInputs,
    load_bearing_trust: &[TrustTag],
) -> AdmissionDecision {
    if let Some(refusal) = refuse_if_provisional_only(load_bearing_trust) {
        return refusal;
    }
    evaluate_admission(params, inputs)
}

fn is_at_least(value: f64, threshold: f64) -> bool {
    matches!(
        value.partial_cmp(&threshold),
        Some(Ordering::Greater | Ordering::Equal)
    )
}

fn is_nonnegative_finite(value: f64) -> bool {
    value.is_finite() && value >= 0.0
}

/// Evaluate one forecast candidate against the local forecast-quality gate.
///
/// Returns an [`AdmissionDecision`]; the default is refusal. Unlike the prior betting design, this
/// takes no confidence budget / bankroll and computes no edge, EV, Kelly fraction, or fee.
pub fn evaluate_admission(params: &AdmissionParams, inputs: &AdmissionInputs) -> AdmissionDecision {
    if !inputs.p_win.is_finite() || !(0.0..=1.0).contains(&inputs.p_win) {
        return AdmissionDecision::refuse(
            "CALYX_POLY_ADMISSION_INVALID_FORECAST_INPUT",
            "p_win must be finite in [0, 1]",
        );
    }
    if !inputs.confidence.is_finite() || inputs.confidence < 0.0 {
        return AdmissionDecision::refuse(
            "CALYX_POLY_ADMISSION_INVALID_FORECAST_INPUT",
            "confidence must be finite and non-negative",
        );
    }
    if inputs.confidence >= 1.0 {
        return AdmissionDecision::refuse(
            "CALYX_POLY_ADMISSION_CONFIDENCE_CEILING",
            format!(
                "confidence {:.6} must be < 1; grounded confidence is n/(n+1) and never reaches \
                 certainty",
                inputs.confidence
            ),
        );
    }

    if inputs.kill_switch_active {
        return AdmissionDecision::refuse(
            "CALYX_POLY_ADMISSION_KILL_SWITCH_ACTIVE",
            "global kill switch is active",
        );
    }
    if !is_nonnegative_finite(params.max_daily_error_score) {
        return AdmissionDecision::refuse(
            "CALYX_POLY_ADMISSION_INVALID_CAPS",
            "daily error cap must be finite and non-negative",
        );
    }
    if !is_nonnegative_finite(inputs.daily_error_score) {
        return AdmissionDecision::refuse(
            "CALYX_POLY_ADMISSION_INVALID_SCORE_STATE",
            "daily error score must be finite and non-negative",
        );
    }
    if is_at_least(inputs.daily_error_score, params.max_daily_error_score) {
        return AdmissionDecision::refuse(
            "CALYX_POLY_ADMISSION_DAILY_ERROR_LIMIT",
            format!(
                "daily error score {:.4} reached limit {:.4}",
                inputs.daily_error_score, params.max_daily_error_score
            ),
        );
    }
    if inputs.source_derived_evidence_count > inputs.evidence_count
        || inputs.stale_evidence_count > inputs.evidence_count
        || inputs.circular_evidence_count > inputs.evidence_count
    {
        return AdmissionDecision::refuse(
            "CALYX_POLY_ADMISSION_INVALID_EVIDENCE_STATE",
            "evidence counters must not exceed total evidence count",
        );
    }
    if inputs.evidence_count == 0 {
        return AdmissionDecision::refuse(
            "CALYX_POLY_ADMISSION_MISSING_EVIDENCE",
            "no local evidence references were supplied",
        );
    }
    if inputs.stale_evidence_count > 0 {
        return AdmissionDecision::refuse(
            "CALYX_POLY_ADMISSION_STALE_EVIDENCE",
            format!(
                "{} evidence reference(s) are stale",
                inputs.stale_evidence_count
            ),
        );
    }
    if inputs.circular_evidence_count > 0 {
        return AdmissionDecision::refuse(
            "CALYX_POLY_ADMISSION_CIRCULAR_EVIDENCE",
            format!(
                "{} evidence reference(s) are circular or self-referential",
                inputs.circular_evidence_count
            ),
        );
    }
    if inputs.source_derived_evidence_count < params.min_source_derived_evidence {
        return AdmissionDecision::refuse(
            "CALYX_POLY_ADMISSION_LOW_SOURCE_SUPPORT",
            format!(
                "source-derived evidence count {} below {}",
                inputs.source_derived_evidence_count, params.min_source_derived_evidence
            ),
        );
    }
    if let Some((code, reason)) = risk_screen_refusal(inputs) {
        return AdmissionDecision::refuse(code, reason);
    }
    if !inputs.sufficiency_ok {
        return AdmissionDecision::refuse(
            "CALYX_POLY_ADMISSION_INSUFFICIENT_PANEL",
            "insufficient panel (no outcome bits)",
        );
    }
    if !inputs.super_intel_pass {
        return AdmissionDecision::refuse(
            "CALYX_POLY_ADMISSION_DOMAIN_NOT_SUPER_INTEL",
            "domain fails super-intelligence predicate",
        );
    }
    if !inputs.guard_calibrated {
        return AdmissionDecision::refuse(
            "CALYX_POLY_ADMISSION_UNCALIBRATED_GUARD",
            "provisional vault: ward guard is not calibrated",
        );
    }
    if inputs.grounding_anchor_count < params.min_grounding_anchors {
        return AdmissionDecision::refuse(
            "CALYX_POLY_ADMISSION_INSUFFICIENT_GROUNDING_ANCHORS",
            format!(
                "provisional vault: {} grounding anchors below {}",
                inputs.grounding_anchor_count, params.min_grounding_anchors
            ),
        );
    }
    if !inputs.guard_pass {
        return AdmissionDecision::refuse(
            "CALYX_POLY_ADMISSION_GUARD_REFUSED",
            "ward guard did not pass (OOD / uncalibrated)",
        );
    }
    if !is_at_least(inputs.p_win, params.min_p_win) {
        return AdmissionDecision::refuse(
            "CALYX_POLY_ADMISSION_LOW_PROBABILITY",
            format!("p_win {:.3} below {:.3}", inputs.p_win, params.min_p_win),
        );
    }
    if !inputs.liquidity_ok {
        return AdmissionDecision::refuse(
            "CALYX_POLY_ADMISSION_INSUFFICIENT_LIQUIDITY",
            "insufficient public-market liquidity for the price to be a meaningful signal",
        );
    }

    AdmissionDecision {
        admitted: true,
        code: "CALYX_POLY_ADMISSION_ADMITTED".to_string(),
        reason: "admitted".to_string(),
    }
}
