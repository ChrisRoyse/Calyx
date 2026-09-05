//! Flow-to-price transfer entropy wiring for Poly issue #52.
//!
//! This module does not estimate transfer entropy itself. It validates Poly's
//! local on-chain-flow and price series, calls the real `calyx-assay`
//! transfer-entropy lag sweep, persists the selected directional association,
//! and reads the report back before returning success.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use calyx_assay::{
    Direction, TEResult, Timestamp, TransferEntropyConfig, transfer_entropy_sweep_with_config,
};
use calyx_core::Clock;
use serde::{Deserialize, Serialize};

use crate::error::{PolyError, Result};

pub const FLOW_PRICE_TE_SCHEMA_VERSION: &str = "poly.flow_price_transfer_entropy.v1";
pub const FLOW_PRICE_TE_ARTIFACT_KIND: &str = "poly_flow_price_transfer_entropy";
pub const ERR_FLOW_PRICE_TE_INVALID_REQUEST: &str = "CALYX_POLY_FLOW_PRICE_TE_INVALID_REQUEST";
pub const ERR_FLOW_PRICE_TE_NO_DIRECTIONAL_SIGNAL: &str =
    "CALYX_POLY_FLOW_PRICE_TE_NO_DIRECTIONAL_SIGNAL";
pub const ERR_FLOW_PRICE_TE_READBACK_MISMATCH: &str = "CALYX_POLY_FLOW_PRICE_TE_READBACK_MISMATCH";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FlowPricePoint {
    pub ts: Timestamp,
    pub value: f32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowPriceTransferEntropyConfig {
    pub window_size: usize,
    pub k: usize,
    pub bootstrap_resamples: usize,
    pub bootstrap_seed: u64,
}

impl From<FlowPriceTransferEntropyConfig> for TransferEntropyConfig {
    fn from(config: FlowPriceTransferEntropyConfig) -> Self {
        Self {
            window_size: config.window_size,
            k: config.k,
            bootstrap_resamples: config.bootstrap_resamples,
            bootstrap_seed: config.bootstrap_seed,
        }
    }
}

impl Default for FlowPriceTransferEntropyConfig {
    fn default() -> Self {
        let assay = TransferEntropyConfig::default();
        Self {
            window_size: assay.window_size,
            k: assay.k,
            bootstrap_resamples: assay.bootstrap_resamples,
            bootstrap_seed: assay.bootstrap_seed,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FlowPriceTransferEntropyRequest {
    pub domain: String,
    pub market_id: String,
    pub flow_source: String,
    pub price_source: String,
    pub flow_series: Vec<FlowPricePoint>,
    pub price_series: Vec<FlowPricePoint>,
    pub candidate_lags: Vec<usize>,
    pub config: FlowPriceTransferEntropyConfig,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FlowPriceTransferEntropyReport {
    pub schema_version: String,
    pub artifact_kind: String,
    pub domain: String,
    pub market_id: String,
    pub flow_source: String,
    pub price_source: String,
    pub flow_sample_count: usize,
    pub price_sample_count: usize,
    pub candidate_lags: Vec<usize>,
    pub selected_lag: usize,
    pub selected_direction: Direction,
    pub selected_flow_to_price_te: f32,
    pub selected_price_to_flow_te: f32,
    pub selected_difference: f32,
    pub selected_difference_ci_95: (f32, f32),
    pub selected_n_samples: usize,
    pub config: FlowPriceTransferEntropyConfig,
    pub sweep: Vec<TEResult>,
    pub sweep_order: Vec<usize>,
    pub association_source: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FlowPriceTransferEntropyRun {
    pub report_path: PathBuf,
    pub report: FlowPriceTransferEntropyReport,
}

pub fn run_flow_price_transfer_entropy(
    request: FlowPriceTransferEntropyRequest,
    output_dir: &Path,
    clock: &dyn Clock,
) -> Result<FlowPriceTransferEntropyRun> {
    validate_request(&request)?;
    let flow = to_stream("flow", &request.flow_series)?;
    let price = to_stream("price", &request.price_series)?;
    let assay_config = TransferEntropyConfig::from(request.config.clone());
    let sweep = transfer_entropy_sweep_with_config(
        &flow,
        &price,
        &request.candidate_lags,
        clock,
        &assay_config,
    );
    let selected = select_flow_to_price(&sweep)?;
    let report = FlowPriceTransferEntropyReport {
        schema_version: FLOW_PRICE_TE_SCHEMA_VERSION.to_string(),
        artifact_kind: FLOW_PRICE_TE_ARTIFACT_KIND.to_string(),
        domain: request.domain,
        market_id: request.market_id,
        flow_source: request.flow_source,
        price_source: request.price_source,
        flow_sample_count: flow.len(),
        price_sample_count: price.len(),
        candidate_lags: request.candidate_lags,
        selected_lag: selected.lag,
        selected_direction: selected.dominant_direction,
        selected_flow_to_price_te: selected.t_a_to_b,
        selected_price_to_flow_te: selected.t_b_to_a,
        selected_difference: selected.t_a_to_b - selected.t_b_to_a,
        selected_difference_ci_95: selected.difference_ci_95,
        selected_n_samples: selected.n_samples,
        config: request.config,
        sweep_order: sweep.iter().map(|result| result.lag).collect(),
        sweep,
        association_source:
            "calyx_poly::flow_price_transfer_entropy/calyx_assay::transfer_entropy_sweep_with_config"
                .to_string(),
    };
    let report_path = write_flow_price_transfer_entropy_report(output_dir, &report)?;
    let readback = read_flow_price_transfer_entropy_report(&report_path)?;
    if readback != report {
        return Err(PolyError::diagnostics(
            ERR_FLOW_PRICE_TE_READBACK_MISMATCH,
            format!(
                "flow-price TE report readback mismatch at {}",
                report_path.display()
            ),
        ));
    }
    Ok(FlowPriceTransferEntropyRun {
        report_path,
        report,
    })
}

pub fn write_flow_price_transfer_entropy_report(
    dir: &Path,
    report: &FlowPriceTransferEntropyReport,
) -> Result<PathBuf> {
    let file_name = format!(
        "flow_price_transfer_entropy_{}_{}.json",
        sanitize(&report.domain),
        sanitize(&report.market_id)
    );
    crate::diagnostics_store::write_json(dir, &file_name, report)
}

pub fn read_flow_price_transfer_entropy_report(
    path: &Path,
) -> Result<FlowPriceTransferEntropyReport> {
    crate::diagnostics_store::read_json(path)
}

fn validate_request(request: &FlowPriceTransferEntropyRequest) -> Result<()> {
    if request.domain.trim().is_empty()
        || request.market_id.trim().is_empty()
        || request.flow_source.trim().is_empty()
        || request.price_source.trim().is_empty()
    {
        return invalid("domain, market_id, flow_source, and price_source are required");
    }
    if request.candidate_lags.is_empty() {
        return invalid("at least one positive candidate lag is required");
    }
    let mut seen = BTreeSet::new();
    for lag in &request.candidate_lags {
        if *lag == 0 {
            return invalid("candidate lags must be positive");
        }
        if !seen.insert(*lag) {
            return invalid(format!("duplicate candidate lag {lag}"));
        }
    }
    if request.config.window_size == 0
        || request.config.k == 0
        || request.config.bootstrap_resamples == 0
    {
        return invalid("window_size, k, and bootstrap_resamples must be positive");
    }
    Ok(())
}

fn to_stream(name: &'static str, points: &[FlowPricePoint]) -> Result<Vec<(Timestamp, f32)>> {
    if points.len() < 2 {
        return invalid(format!("{name} series requires at least two samples"));
    }
    let mut seen = BTreeSet::new();
    let mut stream = Vec::with_capacity(points.len());
    for (index, point) in points.iter().enumerate() {
        if !point.value.is_finite() {
            return invalid(format!(
                "{name} sample {index} at ts {} has non-finite value",
                point.ts
            ));
        }
        if !seen.insert(point.ts) {
            return invalid(format!(
                "{name} series has duplicate timestamp {}",
                point.ts
            ));
        }
        stream.push((point.ts, point.value));
    }
    stream.sort_by_key(|(ts, _)| *ts);
    Ok(stream)
}

fn select_flow_to_price(sweep: &[TEResult]) -> Result<&TEResult> {
    sweep
        .iter()
        .filter(|result| {
            !result.provisional
                && result.error_code.is_none()
                && result.dominant_direction == Direction::AToB
                && result.t_a_to_b > result.t_b_to_a
        })
        .max_by(|left, right| {
            let left_margin = left.t_a_to_b - left.t_b_to_a;
            let right_margin = right.t_a_to_b - right.t_b_to_a;
            left_margin.total_cmp(&right_margin)
        })
        .ok_or_else(|| {
            PolyError::diagnostics(
                ERR_FLOW_PRICE_TE_NO_DIRECTIONAL_SIGNAL,
                "Assay transfer-entropy sweep found no decisive flow -> price signal",
            )
        })
}

fn invalid<T>(message: impl Into<String>) -> Result<T> {
    Err(PolyError::diagnostics(
        ERR_FLOW_PRICE_TE_INVALID_REQUEST,
        message,
    ))
}

fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}
