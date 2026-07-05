//! Native deterministic bridges between biomedical discovery stage artifacts.

use std::collections::BTreeMap;
use std::path::PathBuf;

use super::discovery_run_preflight::{
    PreflightInput, RUN_MANIFEST_FLAG, RUN_STAGE_ID_FLAG, preflight_input_files,
};
use super::value;
use crate::error::{CliError, CliResult};

mod transform;
mod types;

use transform::{evaluation_input, rank_input};
use types::{
    EvaluateRankBridgeArgs, EvaluationArtifact, EvaluationBridgeOutput,
    FalsificationEvaluateBridgeArgs, FalsificationReport, MinerReport, RankBridgeOutput,
    bridge_error, eval_count, metadata, persist_bridge_output, print_bridge_summary, rank_count,
    read_source, require_path,
};

pub(crate) fn try_run(args: &[String]) -> Option<CliResult> {
    let (command, rest) = args.split_first()?;
    if matches!(rest, [flag] if flag == "--help" || flag == "-h") {
        return Some(crate::usage::print_command_usage(command));
    }
    match command.as_str() {
        "bridge-falsification-evaluate" => Some(
            parse_bridge_falsification_evaluate(rest).and_then(run_bridge_falsification_evaluate),
        ),
        "bridge-evaluate-rank" => {
            Some(parse_bridge_evaluate_rank(rest).and_then(run_bridge_evaluate_rank))
        }
        _ => None,
    }
}

pub(crate) fn run_bridge_falsification_evaluate(
    args: FalsificationEvaluateBridgeArgs,
) -> CliResult {
    let miner = read_source::<MinerReport>("miner_report", &args.miner_report)?;
    let falsification =
        read_source::<FalsificationReport>("falsification_report", &args.falsification_report)?;
    let preflight = preflight_input_files(
        &args.preflight,
        &[
            PreflightInput::new(&args.miner_report, &miner.bytes),
            PreflightInput::new(&args.falsification_report, &falsification.bytes),
        ],
    )?;
    let flag_by_id = falsification
        .value
        .hypothesis_flags
        .iter()
        .map(|flag| (flag.hypothesis_id.as_str(), flag))
        .collect::<BTreeMap<_, _>>();
    let mut evidence_by_id = BTreeMap::<&str, Vec<&types::EvidenceRow>>::new();
    for row in falsification
        .value
        .support_evidence
        .iter()
        .chain(falsification.value.counter_evidence.iter())
    {
        evidence_by_id
            .entry(row.hypothesis_id.as_str())
            .or_default()
            .push(row);
    }
    let mut inputs = Vec::new();
    let mut skipped_no_evidence = 0_usize;
    for hypothesis in &miner.value.hypotheses {
        let flag = flag_by_id
            .get(hypothesis.hypothesis_id.as_str())
            .ok_or_else(|| {
                bridge_error(format!(
                    "missing falsification flag for {}",
                    hypothesis.hypothesis_id
                ))
            })?;
        let evidence_rows = evidence_by_id
            .get(hypothesis.hypothesis_id.as_str())
            .cloned()
            .unwrap_or_default();
        if evidence_rows.is_empty() {
            skipped_no_evidence += 1;
            continue;
        }
        inputs.push(evaluation_input(
            hypothesis,
            flag,
            &evidence_rows,
            &miner.artifact,
            &falsification.artifact,
        )?);
    }
    if inputs.is_empty() {
        return Err(bridge_error("no evidence-backed hypotheses to bridge"));
    }
    let mut counts = BTreeMap::new();
    counts.insert("miner_hypotheses".to_string(), miner.value.hypotheses.len());
    counts.insert("bridged_inputs".to_string(), inputs.len());
    counts.insert("skipped_no_evidence".to_string(), skipped_no_evidence);
    counts.insert(
        "falsification_flags".to_string(),
        falsification.value.hypothesis_flags.len(),
    );
    let metadata = metadata(
        "bridge-falsification-evaluate",
        vec![miner.artifact, falsification.artifact],
        counts,
    );
    let output = EvaluationBridgeOutput {
        schema_version: types::BRIDGE_SCHEMA_VERSION,
        bridge_metadata: metadata.clone(),
        inputs,
    };
    let persisted = persist_bridge_output(&args.out, &output, &metadata, preflight, eval_count)?;
    print_bridge_summary(&persisted)
}

