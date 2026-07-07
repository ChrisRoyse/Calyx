//! Thin/manipulable-market screens computed from persisted public-market evidence.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::features::{herfindahl, top_share};
use crate::model::{HolderShare, MakerShare, MakerShareEvidenceSource, MarketSnapshot};

pub const MARKET_INTEGRITY_OK: &str = "CALYX_POLY_ADMISSION_MARKET_INTEGRITY_OK";
pub const MARKET_INTEGRITY_INVALID_CONFIG: &str =
    "CALYX_POLY_ADMISSION_MARKET_INTEGRITY_INVALID_CONFIG";
pub const MARKET_INTEGRITY_INVALID_EVIDENCE: &str =
    "CALYX_POLY_ADMISSION_MARKET_INTEGRITY_INVALID_EVIDENCE";
pub const MARKET_INTEGRITY_MISSING_HOLDER_EVIDENCE: &str =
    "CALYX_POLY_ADMISSION_MARKET_INTEGRITY_MISSING_HOLDER_EVIDENCE";
pub const MARKET_INTEGRITY_LOW_HOLDER_DIVERSITY: &str =
    "CALYX_POLY_ADMISSION_MARKET_INTEGRITY_LOW_HOLDER_DIVERSITY";
pub const MARKET_INTEGRITY_HOLDER_CONCENTRATION: &str =
    "CALYX_POLY_ADMISSION_MARKET_INTEGRITY_HOLDER_CONCENTRATION";
pub const MARKET_INTEGRITY_HOLDER_DOMINANCE: &str =
    "CALYX_POLY_ADMISSION_MARKET_INTEGRITY_HOLDER_DOMINANCE";
pub const MARKET_INTEGRITY_MISSING_MAKER_EVIDENCE: &str =
    "CALYX_POLY_ADMISSION_MARKET_INTEGRITY_MISSING_MAKER_EVIDENCE";
pub const MARKET_INTEGRITY_LOW_MAKER_DIVERSITY: &str =
    "CALYX_POLY_ADMISSION_MARKET_INTEGRITY_LOW_MAKER_DIVERSITY";
pub const MARKET_INTEGRITY_MAKER_CONCENTRATION: &str =
    "CALYX_POLY_ADMISSION_MARKET_INTEGRITY_MAKER_CONCENTRATION";
pub const MARKET_INTEGRITY_MAKER_DOMINANCE: &str =
    "CALYX_POLY_ADMISSION_MARKET_INTEGRITY_MAKER_DOMINANCE";

/// Thresholds for refusing thin or manipulable market evidence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketIntegrityParams {
    pub min_holder_count: u32,
    pub max_holder_herfindahl: f64,
    pub max_top_holder_share: f64,
    pub min_maker_count: u32,
    pub max_maker_herfindahl: f64,
    pub max_top_maker_share: f64,
}

impl Default for MarketIntegrityParams {
    fn default() -> Self {
        Self {
            min_holder_count: 9,
            max_holder_herfindahl: 0.18,
            max_top_holder_share: 0.30,
            min_maker_count: 3,
            max_maker_herfindahl: 0.45,
            max_top_maker_share: 0.60,
        }
    }
}

/// Readable screen result that is stored with admission evidence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketIntegrityScreen {
    pub ok: bool,
    pub code: String,
    pub reason: String,
    pub holder_count: u32,
    pub holder_herfindahl: f64,
    pub top_holder_share: f64,
    pub invalid_holder_rows: u32,
    pub maker_count: u32,
    pub maker_herfindahl: f64,
    pub top_maker_share: f64,
    pub invalid_maker_rows: u32,
}

impl MarketIntegrityScreen {
    pub fn valid_state(&self) -> bool {
        !self.code.trim().is_empty()
            && !self.reason.trim().is_empty()
            && self
                .code
                .starts_with("CALYX_POLY_ADMISSION_MARKET_INTEGRITY_")
            && self.holder_herfindahl.is_finite()
            && (0.0..=1.0).contains(&self.holder_herfindahl)
            && self.top_holder_share.is_finite()
            && (0.0..=1.0).contains(&self.top_holder_share)
            && self.maker_herfindahl.is_finite()
            && (0.0..=1.0).contains(&self.maker_herfindahl)
            && self.top_maker_share.is_finite()
            && (0.0..=1.0).contains(&self.top_maker_share)
            && ((self.ok && self.code == MARKET_INTEGRITY_OK)
                || (!self.ok && self.code != MARKET_INTEGRITY_OK))
    }
}

