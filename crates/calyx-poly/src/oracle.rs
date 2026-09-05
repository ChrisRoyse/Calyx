//! Oracle-risk screens for local forecast admission.

use serde::{Deserialize, Serialize};

use crate::model::MarketSnapshot;

pub const ORACLE_RISK_OK: &str = "CALYX_POLY_ADMISSION_ORACLE_RISK_OK";
pub const ORACLE_RISK_INVALID_CONFIG: &str = "CALYX_POLY_ADMISSION_ORACLE_RISK_INVALID_CONFIG";
pub const ORACLE_RISK_INVALID_EVIDENCE: &str = "CALYX_POLY_ADMISSION_ORACLE_RISK_INVALID_EVIDENCE";
pub const ORACLE_RISK_MISSING_UMA_EVIDENCE: &str =
    "CALYX_POLY_ADMISSION_ORACLE_RISK_MISSING_UMA_EVIDENCE";
pub const ORACLE_RISK_ACTIVE_DISPUTE: &str = "CALYX_POLY_ADMISSION_ORACLE_RISK_ACTIVE_DISPUTE";
pub const ORACLE_RISK_LIVENESS_WINDOW: &str = "CALYX_POLY_ADMISSION_ORACLE_RISK_LIVENESS_WINDOW";
pub const ORACLE_RISK_ELEVATED_DISPUTE: &str = "CALYX_POLY_ADMISSION_ORACLE_RISK_ELEVATED_DISPUTE";
pub const ORACLE_RISK_NEAR_CERTAIN_PRICE: &str =
    "CALYX_POLY_ADMISSION_ORACLE_RISK_NEAR_CERTAIN_PRICE";

/// Thresholds for refusing oracle-risky forecast admissions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OracleRiskParams {
    pub max_dispute_risk: f64,
    pub max_liveness_seconds_remaining: f64,
    pub near_certain_price_floor: f64,
    pub dispute_risk_haircut_scale: f64,
}

impl Default for OracleRiskParams {
    fn default() -> Self {
        Self {
            max_dispute_risk: 0.20,
            max_liveness_seconds_remaining: 0.0,
            near_certain_price_floor: 0.99,
            dispute_risk_haircut_scale: 1.0,
        }
    }
}

/// Readable oracle-risk screen result that is stored with admission evidence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OracleRiskScreen {
    pub ok: bool,
    pub code: String,
    pub reason: String,
    pub oracle: String,
    pub raw_p_win: f64,
    pub p_win_haircut: f64,
    pub p_win_adjusted: f64,
    pub dispute_risk: f64,
    pub active_dispute: bool,
    pub liveness_seconds_remaining: f64,
    pub market_price: f64,
    pub near_certain_price: bool,
}

impl OracleRiskScreen {
    pub fn valid_state(&self) -> bool {
        !self.code.trim().is_empty()
            && !self.reason.trim().is_empty()
            && self.code.starts_with("CALYX_POLY_ADMISSION_ORACLE_RISK_")
            && in_unit_interval(self.raw_p_win)
            && nonnegative(self.p_win_haircut)
            && in_unit_interval(self.p_win_adjusted)
            && self.p_win_adjusted <= self.raw_p_win + 1.0e-9
            && in_unit_interval(self.dispute_risk)
            && nonnegative(self.liveness_seconds_remaining)
            && in_unit_interval(self.market_price)
            && ((self.ok && self.code == ORACLE_RISK_OK)
                || (!self.ok && self.code != ORACLE_RISK_OK))
    }
}

