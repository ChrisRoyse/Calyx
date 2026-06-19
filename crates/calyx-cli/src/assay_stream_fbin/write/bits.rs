use std::collections::BTreeMap;
use std::fs;

use serde::Deserialize;

use crate::error::CliResult;

use super::super::args::Args;
use super::super::{io_error, local_error};

#[derive(Clone, Debug, Deserialize)]
struct BitsReport {
    lenses: Option<Vec<BitsLens>>,
    report: Option<BitsReportInner>,
}

#[derive(Clone, Debug, Deserialize)]
struct BitsReportInner {
    lenses: Vec<BitsLens>,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct BitsLens {
    pub(super) name: String,
    pub(super) bits_about: f32,
    pub(super) admitted: bool,
}

pub(super) fn load_bits(args: &Args) -> CliResult<BTreeMap<String, BitsLens>> {
    let report: BitsReport = serde_json::from_slice(
        &fs::read(&args.bits_report).map_err(io_error)?,
    )
    .map_err(|error| {
        local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_BITS_INVALID",
            format!("parse {} failed: {error}", args.bits_report.display()),
            "pass assay_abundance.json or full bits-validate evidence",
        )
    })?;
    let lenses = report
        .lenses
        .or_else(|| report.report.map(|inner| inner.lenses))
        .ok_or_else(|| {
            local_error(
                "CALYX_FSV_ASSAY_STREAM_FBIN_BITS_INVALID",
                "bits report missing lenses",
                "pass a bits report with per-lens bits_about",
            )
        })?;
    Ok(lenses
        .into_iter()
        .map(|lens| (lens.name.clone(), lens))
        .collect())
}