/// Compute the market-integrity screen directly from the snapshot evidence.
pub fn screen_market_integrity(
    snapshot: &MarketSnapshot,
    params: &MarketIntegrityParams,
) -> MarketIntegrityScreen {
    if !valid_params(params) {
        return screen(
            false,
            MARKET_INTEGRITY_INVALID_CONFIG,
            "market-integrity thresholds must be finite and within [0, 1]",
            Counts::default(),
        );
    }

    let holders = aggregate_holders(&snapshot.holders);
    let makers = aggregate_makers(&snapshot.makers);
    let counts = Counts {
        holder_count: holders.amounts.len() as u32,
        holder_herfindahl: herfindahl(&holders.amounts),
        top_holder_share: top_share(&holders.amounts),
        invalid_holder_rows: holders.invalid_rows,
        maker_count: makers.amounts.len() as u32,
        maker_herfindahl: herfindahl(&makers.amounts),
        top_maker_share: top_share(&makers.amounts),
        invalid_maker_rows: makers.invalid_rows,
    };

    if counts.invalid_holder_rows > 0 || counts.invalid_maker_rows > 0 {
        return screen(
            false,
            MARKET_INTEGRITY_INVALID_EVIDENCE,
            format!(
                "invalid concentration evidence rows: holders={}, makers={}",
                counts.invalid_holder_rows, counts.invalid_maker_rows
            ),
            counts,
        );
    }
    if counts.holder_count == 0 {
        return screen(
            false,
            MARKET_INTEGRITY_MISSING_HOLDER_EVIDENCE,
            "missing holder concentration evidence",
            counts,
        );
    }
    if counts.holder_count < params.min_holder_count {
        return screen(
            false,
            MARKET_INTEGRITY_LOW_HOLDER_DIVERSITY,
            format!(
                "distinct holders {} below required {}",
                counts.holder_count, params.min_holder_count
            ),
            counts,
        );
    }
    if counts.holder_herfindahl > params.max_holder_herfindahl {
        return screen(
            false,
            MARKET_INTEGRITY_HOLDER_CONCENTRATION,
            format!(
                "holder HHI {:.6} above max {:.6}",
                counts.holder_herfindahl, params.max_holder_herfindahl
            ),
            counts,
        );
    }
    if counts.top_holder_share > params.max_top_holder_share {
        return screen(
            false,
            MARKET_INTEGRITY_HOLDER_DOMINANCE,
            format!(
                "top holder share {:.6} above max {:.6}",
                counts.top_holder_share, params.max_top_holder_share
            ),
            counts,
        );
    }
    if counts.maker_count == 0 {
        return screen(
            false,
            MARKET_INTEGRITY_MISSING_MAKER_EVIDENCE,
            "missing maker-address concentration evidence",
            counts,
        );
    }
    if counts.maker_count < params.min_maker_count {
        return screen(
            false,
            MARKET_INTEGRITY_LOW_MAKER_DIVERSITY,
            format!(
                "distinct makers {} below required {}",
                counts.maker_count, params.min_maker_count
            ),
            counts,
        );
    }
    if counts.maker_herfindahl > params.max_maker_herfindahl {
        return screen(
            false,
            MARKET_INTEGRITY_MAKER_CONCENTRATION,
            format!(
                "maker HHI {:.6} above max {:.6}",
                counts.maker_herfindahl, params.max_maker_herfindahl
            ),
            counts,
        );
    }
    if counts.top_maker_share > params.max_top_maker_share {
        return screen(
            false,
            MARKET_INTEGRITY_MAKER_DOMINANCE,
            format!(
                "top maker share {:.6} above max {:.6}",
                counts.top_maker_share, params.max_top_maker_share
            ),
            counts,
        );
    }

    screen(
        true,
        MARKET_INTEGRITY_OK,
        "market integrity screen passed",
        counts,
    )
}

fn valid_params(params: &MarketIntegrityParams) -> bool {
    params.min_holder_count > 0
        && params.min_maker_count > 0
        && in_unit_interval(params.max_holder_herfindahl)
        && in_unit_interval(params.max_top_holder_share)
        && in_unit_interval(params.max_maker_herfindahl)
        && in_unit_interval(params.max_top_maker_share)
}

fn in_unit_interval(value: f64) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}

fn aggregate_holders(rows: &[HolderShare]) -> Aggregated {
    let mut by_wallet = BTreeMap::new();
    let mut invalid_rows = 0;
    for row in rows {
        if !valid_amount(row.amount) {
            invalid_rows += 1;
            continue;
        }
        let wallet = normalized_key(&row.wallet);
        if wallet.is_empty() {
            invalid_rows += 1;
            continue;
        }
        *by_wallet.entry(wallet).or_insert(0.0) += row.amount;
    }
    Aggregated::from_map(by_wallet, invalid_rows)
}

fn aggregate_makers(rows: &[MakerShare]) -> Aggregated {
    let mut by_maker = BTreeMap::new();
    let mut invalid_rows = 0;
    for row in rows {
        if !valid_amount(row.size) {
            invalid_rows += 1;
            continue;
        }
        if row.evidence_source != MakerShareEvidenceSource::RestingClobOrderBook {
            invalid_rows += 1;
            continue;
        }
        let maker = normalized_key(&row.maker);
        if maker.is_empty() {
            invalid_rows += 1;
            continue;
        }
        *by_maker.entry(maker).or_insert(0.0) += row.size;
    }
    Aggregated::from_map(by_maker, invalid_rows)
}

fn normalized_key(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn valid_amount(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

fn screen(
    ok: bool,
    code: impl Into<String>,
    reason: impl Into<String>,
    counts: Counts,
) -> MarketIntegrityScreen {
    MarketIntegrityScreen {
        ok,
        code: code.into(),
        reason: reason.into(),
        holder_count: counts.holder_count,
        holder_herfindahl: counts.holder_herfindahl,
        top_holder_share: counts.top_holder_share,
        invalid_holder_rows: counts.invalid_holder_rows,
        maker_count: counts.maker_count,
        maker_herfindahl: counts.maker_herfindahl,
        top_maker_share: counts.top_maker_share,
        invalid_maker_rows: counts.invalid_maker_rows,
    }
}

#[derive(Default)]
struct Counts {
    holder_count: u32,
    holder_herfindahl: f64,
    top_holder_share: f64,
    invalid_holder_rows: u32,
    maker_count: u32,
    maker_herfindahl: f64,
    top_maker_share: f64,
    invalid_maker_rows: u32,
}

struct Aggregated {
    amounts: Vec<f64>,
    invalid_rows: u32,
}

impl Aggregated {
    fn from_map(values: BTreeMap<String, f64>, invalid_rows: u32) -> Self {
        Self {
            amounts: values.into_values().collect(),
            invalid_rows,
        }
    }
}