pub fn screen_oracle_risk(
    snapshot: &MarketSnapshot,
    raw_p_win: f64,
    params: &OracleRiskParams,
) -> OracleRiskScreen {
    if !valid_params(params) {
        return screen(
            false,
            ORACLE_RISK_INVALID_CONFIG,
            "oracle-risk thresholds must be finite and within allowed ranges",
            Values::from_raw(raw_p_win),
        );
    }
    if !in_unit_interval(raw_p_win) {
        return screen(
            false,
            ORACLE_RISK_INVALID_EVIDENCE,
            "raw p_win must be finite in [0, 1]",
            Values::from_raw(0.0),
        );
    }
    let Some(market_price) = market_price(snapshot) else {
        return screen(
            false,
            ORACLE_RISK_INVALID_EVIDENCE,
            "missing finite market price for near-certain oracle-risk screen",
            Values::from_raw(raw_p_win),
        );
    };
    let source = snapshot
        .resolution_source
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let evidence = &snapshot.oracle_risk;
    let oracle = evidence.oracle.trim().to_ascii_lowercase();
    if !source.contains("uma") || oracle != "uma" {
        return screen(
            false,
            ORACLE_RISK_MISSING_UMA_EVIDENCE,
            "missing UMA oracle evidence for Polymarket resolution risk",
            Values {
                oracle,
                raw_p_win,
                market_price,
                ..Values::default()
            },
        );
    }
    if !in_unit_interval(evidence.dispute_risk) || !nonnegative(evidence.liveness_seconds_remaining)
    {
        return screen(
            false,
            ORACLE_RISK_INVALID_EVIDENCE,
            "oracle evidence must contain finite dispute risk and liveness values",
            Values {
                oracle,
                raw_p_win,
                market_price,
                ..Values::default()
            },
        );
    }

    let haircut = evidence.dispute_risk * params.dispute_risk_haircut_scale;
    let p_win_adjusted = (raw_p_win - haircut).max(0.0);
    let near_certain_price = market_price >= params.near_certain_price_floor;
    let values = Values {
        oracle,
        raw_p_win,
        p_win_haircut: haircut,
        p_win_adjusted,
        dispute_risk: evidence.dispute_risk,
        active_dispute: evidence.active_dispute,
        liveness_seconds_remaining: evidence.liveness_seconds_remaining,
        market_price,
        near_certain_price,
    };

    if evidence.active_dispute {
        return screen(
            false,
            ORACLE_RISK_ACTIVE_DISPUTE,
            "UMA dispute is active",
            values,
        );
    }
    if evidence.liveness_seconds_remaining > params.max_liveness_seconds_remaining {
        return screen(
            false,
            ORACLE_RISK_LIVENESS_WINDOW,
            "UMA optimistic-oracle liveness window is still open",
            values,
        );
    }
    if evidence.dispute_risk > params.max_dispute_risk {
        return screen(
            false,
            ORACLE_RISK_ELEVATED_DISPUTE,
            "UMA dispute-risk score exceeds threshold",
            values,
        );
    }
    if near_certain_price {
        return screen(
            false,
            ORACLE_RISK_NEAR_CERTAIN_PRICE,
            "market price is near certain before oracle finality",
            values,
        );
    }

    screen(true, ORACLE_RISK_OK, "oracle-risk screen passed", values)
}

fn market_price(snapshot: &MarketSnapshot) -> Option<f64> {
    [
        snapshot.best_ask,
        snapshot.mid,
        snapshot.price,
        snapshot.best_bid,
    ]
    .into_iter()
    .flatten()
    .filter(|value| in_unit_interval(*value))
    .max_by(|a, b| a.partial_cmp(b).expect("finite prices"))
}

fn valid_params(params: &OracleRiskParams) -> bool {
    in_unit_interval(params.max_dispute_risk)
        && nonnegative(params.max_liveness_seconds_remaining)
        && params.near_certain_price_floor.is_finite()
        && params.near_certain_price_floor > 0.5
        && params.near_certain_price_floor < 1.0
        && nonnegative(params.dispute_risk_haircut_scale)
        && params.dispute_risk_haircut_scale <= 1.0
}

fn in_unit_interval(value: f64) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}

fn nonnegative(value: f64) -> bool {
    value.is_finite() && value >= 0.0
}

fn screen(
    ok: bool,
    code: impl Into<String>,
    reason: impl Into<String>,
    values: Values,
) -> OracleRiskScreen {
    OracleRiskScreen {
        ok,
        code: code.into(),
        reason: reason.into(),
        oracle: values.oracle,
        raw_p_win: values.raw_p_win,
        p_win_haircut: values.p_win_haircut,
        p_win_adjusted: values.p_win_adjusted,
        dispute_risk: values.dispute_risk,
        active_dispute: values.active_dispute,
        liveness_seconds_remaining: values.liveness_seconds_remaining,
        market_price: values.market_price,
        near_certain_price: values.near_certain_price,
    }
}

#[derive(Default)]
struct Values {
    oracle: String,
    raw_p_win: f64,
    p_win_haircut: f64,
    p_win_adjusted: f64,
    dispute_risk: f64,
    active_dispute: bool,
    liveness_seconds_remaining: f64,
    market_price: f64,
    near_certain_price: bool,
}

impl Values {
    fn from_raw(raw_p_win: f64) -> Self {
        Self {
            raw_p_win,
            p_win_adjusted: raw_p_win,
            ..Self::default()
        }
    }
}
