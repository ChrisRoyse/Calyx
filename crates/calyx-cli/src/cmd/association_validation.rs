//! `calyx association-validation-gates` builds power-proven biomedical
//! association validation artifacts from persisted source evidence.

use std::path::PathBuf;

use serde::Serialize;

use super::discovery_run_preflight::{
    DiscoveryRunPreflightArgs, RUN_MANIFEST_FLAG, RUN_STAGE_ID_FLAG, preflight_input_sha256,
    sha256_hex,
};
use super::{Subcommand, value};
use crate::error::{CliError, CliResult};
use crate::output::print_json;

mod load;
mod metrics;
mod model;
#[cfg(test)]
mod tests;

use model::{AssociationValidationReport, GateParams};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AssociationValidationArgs {
    pub typed_root: PathBuf,
    pub open_targets_root: PathBuf,
    pub pubtator_root: PathBuf,
    pub clinicaltrials_root: PathBuf,
    pub dgidb_root: PathBuf,
    pub out_dir: PathBuf,
    pub cutoff_year: i32,
    pub score_threshold: f64,
    pub min_auroc: f64,
    pub min_positive_recall: f64,
    pub min_negative_suppression: f64,
    pub preflight: DiscoveryRunPreflightArgs,
}

impl Default for AssociationValidationArgs {
    fn default() -> Self {
        Self {
            typed_root: PathBuf::new(),
            open_targets_root: PathBuf::new(),
            pubtator_root: PathBuf::new(),
            clinicaltrials_root: PathBuf::new(),
            dgidb_root: PathBuf::new(),
            out_dir: PathBuf::new(),
            cutoff_year: 2016,
            score_threshold: 0.50,
            min_auroc: 0.70,
            min_positive_recall: 0.75,
            min_negative_suppression: 0.75,
            preflight: DiscoveryRunPreflightArgs::default(),
        }
    }
}

#[derive(Debug, Serialize)]
struct AssociationValidationCliSummary {
    status: &'static str,
    out_dir: String,
    report: String,
    report_sha256: String,
    gate_passed: bool,
    known_positive_count: usize,
    known_negative_count: usize,
    time_split_count: usize,
    mechanistic_direction_blocked_count: usize,
    scored_output_count: usize,
    readback: model::ReadbackSummary,
}

pub(crate) fn parse_association_validation_gates(rest: &[String]) -> CliResult<Subcommand> {
    let mut args = AssociationValidationArgs::default();
    let mut idx = 0;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--typed-root" => {
                idx += 1;
                args.typed_root = value(rest, idx, "--typed-root")?.into();
            }
            "--open-targets-root" => {
                idx += 1;
                args.open_targets_root = value(rest, idx, "--open-targets-root")?.into();
            }
            "--pubtator-root" => {
                idx += 1;
                args.pubtator_root = value(rest, idx, "--pubtator-root")?.into();
            }
            "--clinicaltrials-root" => {
                idx += 1;
                args.clinicaltrials_root = value(rest, idx, "--clinicaltrials-root")?.into();
            }
            "--dgidb-root" => {
                idx += 1;
                args.dgidb_root = value(rest, idx, "--dgidb-root")?.into();
            }
            "--out-dir" => {
                idx += 1;
                args.out_dir = value(rest, idx, "--out-dir")?.into();
            }
            "--cutoff-year" => {
                idx += 1;
                args.cutoff_year = parse_year(value(rest, idx, "--cutoff-year")?)?;
            }
            "--score-threshold" => {
                idx += 1;
                args.score_threshold = parse_unit(value(rest, idx, "--score-threshold")?)?;
            }
            "--min-auroc" => {
                idx += 1;
                args.min_auroc = parse_unit(value(rest, idx, "--min-auroc")?)?;
            }
            "--min-positive-recall" => {
                idx += 1;
                args.min_positive_recall = parse_unit(value(rest, idx, "--min-positive-recall")?)?;
            }
            "--min-negative-suppression" => {
                idx += 1;
                args.min_negative_suppression =
                    parse_unit(value(rest, idx, "--min-negative-suppression")?)?;
            }
            RUN_MANIFEST_FLAG => {
                idx += 1;
                args.preflight.manifest = Some(PathBuf::from(value(rest, idx, RUN_MANIFEST_FLAG)?));
            }
            RUN_STAGE_ID_FLAG => {
                idx += 1;
                args.preflight.stage_id = Some(value(rest, idx, RUN_STAGE_ID_FLAG)?.to_string());
            }
            other => {
                return Err(CliError::usage(format!(
                    "unexpected association-validation-gates flag {other}"
                )));
            }
        }
        idx += 1;
    }
    require_path(&args.typed_root, "--typed-root")?;
    require_path(&args.open_targets_root, "--open-targets-root")?;
    require_path(&args.pubtator_root, "--pubtator-root")?;
    require_path(&args.clinicaltrials_root, "--clinicaltrials-root")?;
    require_path(&args.dgidb_root, "--dgidb-root")?;
    require_path(&args.out_dir, "--out-dir")?;
    args.preflight
        .validate_for_command("association-validation-gates")?;
    Ok(Subcommand::AssociationValidationGates(args))
}

