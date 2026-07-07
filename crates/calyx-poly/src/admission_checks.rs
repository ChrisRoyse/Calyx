//! Admission risk-screen checks kept outside the main admission math.

use crate::admission::AdmissionInputs;

const EPS: f64 = 1.0e-9;

pub(crate) fn risk_screen_refusal(inputs: &AdmissionInputs) -> Option<(String, String)> {
    if !inputs.market_integrity.valid_state() {
        return Some((
            "CALYX_POLY_ADMISSION_INVALID_MARKET_INTEGRITY_STATE".to_string(),
            "market integrity screen state is malformed".to_string(),
        ));
    }
    if !inputs.market_integrity.ok {
        return Some((
            inputs.market_integrity.code.clone(),
            format!(
                "{}; holders={} holder_hhi={:.6} top_holder={:.6} makers={} maker_hhi={:.6} top_maker={:.6}",
                inputs.market_integrity.reason,
                inputs.market_integrity.holder_count,
                inputs.market_integrity.holder_herfindahl,
                inputs.market_integrity.top_holder_share,
                inputs.market_integrity.maker_count,
                inputs.market_integrity.maker_herfindahl,
                inputs.market_integrity.top_maker_share
            ),
        ));
    }
    if !inputs.oracle_risk.valid_state() {
        return Some((
            "CALYX_POLY_ADMISSION_INVALID_ORACLE_RISK_STATE".to_string(),
            "oracle-risk screen state is malformed".to_string(),
        ));
    }
    if (inputs.p_win - inputs.oracle_risk.p_win_adjusted).abs() > EPS {
        return Some((
            "CALYX_POLY_ADMISSION_ORACLE_RISK_PROBABILITY_NOT_ADJUSTED".to_string(),
            format!(
                "p_win {:.6} must equal oracle-adjusted p_win {:.6}",
                inputs.p_win, inputs.oracle_risk.p_win_adjusted
            ),
        ));
    }
    if !inputs.oracle_risk.ok {
        return Some((
            inputs.oracle_risk.code.clone(),
            format!(
                "{}; oracle={} dispute_risk={:.6} active_dispute={} liveness_seconds_remaining={:.4} market_price={:.6} raw_p_win={:.6} adjusted_p_win={:.6} haircut={:.6}",
                inputs.oracle_risk.reason,
                inputs.oracle_risk.oracle,
                inputs.oracle_risk.dispute_risk,
                inputs.oracle_risk.active_dispute,
                inputs.oracle_risk.liveness_seconds_remaining,
                inputs.oracle_risk.market_price,
                inputs.oracle_risk.raw_p_win,
                inputs.oracle_risk.p_win_adjusted,
                inputs.oracle_risk.p_win_haircut
            ),
        ));
    }
    if !inputs.wash_trade.valid_state() {
        return Some((
            "CALYX_POLY_ADMISSION_INVALID_WASH_TRADE_STATE".to_string(),
            "wash-trade screen state is malformed".to_string(),
        ));
    }
    if !inputs.wash_trade.ok {
        return Some((
            inputs.wash_trade.code.clone(),
            format!(
                "{}; raw_volume={:.4} distinct_counterparties={} distinct_volume={:.4} distinct_ratio={:.6} top_counterparty={:.6}",
                inputs.wash_trade.reason,
                inputs.wash_trade.raw_volume,
                inputs.wash_trade.distinct_counterparty_count,
                inputs.wash_trade.distinct_counterparty_volume,
                inputs.wash_trade.distinct_counterparty_volume_ratio,
                inputs.wash_trade.top_counterparty_share
            ),
        ));
    }
    None
}