pub(crate) fn run_bridge_evaluate_rank(args: EvaluateRankBridgeArgs) -> CliResult {
    let evaluation =
        read_source::<EvaluationArtifact>("evaluation_report", &args.evaluation_report)?;
    let preflight = preflight_input_files(
        &args.preflight,
        &[PreflightInput::new(
            &args.evaluation_report,
            &evaluation.bytes,
        )],
    )?;
    let mut inputs = Vec::new();
    let mut skipped_not_retained = 0_usize;
    for evaluation in &evaluation.value.report.evaluations {
        if evaluation.verdict != calyx_lodestar::HypothesisEvaluationVerdict::RetainForRanking {
            skipped_not_retained += 1;
            continue;
        }
        inputs.push(rank_input(evaluation)?);
    }
    if inputs.is_empty() {
        return Err(bridge_error(
            "no retained evaluations to bridge into ranking",
        ));
    }
    let mut counts = BTreeMap::new();
    counts.insert(
        "evaluations".to_string(),
        evaluation.value.report.evaluations.len(),
    );
    counts.insert("bridged_inputs".to_string(), inputs.len());
    counts.insert("skipped_not_retained".to_string(), skipped_not_retained);
    let metadata = metadata("bridge-evaluate-rank", vec![evaluation.artifact], counts);
    let output = RankBridgeOutput {
        schema_version: types::BRIDGE_SCHEMA_VERSION,
        bridge_metadata: metadata.clone(),
        inputs,
    };
    let persisted = persist_bridge_output(&args.out, &output, &metadata, preflight, rank_count)?;
    print_bridge_summary(&persisted)
}

pub(crate) fn parse_bridge_falsification_evaluate(
    rest: &[String],
) -> CliResult<FalsificationEvaluateBridgeArgs> {
    let mut args = FalsificationEvaluateBridgeArgs::default();
    let mut idx = 0;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--miner-report" => {
                idx += 1;
                args.miner_report = PathBuf::from(value(rest, idx, "--miner-report")?);
            }
            "--falsification-report" => {
                idx += 1;
                args.falsification_report =
                    PathBuf::from(value(rest, idx, "--falsification-report")?);
            }
            "--out" => {
                idx += 1;
                args.out = PathBuf::from(value(rest, idx, "--out")?);
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
                    "unexpected bridge-falsification-evaluate flag {other}"
                )));
            }
        }
        idx += 1;
    }
    require_path(
        &args.miner_report,
        "bridge-falsification-evaluate",
        "--miner-report",
    )?;
    require_path(
        &args.falsification_report,
        "bridge-falsification-evaluate",
        "--falsification-report",
    )?;
    require_path(&args.out, "bridge-falsification-evaluate", "--out")?;
    args.preflight
        .validate_for_command("bridge-falsification-evaluate")?;
    Ok(args)
}

pub(crate) fn parse_bridge_evaluate_rank(rest: &[String]) -> CliResult<EvaluateRankBridgeArgs> {
    let mut args = EvaluateRankBridgeArgs::default();
    let mut idx = 0;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--evaluation-report" => {
                idx += 1;
                args.evaluation_report = PathBuf::from(value(rest, idx, "--evaluation-report")?);
            }
            "--out" => {
                idx += 1;
                args.out = PathBuf::from(value(rest, idx, "--out")?);
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
                    "unexpected bridge-evaluate-rank flag {other}"
                )));
            }
        }
        idx += 1;
    }
    require_path(
        &args.evaluation_report,
        "bridge-evaluate-rank",
        "--evaluation-report",
    )?;
    require_path(&args.out, "bridge-evaluate-rank", "--out")?;
    args.preflight
        .validate_for_command("bridge-evaluate-rank")?;
    Ok(args)
}

#[cfg(test)]
mod tests;
