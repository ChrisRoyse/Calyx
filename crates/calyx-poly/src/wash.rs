//! Wash-trade screens from on-chain distinct-counterparty volume evidence.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::features::top_share;
use crate::model::{CounterpartyVolume, MarketSnapshot};

pub const WASH_TRADE_OK: &str = "CALYX_POLY_ADMISSION_WASH_TRADE_OK";
pub const WASH_TRADE_INVALID_CONFIG: &str = "CALYX_POLY_ADMISSION_WASH_TRADE_INVALID_CONFIG";
pub const WASH_TRADE_INVALID_EVIDENCE: &str = "CALYX_POLY_ADMISSION_WASH_TRADE_INVALID_EVIDENCE";
pub const WASH_TRADE_MISSING_RAW_VOLUME: &str =
    "CALYX_POLY_ADMISSION_WASH_TRADE_MISSING_RAW_VOLUME";
pub const WASH_TRADE_MISSING_COUNTERPARTY_EVIDENCE: &str =
    "CALYX_POLY_ADMISSION_WASH_TRADE_MISSING_COUNTERPARTY_EVIDENCE";
pub const WASH_TRADE_LOW_COUNTERPARTY_DIVERSITY: &str =
    "CALYX_POLY_ADMISSION_WASH_TRADE_LOW_COUNTERPARTY_DIVERSITY";
pub const WASH_TRADE_LOW_DISTINCT_VOLUME: &str =
    "CALYX_POLY_ADMISSION_WASH_TRADE_LOW_DISTINCT_COUNTERPARTY_VOLUME";
pub const WASH_TRADE_COUNTERPARTY_CONCENTRATION: &str =
    "CALYX_POLY_ADMISSION_WASH_TRADE_COUNTERPARTY_CONCENTRATION";
pub const WASH_TRADE_DISTINCT_VOLUME_EXCEEDS_RAW: &str =
    "CALYX_POLY_ADMISSION_WASH_TRADE_DISTINCT_VOLUME_EXCEEDS_RAW";

/// Thresholds for refusing wash-dominated volume evidence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WashTradeParams {
    pub min_distinct_counterparties: u32,
    pub min_distinct_counterparty_volume: f64,
    pub min_distinct_counterparty_volume_ratio: f64,
    pub max_top_counterparty_share: f64,
}

impl Default for WashTradeParams {
    fn default() -> Self {
        Self {
            min_distinct_counterparties: 5,
            min_distinct_counterparty_volume: 10_000.0,
            min_distinct_counterparty_volume_ratio: 0.50,
            max_top_counterparty_share: 0.40,
        }
    }
}

/// Readable wash-trade screen result that is stored with admission evidence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WashTradeScreen {
    pub ok: bool,
    pub code: String,
    pub reason: String,
    pub raw_volume: f64,
    pub distinct_counterparty_count: u32,
    pub distinct_counterparty_volume: f64,
    pub distinct_counterparty_volume_ratio: f64,
    pub top_counterparty_share: f64,
    pub invalid_counterparty_rows: u32,
}

impl WashTradeScreen {
    pub fn valid_state(&self) -> bool {
        !self.code.trim().is_empty()
            && !self.reason.trim().is_empty()
            && self.code.starts_with("CALYX_POLY_ADMISSION_WASH_TRADE_")
            && nonnegative(self.raw_volume)
            && nonnegative(self.distinct_counterparty_volume)
            && nonnegative(self.distinct_counterparty_volume_ratio)
            && in_unit_interval(self.top_counterparty_share)
            && ((self.ok && self.code == WASH_TRADE_OK) || (!self.ok && self.code != WASH_TRADE_OK))
    }
}

