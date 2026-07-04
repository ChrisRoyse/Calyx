//! `calyx hypothesis-falsification-sweep` flags mined hypotheses with counter-evidence.

use std::path::PathBuf;

mod load;
mod matching;
mod model;
mod persist;
#[cfg(test)]
mod tests;

pub(crate) use model::HypothesisFalsificationArgs;
use model::{FalsificationReport, FalsificationSummary};

use super::discovery_run_preflight::{RUN_MANIFEST_FLAG, RUN_STAGE_ID_FLAG};
use super::{Subcommand, value};
use crate::error::{CliError, CliResult};
use crate::output::print_json;

const SCHEMA_VERSION: u32 = 2;

pub(crate) fn parse_hypothesis_falsification_sweep(rest: &[String]) -> CliResult<Subcommand> {
    let mut args = HypothesisFalsificationArgs::default();
    let mut idx = 0;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--hypotheses-report" => {
                idx += 1;
                args.hypotheses_reports.push(PathBuf::from(value(
                    rest,
                    idx,
                    "--hypotheses-report",
                )?));
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
            "--open-targets-root" => {
                idx += 1;
                args.open_targets_root = value(rest, idx, "--open-targets-root")?.into();
            }
            "--out-dir" => {
                idx += 1;
                args.out_dir = value(rest, idx, "--out-dir")?.into();
            }
            "--max-hypotheses" => {
                idx += 1;
                args.max_hypotheses =
                    parse_usize(value(rest, idx, "--max-hypotheses")?, 1, "--max-hypotheses")?;
            }
            RUN_MANIFEST_FLAG => {
                idx += 1;
                args.preflight.manifest = Some(value(rest, idx, RUN_MANIFEST_FLAG)?.into());
            }
            RUN_STAGE_ID_FLAG => {
                idx += 1;
                args.preflight.stage_id = Some(value(rest, idx, RUN_STAGE_ID_FLAG)?.to_string());
            }
            other => {
                return Err(CliError::usage(format!(
                    "unexpected hypothesis-falsification-sweep flag {other}"
                )));
            }
        }
        idx += 1;
    }
    require(!args.hypotheses_reports.is_empty(), "--hypotheses-report")?;
    require(
        !args.pubtator_root.as_os_str().is_empty(),
        "--pubtator-root",
    )?;
    require(
        !args.clinicaltrials_root.as_os_str().is_empty(),
        "--clinicaltrials-root",
    )?;
    require(!args.dgidb_root.as_os_str().is_empty(), "--dgidb-root")?;
    require(
        !args.open_targets_root.as_os_str().is_empty(),
        "--open-targets-root",
    )?;
    require(!args.out_dir.as_os_str().is_empty(), "--out-dir")?;
    if args.max_hypotheses > 20_000 {
        return Err(CliError::usage(
            "hypothesis-falsification-sweep --max-hypotheses exceeds hard safety limit",
        ));
    }
    args.preflight
        .validate_for_command("hypothesis-falsification-sweep")?;
    Ok(Subcommand::HypothesisFalsificationSweep(args))
}

pub(crate) fn run(command: Subcommand) -> CliResult {
    let Subcommand::HypothesisFalsificationSweep(args) = command else {
        unreachable!("non-hypothesis-falsification-sweep command routed here");
    };
    let report = build_report(&args)?;
    let readback = persist::persist(&args.out_dir, &report)?;
    print_json(&FalsificationSummary {
        status: "ok",
        out_dir: args.out_dir.display().to_string(),
        report: readback.report.display().to_string(),
        report_sha256: readback.report_sha256,
        support_evidence_jsonl: readback.support_evidence.display().to_string(),
        support_evidence_sha256: readback.support_evidence_sha256,
        counter_evidence_jsonl: readback.counter_evidence.display().to_string(),
        counter_evidence_sha256: readback.counter_evidence_sha256,
        skipped_evidence_jsonl: readback.skipped_evidence.display().to_string(),
        skipped_evidence_sha256: readback.skipped_evidence_sha256,
        hypothesis_flags_jsonl: readback.hypothesis_flags.display().to_string(),
        hypothesis_flags_sha256: readback.hypothesis_flags_sha256,
        raw_query_manifest_jsonl: readback.raw_query_manifest.display().to_string(),
        raw_query_manifest_sha256: readback.raw_query_manifest_sha256,
        input_hypothesis_count: report.input_hypothesis_count,
        deduped_hypothesis_count: report.deduped_hypothesis_count,
        support_evidence_count: report.support_evidence_count,
        counter_evidence_count: report.counter_evidence_count,
        skipped_evidence_count: report.skipped_evidence_count,
        flagged_with_counter_evidence_count: report.flagged_with_counter_evidence_count,
        readback_flag_count: readback.flag_count,
    })
}

fn build_report(args: &HypothesisFalsificationArgs) -> CliResult<FalsificationReport> {
    let loaded = load::load_hypotheses(args)?;
    let mut hypotheses = loaded.hypotheses;
    if hypotheses.len() > args.max_hypotheses {
        return Err(CliError::runtime(format!(
            "hypothesis count {} exceeds --max-hypotheses {}",
            hypotheses.len(),
            args.max_hypotheses
        )));
    }
    hypotheses.sort_by(|a, b| a.hypothesis_id.cmp(&b.hypothesis_id));
    let sources = load::load_sources(args, &hypotheses)?;
    let flags = load::flag_hypotheses(&hypotheses, &sources);
    let support_evidence_count = sources.support_evidence.len();
    let counter_evidence_count = sources.counter_evidence.len();
    let skipped_evidence_count = sources.skipped_evidence.len();
    let flagged_with_counter_evidence_count = flags
        .iter()
        .filter(|flag| flag.counter_evidence_count > 0)
        .count();
    Ok(FalsificationReport {
        schema_version: SCHEMA_VERSION,
        status: "ok".to_string(),
        hypotheses_reports: args
            .hypotheses_reports
            .iter()
            .map(|path| path.display().to_string())
            .collect(),
        pubtator_root: args.pubtator_root.display().to_string(),
        clinicaltrials_root: args.clinicaltrials_root.display().to_string(),
        dgidb_root: args.dgidb_root.display().to_string(),
        open_targets_root: args.open_targets_root.display().to_string(),
        input_hypothesis_count: loaded.input_count,
        deduped_hypothesis_count: hypotheses.len(),
        raw_query_manifest_count: sources.raw_query_manifest.len(),
        support_evidence_count,
        counter_evidence_count,
        skipped_evidence_count,
        flagged_with_counter_evidence_count,
        hypothesis_flags: flags,
        support_evidence: sources.support_evidence,
        counter_evidence: sources.counter_evidence,
        skipped_evidence: sources.skipped_evidence,
        raw_query_manifest: sources.raw_query_manifest,
    })
}

fn parse_usize(raw: &str, min: usize, flag: &str) -> CliResult<usize> {
    let value = raw
        .parse::<usize>()
        .map_err(|error| CliError::usage(format!("parse {flag} {raw}: {error}")))?;
    if value < min {
        return Err(CliError::usage(format!("{flag} must be >= {min}")));
    }
    Ok(value)
}

fn require(ok: bool, flag: &str) -> CliResult {
    if ok {
        Ok(())
    } else {
        Err(CliError::usage(format!(
            "hypothesis-falsification-sweep requires {flag}"
        )))
    }
}