pub(crate) fn run(command: Subcommand) -> CliResult {
    let Subcommand::AssociationValidationGates(args) = command else {
        unreachable!("non-association-validation-gates command routed here");
    };
    let report = build_report(&args)?;
    let readback = model::persist_report_set(&args.out_dir, &report)?;
    if !report.gate_passed {
        return Err(CliError::runtime(format!(
            "association validation gates failed; report persisted at {}",
            readback.report.display()
        )));
    }
    print_json(&AssociationValidationCliSummary {
        status: "ok",
        out_dir: args.out_dir.display().to_string(),
        report: readback.report.display().to_string(),
        report_sha256: readback.report_sha256.clone(),
        gate_passed: report.gate_passed,
        known_positive_count: report.benchmark_counts.known_positive,
        known_negative_count: report.benchmark_counts.known_negative,
        time_split_count: report.benchmark_counts.time_split,
        mechanistic_direction_blocked_count: report
            .mechanistic_direction_counts
            .blocked_direction_rows,
        scored_output_count: report.scored_outputs.len(),
        readback,
    })
}

fn build_report(args: &AssociationValidationArgs) -> CliResult<AssociationValidationReport> {
    let params = GateParams {
        cutoff_year: args.cutoff_year,
        score_threshold: args.score_threshold,
        min_auroc: args.min_auroc,
        min_positive_recall: args.min_positive_recall,
        min_negative_suppression: args.min_negative_suppression,
    };
    let loaded = load::load_sources(args, &params)?;
    let manifest_bytes = serde_json::to_vec(&loaded.manifests)
        .map_err(|error| CliError::runtime(format!("serialize source manifests: {error}")))?;
    preflight_input_sha256(&args.preflight, sha256_hex(&manifest_bytes))?;
    metrics::score_and_report(params, loaded)
}

fn require_path(path: &std::path::Path, flag: &str) -> CliResult {
    if path.as_os_str().is_empty() {
        Err(CliError::usage(format!(
            "association-validation-gates requires {flag} <dir>"
        )))
    } else {
        Ok(())
    }
}

fn parse_year(raw: &str) -> CliResult<i32> {
    let year = raw
        .parse::<i32>()
        .map_err(|error| CliError::usage(format!("parse --cutoff-year {raw}: {error}")))?;
    if !(1900..=2100).contains(&year) {
        return Err(CliError::usage(
            "--cutoff-year must be a four digit year in [1900,2100]",
        ));
    }
    Ok(year)
}

fn parse_unit(raw: &str) -> CliResult<f64> {
    let value = raw
        .parse::<f64>()
        .map_err(|error| CliError::usage(format!("parse unit threshold {raw}: {error}")))?;
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(CliError::usage("thresholds must be finite and in [0,1]"));
    }
    Ok(value)
}