/// Compute the wash-trade screen directly from snapshot counterparty evidence.
pub fn screen_wash_trading(snapshot: &MarketSnapshot, params: &WashTradeParams) -> WashTradeScreen {
    if !valid_params(params) {
        return screen(
            false,
            WASH_TRADE_INVALID_CONFIG,
            "wash-trade thresholds must be finite and within allowed ranges",
            Counts::default(),
        );
    }

    let Some(raw_volume) = snapshot
        .volume_24h
        .filter(|value| value.is_finite() && *value > 0.0)
    else {
        return screen(
            false,
            WASH_TRADE_MISSING_RAW_VOLUME,
            "missing positive raw 24h volume",
            Counts::default(),
        );
    };

    let counterparties = aggregate_counterparties(&snapshot.counterparty_volumes);
    let distinct_volume: f64 = counterparties.volumes.iter().sum();
    let ratio = distinct_volume / raw_volume;
    let counts = Counts {
        raw_volume,
        distinct_counterparty_count: counterparties.volumes.len() as u32,
        distinct_counterparty_volume: distinct_volume,
        distinct_counterparty_volume_ratio: ratio,
        top_counterparty_share: top_share(&counterparties.volumes),
        invalid_counterparty_rows: counterparties.invalid_rows,
    };

    if counts.invalid_counterparty_rows > 0 {
        return screen(
            false,
            WASH_TRADE_INVALID_EVIDENCE,
            format!(
                "{} invalid counterparty volume row(s)",
                counts.invalid_counterparty_rows
            ),
            counts,
        );
    }
    if counts.distinct_counterparty_count == 0 {
        return screen(
            false,
            WASH_TRADE_MISSING_COUNTERPARTY_EVIDENCE,
            "missing distinct-counterparty volume evidence",
            counts,
        );
    }
    if counts.distinct_counterparty_volume > raw_volume * 1.000001 {
        return screen(
            false,
            WASH_TRADE_DISTINCT_VOLUME_EXCEEDS_RAW,
            format!(
                "distinct-counterparty volume {:.4} exceeds raw volume {:.4}",
                counts.distinct_counterparty_volume, raw_volume
            ),
            counts,
        );
    }
    if counts.distinct_counterparty_count < params.min_distinct_counterparties {
        return screen(
            false,
            WASH_TRADE_LOW_COUNTERPARTY_DIVERSITY,
            format!(
                "distinct counterparties {} below required {}",
                counts.distinct_counterparty_count, params.min_distinct_counterparties
            ),
            counts,
        );
    }
    if counts.distinct_counterparty_volume < params.min_distinct_counterparty_volume {
        return screen(
            false,
            WASH_TRADE_LOW_DISTINCT_VOLUME,
            format!(
                "distinct-counterparty volume {:.4} below required {:.4}",
                counts.distinct_counterparty_volume, params.min_distinct_counterparty_volume
            ),
            counts,
        );
    }
    if counts.distinct_counterparty_volume_ratio < params.min_distinct_counterparty_volume_ratio {
        return screen(
            false,
            WASH_TRADE_LOW_DISTINCT_VOLUME,
            format!(
                "distinct-counterparty volume ratio {:.6} below required {:.6}",
                counts.distinct_counterparty_volume_ratio,
                params.min_distinct_counterparty_volume_ratio
            ),
            counts,
        );
    }
    if counts.top_counterparty_share > params.max_top_counterparty_share {
        return screen(
            false,
            WASH_TRADE_COUNTERPARTY_CONCENTRATION,
            format!(
                "top counterparty share {:.6} above max {:.6}",
                counts.top_counterparty_share, params.max_top_counterparty_share
            ),
            counts,
        );
    }

    screen(true, WASH_TRADE_OK, "wash-trade screen passed", counts)
}

fn valid_params(params: &WashTradeParams) -> bool {
    params.min_distinct_counterparties > 0
        && nonnegative(params.min_distinct_counterparty_volume)
        && in_unit_interval(params.min_distinct_counterparty_volume_ratio)
        && in_unit_interval(params.max_top_counterparty_share)
}

fn in_unit_interval(value: f64) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}

fn nonnegative(value: f64) -> bool {
    value.is_finite() && value >= 0.0
}

fn aggregate_counterparties(rows: &[CounterpartyVolume]) -> Aggregated {
    let mut by_counterparty = BTreeMap::new();
    let mut invalid_rows = 0;
    for row in rows {
        if !valid_volume(row.volume) {
            invalid_rows += 1;
            continue;
        }
        let counterparty = row.counterparty.trim().to_ascii_lowercase();
        if counterparty.is_empty() {
            invalid_rows += 1;
            continue;
        }
        *by_counterparty.entry(counterparty).or_insert(0.0) += row.volume;
    }
    Aggregated {
        volumes: by_counterparty.into_values().collect(),
        invalid_rows,
    }
}

fn valid_volume(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

fn screen(
    ok: bool,
    code: impl Into<String>,
    reason: impl Into<String>,
    counts: Counts,
) -> WashTradeScreen {
    WashTradeScreen {
        ok,
        code: code.into(),
        reason: reason.into(),
        raw_volume: counts.raw_volume,
        distinct_counterparty_count: counts.distinct_counterparty_count,
        distinct_counterparty_volume: counts.distinct_counterparty_volume,
        distinct_counterparty_volume_ratio: counts.distinct_counterparty_volume_ratio,
        top_counterparty_share: counts.top_counterparty_share,
        invalid_counterparty_rows: counts.invalid_counterparty_rows,
    }
}

#[derive(Default)]
struct Counts {
    raw_volume: f64,
    distinct_counterparty_count: u32,
    distinct_counterparty_volume: f64,
    distinct_counterparty_volume_ratio: f64,
    top_counterparty_share: f64,
    invalid_counterparty_rows: u32,
}

struct Aggregated {
    volumes: Vec<f64>,
    invalid_rows: u32,
}
