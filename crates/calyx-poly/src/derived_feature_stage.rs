//! Persistence-backed derived-feature computation stage (#30).
//!
//! The pure formulas live in [`crate::features`]. This module is the ingestion-stage wrapper that
//! writes a raw feature request, reads it back, computes derived values, and persists the normalized
//! row for downstream Calyx slots/scalars.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::diagnostics_store::{read_json, write_json};
use crate::error::{PolyError, Result};
use crate::features;

pub const DERIVED_FEATURE_SCHEMA_VERSION: &str = "poly.derived_feature_stage.v1";
pub const DERIVED_FEATURE_INPUT_ARTIFACT_KIND: &str = "poly_derived_feature_input";
pub const DERIVED_FEATURE_ROW_ARTIFACT_KIND: &str = "poly_derived_feature_row";

pub const DERIVED_FEATURE_READY: &str = "CALYX_POLY_DERIVED_FEATURE_READY";
pub const DERIVED_FEATURE_DEGRADED: &str = "CALYX_POLY_DERIVED_FEATURE_DEGRADED";
const ERR_INVALID_REQUEST: &str = "CALYX_POLY_DERIVED_FEATURE_INVALID_REQUEST";
const ERR_READBACK_MISMATCH: &str = "CALYX_POLY_DERIVED_FEATURE_READBACK_MISMATCH";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DerivedFeatureInput {
    pub schema_version: String,
    pub artifact_kind: String,
    pub source_id: String,
    pub token_id: String,
    pub condition_id: String,
    pub snapshot_ts: u64,
    pub buy_volume: Option<f64>,
    pub sell_volume: Option<f64>,
    pub holder_amounts: Vec<f64>,
    pub maker_sizes: Vec<f64>,
    pub yes_price: Option<f64>,
    pub no_price: Option<f64>,
    pub negrisk_yes_prices: Vec<f64>,
    pub price: Option<f64>,
    pub mid: Option<f64>,
    pub returns: Vec<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DerivedFeatureRow {
    pub schema_version: String,
    pub artifact_kind: String,
    pub source_id: String,
    pub token_id: String,
    pub condition_id: String,
    pub snapshot_ts: u64,
    pub ofi: Option<f64>,
    pub holder_herfindahl: Option<f64>,
    pub top_holder_share: Option<f64>,
    pub maker_herfindahl: Option<f64>,
    pub top_maker_share: Option<f64>,
    pub yes_no_residual: Option<f64>,
    pub negrisk_sum_residual: Option<f64>,
    pub distance_from_50: Option<f64>,
    pub realized_vol: Option<f64>,
    pub absent_features: Vec<String>,
    pub degraded: bool,
    pub status_code: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DerivedFeatureRun {
    pub input_path: PathBuf,
    pub row_path: PathBuf,
    pub input: DerivedFeatureInput,
    pub row: DerivedFeatureRow,
}

pub fn run_derived_feature_stage(
    input: &DerivedFeatureInput,
    output_root: &Path,
) -> Result<DerivedFeatureRun> {
    validate_input_shape(input)?;
    let input_path = write_json(
        output_root,
        &format!(
            "{}-derived-feature-input.json",
            safe_file_id(&input.source_id)
        ),
        input,
    )?;
    let readback: DerivedFeatureInput = read_json(&input_path)?;
    if readback != *input {
        return Err(readback_mismatch(format!(
            "derived feature input {} did not read back as written",
            input_path.display()
        )));
    }
    let row = compute_derived_feature_row(&readback)?;
    let row_path = write_json(
        output_root,
        &format!(
            "{}-derived-feature-row.json",
            safe_file_id(&input.source_id)
        ),
        &row,
    )?;
    let row_readback: DerivedFeatureRow = read_json(&row_path)?;
    if !rows_equivalent(&row_readback, &row) {
        return Err(readback_mismatch(format!(
            "derived feature row {} did not read back as written",
            row_path.display()
        )));
    }
    Ok(DerivedFeatureRun {
        input_path,
        row_path,
        input: readback,
        row: row_readback,
    })
}

pub fn compute_derived_feature_row(input: &DerivedFeatureInput) -> Result<DerivedFeatureRow> {
    validate_input_values(input)?;
    let mut absent = Vec::new();

    let ofi = match (input.buy_volume, input.sell_volume) {
        (Some(buy), Some(sell)) => features::order_flow_imbalance(buy, sell),
        _ => None,
    };
    push_absent(&mut absent, "ofi", ofi);

    let holder_herfindahl =
        non_empty(&input.holder_amounts).then(|| features::herfindahl(&input.holder_amounts));
    let top_holder_share =
        non_empty(&input.holder_amounts).then(|| features::top_share(&input.holder_amounts));
    push_absent(&mut absent, "holder_herfindahl", holder_herfindahl);
    push_absent(&mut absent, "top_holder_share", top_holder_share);

    let maker_herfindahl =
        non_empty(&input.maker_sizes).then(|| features::herfindahl(&input.maker_sizes));
    let top_maker_share =
        non_empty(&input.maker_sizes).then(|| features::top_share(&input.maker_sizes));
    push_absent(&mut absent, "maker_herfindahl", maker_herfindahl);
    push_absent(&mut absent, "top_maker_share", top_maker_share);

    let yes_no_residual = match (input.yes_price, input.no_price) {
        (Some(yes), Some(no)) => Some(features::yes_no_residual(yes, no)),
        _ => None,
    };
    push_absent(&mut absent, "yes_no_residual", yes_no_residual);

    let negrisk_sum_residual = non_empty(&input.negrisk_yes_prices)
        .then(|| features::negrisk_sum_residual(&input.negrisk_yes_prices));
    push_absent(&mut absent, "negrisk_sum_residual", negrisk_sum_residual);

    let distance_from_50 = input
        .price
        .or(input.mid)
        .and_then(features::distance_from_50);
    push_absent(&mut absent, "distance_from_50", distance_from_50);

    let realized_vol = features::realized_vol(&input.returns);
    push_absent(&mut absent, "realized_vol", realized_vol);

    let degraded = !absent.is_empty();
    Ok(DerivedFeatureRow {
        schema_version: DERIVED_FEATURE_SCHEMA_VERSION.to_string(),
        artifact_kind: DERIVED_FEATURE_ROW_ARTIFACT_KIND.to_string(),
        source_id: input.source_id.clone(),
        token_id: input.token_id.clone(),
        condition_id: input.condition_id.clone(),
        snapshot_ts: input.snapshot_ts,
        ofi,
        holder_herfindahl,
        top_holder_share,
        maker_herfindahl,
        top_maker_share,
        yes_no_residual,
        negrisk_sum_residual,
        distance_from_50,
        realized_vol,
        absent_features: absent,
        degraded,
        status_code: if degraded {
            DERIVED_FEATURE_DEGRADED
        } else {
            DERIVED_FEATURE_READY
        }
        .to_string(),
    })
}

fn validate_input_shape(input: &DerivedFeatureInput) -> Result<()> {
    if input.schema_version != DERIVED_FEATURE_SCHEMA_VERSION
        || input.artifact_kind != DERIVED_FEATURE_INPUT_ARTIFACT_KIND
    {
        return invalid_request("unexpected derived-feature input schema or artifact kind");
    }
    if input.source_id.trim().is_empty()
        || input.token_id.trim().is_empty()
        || input.condition_id.trim().is_empty()
    {
        return invalid_request("source_id, token_id, and condition_id are required");
    }
    Ok(())
}

fn validate_input_values(input: &DerivedFeatureInput) -> Result<()> {
    check_nonnegative_option("buy_volume", input.buy_volume)?;
    check_nonnegative_option("sell_volume", input.sell_volume)?;
    check_unit_option("yes_price", input.yes_price)?;
    check_unit_option("no_price", input.no_price)?;
    check_unit_option("price", input.price)?;
    check_unit_option("mid", input.mid)?;
    check_nonnegative_slice("holder_amounts", &input.holder_amounts)?;
    check_nonnegative_slice("maker_sizes", &input.maker_sizes)?;
    for (idx, value) in input.negrisk_yes_prices.iter().enumerate() {
        if !value.is_finite() || !(0.0..=1.0).contains(value) {
            return invalid_request(format!(
                "negrisk_yes_prices[{idx}] must be finite in [0, 1]"
            ));
        }
    }
    for (idx, value) in input.returns.iter().enumerate() {
        if !value.is_finite() {
            return invalid_request(format!("returns[{idx}] must be finite"));
        }
    }
    Ok(())
}

fn check_nonnegative_option(field: &str, value: Option<f64>) -> Result<()> {
    if let Some(value) = value
        && (!value.is_finite() || value < 0.0)
    {
        return invalid_request(format!(
            "{field} must be finite and non-negative when present"
        ));
    }
    Ok(())
}

fn check_unit_option(field: &str, value: Option<f64>) -> Result<()> {
    if let Some(value) = value
        && (!value.is_finite() || !(0.0..=1.0).contains(&value))
    {
        return invalid_request(format!("{field} must be finite in [0, 1] when present"));
    }
    Ok(())
}

fn check_nonnegative_slice(field: &str, values: &[f64]) -> Result<()> {
    for (idx, value) in values.iter().enumerate() {
        if !value.is_finite() || *value < 0.0 {
            return invalid_request(format!("{field}[{idx}] must be finite and non-negative"));
        }
    }
    Ok(())
}

fn push_absent(out: &mut Vec<String>, feature: &str, value: Option<f64>) {
    if value.is_none() {
        out.push(feature.to_string());
    }
}

fn rows_equivalent(left: &DerivedFeatureRow, right: &DerivedFeatureRow) -> bool {
    left.schema_version == right.schema_version
        && left.artifact_kind == right.artifact_kind
        && left.source_id == right.source_id
        && left.token_id == right.token_id
        && left.condition_id == right.condition_id
        && left.snapshot_ts == right.snapshot_ts
        && opt_close(left.ofi, right.ofi)
        && opt_close(left.holder_herfindahl, right.holder_herfindahl)
        && opt_close(left.top_holder_share, right.top_holder_share)
        && opt_close(left.maker_herfindahl, right.maker_herfindahl)
        && opt_close(left.top_maker_share, right.top_maker_share)
        && opt_close(left.yes_no_residual, right.yes_no_residual)
        && opt_close(left.negrisk_sum_residual, right.negrisk_sum_residual)
        && opt_close(left.distance_from_50, right.distance_from_50)
        && opt_close(left.realized_vol, right.realized_vol)
        && left.absent_features == right.absent_features
        && left.degraded == right.degraded
        && left.status_code == right.status_code
}

fn opt_close(left: Option<f64>, right: Option<f64>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => (left - right).abs() <= 1.0e-12,
        (None, None) => true,
        _ => false,
    }
}

fn non_empty(values: &[f64]) -> bool {
    !values.is_empty()
}

fn safe_file_id(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

fn invalid_request<T>(message: impl Into<String>) -> Result<T> {
    Err(PolyError::raw_source(ERR_INVALID_REQUEST, message.into()))
}

fn readback_mismatch(message: impl Into<String>) -> PolyError {
    PolyError::raw_source(ERR_READBACK_MISMATCH, message.into())
}
