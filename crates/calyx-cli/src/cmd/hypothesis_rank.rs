//! `calyx hypothesis-rank` -- rank traceable evaluated hypotheses (#882).

use std::fs;
use std::path::{Path, PathBuf};

use calyx_lodestar::{
    RankedHypothesisParams, RankedHypothesisReport, TraceableHypothesisInput,
    rank_traceable_hypotheses,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::value;
use crate::error::{CliError, CliResult};
use crate::output::print_json;

const HYPOTHESIS_RANK_ARTIFACT_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct HypothesisRankArgs {
    pub input: PathBuf,
    pub out: PathBuf,
    pub max_ranked: usize,
    pub review_top_n: usize,
    pub min_review_score: f32,
}

impl Default for HypothesisRankArgs {
    fn default() -> Self {
        let params = RankedHypothesisParams::default();
        Self {
            input: PathBuf::new(),
            out: PathBuf::new(),
            max_ranked: params.max_ranked,
            review_top_n: params.review_top_n,
            min_review_score: params.min_review_score,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct HypothesisRankInputFile {
    inputs: Vec<TraceableHypothesisInput>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct HypothesisRankArtifact {
    schema_version: u32,
    params: RankedHypothesisParams,
    source_input: String,
    source_input_bytes: u64,
    source_input_sha256: String,
    report: RankedHypothesisReport,
}

struct PersistedRanking {
    path: PathBuf,
    bytes: u64,
    sha256: String,
    readback_input_count: usize,
    readback_ranked_count: usize,
    readback_human_review_count: usize,
    readback_top_hypothesis_id: String,
    readback_top_rank_score: f32,
}

pub(crate) fn try_run(args: &[String]) -> Option<CliResult> {
    let (command, rest) = args.split_first()?;
    if command != "hypothesis-rank" {
        return None;
    }
    Some(parse_hypothesis_rank(rest).and_then(run_hypothesis_rank))
}

pub(crate) fn run_hypothesis_rank(args: HypothesisRankArgs) -> CliResult {
    let input_bytes = fs::read(&args.input)
        .map_err(|error| CliError::io(format!("read --input {}: {error}", args.input.display())))?;
    let input_file: HypothesisRankInputFile = serde_json::from_slice(&input_bytes)?;
    if input_file.inputs.is_empty() {
        return Err(CliError::usage(format!(
            "--input {} did not contain any traceable hypothesis inputs",
            args.input.display()
        )));
    }
    let params = args.params();
    let report = rank_traceable_hypotheses(&input_file.inputs, &params)?;
    let artifact = HypothesisRankArtifact {
        schema_version: HYPOTHESIS_RANK_ARTIFACT_SCHEMA_VERSION,
        params,
        source_input: args.input.display().to_string(),
        source_input_bytes: input_bytes.len() as u64,
        source_input_sha256: sha256_hex(&input_bytes),
        report,
    };
    let persisted = persist_ranking(&args.out, &artifact)?;
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
            "ranked_count": persisted.readback_ranked_count,
            "human_review_count": persisted.readback_human_review_count,
            "top_hypothesis_id": persisted.readback_top_hypothesis_id,
            "top_rank_score": persisted.readback_top_rank_score,
        }
    }))
}

fn parse_hypothesis_rank(rest: &[String]) -> CliResult<HypothesisRankArgs> {
    let mut args = HypothesisRankArgs::default();
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
            "--max-ranked" => {
                idx += 1;
                args.max_ranked =
                    parse_usize(value(rest, idx, "--max-ranked")?, "--max-ranked", 1)?;
            }
            "--review-top-n" => {
                idx += 1;
                args.review_top_n =
                    parse_usize(value(rest, idx, "--review-top-n")?, "--review-top-n", 0)?;
            }
            "--min-review-score" => {
                idx += 1;
                args.min_review_score = parse_unit(
                    value(rest, idx, "--min-review-score")?,
                    "--min-review-score",
                )?;
            }
            other => {
                return Err(CliError::usage(format!(
                    "unexpected hypothesis-rank flag {other}"
                )));
            }
        }
        idx += 1;
    }
    if args.input.as_os_str().is_empty() {
        return Err(CliError::usage("hypothesis-rank requires --input <json>"));
    }
    if args.out.as_os_str().is_empty() {
        return Err(CliError::usage("hypothesis-rank requires --out <json>"));
    }
    Ok(args)
}

impl HypothesisRankArgs {
    fn params(&self) -> RankedHypothesisParams {
        RankedHypothesisParams {
            max_ranked: self.max_ranked,
            review_top_n: self.review_top_n,
            min_review_score: self.min_review_score,
        }
    }
}

fn persist_ranking(path: &Path, artifact: &HypothesisRankArtifact) -> CliResult<PersistedRanking> {
    let bytes = serde_json::to_vec_pretty(artifact)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if path.exists() {
        let existing = fs::read(path)?;
        if existing != bytes {
            return Err(CliError::usage(format!(
                "refusing to overwrite existing different hypothesis ranking report {}",
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
            "hypothesis ranking report readback mismatch at {}",
            path.display()
        )));
    }
    let decoded: HypothesisRankArtifact = serde_json::from_slice(&readback)?;
    let top = decoded
        .report
        .hypotheses
        .first()
        .ok_or_else(|| CliError::usage("hypothesis ranking report had no ranked rows"))?;
    Ok(PersistedRanking {
        path: path.to_path_buf(),
        bytes: readback.len() as u64,
        sha256: sha256_hex(&readback),
        readback_input_count: decoded.report.input_count,
        readback_ranked_count: decoded.report.ranked_count,
        readback_human_review_count: decoded.report.human_review_count,
        readback_top_hypothesis_id: top.hypothesis_id.clone(),
        readback_top_rank_score: top.rank_score,
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
        let err = parse_hypothesis_rank(&["--input".to_string(), "target/in.json".to_string()])
            .unwrap_err();
        assert!(err.to_string().contains("--out"));
    }

    #[test]
    fn parse_accepts_all_thresholds() {
        let args = parse_hypothesis_rank(&[
            "--input".to_string(),
            "target/in.json".to_string(),
            "--out".to_string(),
            "target/out.json".to_string(),
            "--max-ranked".to_string(),
            "12".to_string(),
            "--review-top-n".to_string(),
            "3".to_string(),
            "--min-review-score".to_string(),
            "0.7".to_string(),
        ])
        .unwrap();
        assert_eq!(args.input, PathBuf::from("target/in.json"));
        assert_eq!(args.out, PathBuf::from("target/out.json"));
        assert_eq!(args.max_ranked, 12);
        assert_eq!(args.review_top_n, 3);
        assert_eq!(args.min_review_score, 0.7);
    }
}
