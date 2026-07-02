//! `calyx hypothesis-evaluate` -- aggregate grounded hypothesis evaluator runs (#881).

use std::fs;
use std::path::{Path, PathBuf};

use calyx_lodestar::{
    HypothesisEvaluationInput, HypothesisEvaluationParams, HypothesisEvaluationReport,
    aggregate_hypothesis_evaluations,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::value;
use crate::error::{CliError, CliResult};
use crate::output::print_json;

const HYPOTHESIS_EVALUATE_ARTIFACT_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct HypothesisEvaluateArgs {
    pub input: PathBuf,
    pub out: PathBuf,
    pub min_runs_per_hypothesis: usize,
    pub min_prompt_variants: usize,
    pub min_temperature_variants: usize,
    pub min_retrieved_evidence: usize,
    pub retain_score_floor: f32,
    pub max_ranked: usize,
}

impl Default for HypothesisEvaluateArgs {
    fn default() -> Self {
        let params = HypothesisEvaluationParams::default();
        Self {
            input: PathBuf::new(),
            out: PathBuf::new(),
            min_runs_per_hypothesis: params.min_runs_per_hypothesis,
            min_prompt_variants: params.min_prompt_variants,
            min_temperature_variants: params.min_temperature_variants,
            min_retrieved_evidence: params.min_retrieved_evidence,
            retain_score_floor: params.retain_score_floor,
            max_ranked: params.max_ranked,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct HypothesisEvaluateInputFile {
    inputs: Vec<HypothesisEvaluationInput>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct HypothesisEvaluateArtifact {
    schema_version: u32,
    params: HypothesisEvaluationParams,
    source_input: String,
    source_input_bytes: u64,
    source_input_sha256: String,
    report: HypothesisEvaluationReport,
}

struct PersistedEvaluation {
    path: PathBuf,
    bytes: u64,
    sha256: String,
    readback_input_count: usize,
    readback_retained_count: usize,
    readback_needs_more_evidence_count: usize,
    readback_rejected_count: usize,
    readback_evaluation_count: usize,
}

pub(crate) fn try_run(args: &[String]) -> Option<CliResult> {
    let (command, rest) = args.split_first()?;
    if command != "hypothesis-evaluate" {
        return None;
    }
    Some(parse_hypothesis_evaluate(rest).and_then(run_hypothesis_evaluate))
}

pub(crate) fn run_hypothesis_evaluate(args: HypothesisEvaluateArgs) -> CliResult {
    let input_bytes = fs::read(&args.input)
        .map_err(|error| CliError::io(format!("read --input {}: {error}", args.input.display())))?;
    let input_file: HypothesisEvaluateInputFile = serde_json::from_slice(&input_bytes)?;
    if input_file.inputs.is_empty() {
        return Err(CliError::usage(format!(
            "--input {} did not contain any hypothesis inputs",
            args.input.display()
        )));
    }
    let params = args.params();
    let report = aggregate_hypothesis_evaluations(&input_file.inputs, &params)?;
    let artifact = HypothesisEvaluateArtifact {
        schema_version: HYPOTHESIS_EVALUATE_ARTIFACT_SCHEMA_VERSION,
        params,
        source_input: args.input.display().to_string(),
        source_input_bytes: input_bytes.len() as u64,
        source_input_sha256: sha256_hex(&input_bytes),
        report,
    };
    let persisted = persist_evaluation(&args.out, &artifact)?;
    print_json(&json!({
        "status": "ok",
        "input": args.input,
        "input_bytes": artifact.source_input_bytes,
        "input_sha256": artifact.source_input_sha256,
        "out": persisted.path,
        "out_bytes": persisted.bytes,
        "out_sha256": persisted.sha256,
        "readback": {
            "input_count": persisted.readback_input_count,
            "evaluation_count": persisted.readback_evaluation_count,
            "retained_count": persisted.readback_retained_count,
            "needs_more_evidence_count": persisted.readback_needs_more_evidence_count,
            "rejected_count": persisted.readback_rejected_count,
        }
    }))
}

fn parse_hypothesis_evaluate(rest: &[String]) -> CliResult<HypothesisEvaluateArgs> {
    let mut args = HypothesisEvaluateArgs::default();
    let mut idx = 0;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--input" => {
                idx += 1;
                args.input = PathBuf::from(value(rest, idx, "--input")?);
            }
            "--out" => {
                idx += 1;
                args.out = PathBuf::from(value(rest, idx, "--out")?);
            }
            "--min-runs-per-hypothesis" => {
                idx += 1;
                args.min_runs_per_hypothesis = parse_usize(
                    value(rest, idx, "--min-runs-per-hypothesis")?,
                    "--min-runs-per-hypothesis",
                    1,
                )?;
            }
            "--min-prompt-variants" => {
                idx += 1;
                args.min_prompt_variants = parse_usize(
                    value(rest, idx, "--min-prompt-variants")?,
                    "--min-prompt-variants",
                    1,
                )?;
            }
            "--min-temperature-variants" => {
                idx += 1;
                args.min_temperature_variants = parse_usize(
                    value(rest, idx, "--min-temperature-variants")?,
                    "--min-temperature-variants",
                    1,
                )?;
            }
            "--min-retrieved-evidence" => {
                idx += 1;
                args.min_retrieved_evidence = parse_usize(
                    value(rest, idx, "--min-retrieved-evidence")?,
                    "--min-retrieved-evidence",
                    0,
                )?;
            }
            "--retain-score-floor" => {
                idx += 1;
                args.retain_score_floor = parse_unit(
                    value(rest, idx, "--retain-score-floor")?,
                    "--retain-score-floor",
                )?;
            }
            "--max-ranked" => {
                idx += 1;
                args.max_ranked =
                    parse_usize(value(rest, idx, "--max-ranked")?, "--max-ranked", 1)?;
            }
            other => {
                return Err(CliError::usage(format!(
                    "unexpected hypothesis-evaluate flag {other}"
                )));
            }
        }
        idx += 1;
    }
    if args.input.as_os_str().is_empty() {
        return Err(CliError::usage(
            "hypothesis-evaluate requires --input <json>",
        ));
    }
    if args.out.as_os_str().is_empty() {
        return Err(CliError::usage("hypothesis-evaluate requires --out <json>"));
    }
    Ok(args)
}

impl HypothesisEvaluateArgs {
    fn params(&self) -> HypothesisEvaluationParams {
        HypothesisEvaluationParams {
            min_runs_per_hypothesis: self.min_runs_per_hypothesis,
            min_prompt_variants: self.min_prompt_variants,
            min_temperature_variants: self.min_temperature_variants,
            min_retrieved_evidence: self.min_retrieved_evidence,
            retain_score_floor: self.retain_score_floor,
            max_ranked: self.max_ranked,
        }
    }
}

fn persist_evaluation(
    path: &Path,
    artifact: &HypothesisEvaluateArtifact,
) -> CliResult<PersistedEvaluation> {
    let bytes = serde_json::to_vec_pretty(artifact)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if path.exists() {
        let existing = fs::read(path)?;
        if existing != bytes {
            return Err(CliError::usage(format!(
                "refusing to overwrite existing different hypothesis evaluation report {}",
                path.display()
            )));
        }
    } else {
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, &bytes)?;
        fs::rename(&tmp, path)?;
    }
    let readback = fs::read(path)?;
    if readback != bytes {
        return Err(CliError::usage(format!(
            "hypothesis evaluation report readback mismatch at {}",
            path.display()
        )));
    }
    let decoded: HypothesisEvaluateArtifact = serde_json::from_slice(&readback)?;
    Ok(PersistedEvaluation {
        path: path.to_path_buf(),
        bytes: readback.len() as u64,
        sha256: sha256_hex(&readback),
        readback_input_count: decoded.report.input_count,
        readback_retained_count: decoded.report.retained_count,
        readback_needs_more_evidence_count: decoded.report.needs_more_evidence_count,
        readback_rejected_count: decoded.report.rejected_count,
        readback_evaluation_count: decoded.report.evaluations.len(),
    })
}

fn parse_usize(raw: &str, flag: &str, min: usize) -> CliResult<usize> {
    let value = raw
        .parse::<usize>()
        .map_err(|err| CliError::usage(format!("parse {flag} {raw}: {err}")))?;
    if value < min {
        return Err(CliError::usage(format!("{flag} must be >= {min}")));
    }
    Ok(value)
}

fn parse_unit(raw: &str, flag: &str) -> CliResult<f32> {
    let value = raw
        .parse::<f32>()
        .map_err(|err| CliError::usage(format!("parse {flag} {raw}: {err}")))?;
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(CliError::usage(format!(
            "{flag} must be finite and in [0,1]"
        )));
    }
    Ok(value)
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex_lower(&Sha256::digest(bytes))
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_requires_input_and_output() {
        let err = parse_hypothesis_evaluate(&["--input".to_string(), "target/in.json".to_string()])
            .unwrap_err();
        assert!(err.to_string().contains("--out"));
    }

    #[test]
    fn parse_accepts_all_thresholds() {
        let args = parse_hypothesis_evaluate(&[
            "--input".to_string(),
            "target/in.json".to_string(),
            "--out".to_string(),
            "target/out.json".to_string(),
            "--min-runs-per-hypothesis".to_string(),
            "3".to_string(),
            "--min-prompt-variants".to_string(),
            "2".to_string(),
            "--min-temperature-variants".to_string(),
            "2".to_string(),
            "--min-retrieved-evidence".to_string(),
            "2".to_string(),
            "--retain-score-floor".to_string(),
            "0.7".to_string(),
            "--max-ranked".to_string(),
            "12".to_string(),
        ])
        .unwrap();
        assert_eq!(args.input, PathBuf::from("target/in.json"));
        assert_eq!(args.out, PathBuf::from("target/out.json"));
        assert_eq!(args.min_runs_per_hypothesis, 3);
        assert_eq!(args.min_retrieved_evidence, 2);
        assert_eq!(args.retain_score_floor, 0.7);
        assert_eq!(args.max_ranked, 12);
    }
}
