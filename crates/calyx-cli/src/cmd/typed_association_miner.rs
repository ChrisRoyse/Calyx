//! `calyx typed-association-miner` ranks bounded typed concept-pair hypotheses.

use std::fs;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

mod load;
mod model;
mod persist;
#[cfg(test)]
mod tests;

pub(crate) use model::TypedAssociationMinerArgs;
use model::{MinerCliSummary, MinerReport};

use super::discovery_run_preflight::{RUN_MANIFEST_FLAG, RUN_STAGE_ID_FLAG, preflight_input_bytes};
use super::{Subcommand, value};
use crate::error::{CliError, CliResult};
use crate::output::print_json;

const SCHEMA_VERSION: u32 = 2;

pub(crate) fn parse_typed_association_miner(rest: &[String]) -> CliResult<Subcommand> {
    let mut args = TypedAssociationMinerArgs::default();
    let mut idx = 0;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--typed-root" => {
                idx += 1;
                args.typed_root = value(rest, idx, "--typed-root")?.into();
            }
            "--validation-report" => {
                idx += 1;
                args.validation_report = value(rest, idx, "--validation-report")?.into();
            }
            "--out-dir" => {
                idx += 1;
                args.out_dir = value(rest, idx, "--out-dir")?.into();
            }
            "--source-type" => {
                idx += 1;
                args.source_type = Some(value(rest, idx, "--source-type")?.to_ascii_lowercase());
            }
            "--target-type" => {
                idx += 1;
                args.target_type = Some(value(rest, idx, "--target-type")?.to_ascii_lowercase());
            }
            "--name-contains" => {
                idx += 1;
                args.name_contains =
                    Some(value(rest, idx, "--name-contains")?.to_ascii_lowercase());
            }
            "--source-issue" => {
                idx += 1;
                args.source_issue = Some(parse_u64(value(rest, idx, "--source-issue")?)?);
            }
            "--min-support" => {
                idx += 1;
                args.min_support =
                    parse_usize(value(rest, idx, "--min-support")?, 1, "--min-support")?;
            }
            "--max-pairs" => {
                idx += 1;
                args.max_pairs = parse_usize(value(rest, idx, "--max-pairs")?, 1, "--max-pairs")?;
            }
            "--max-input-edges" => {
                idx += 1;
                args.max_input_edges = parse_usize(
                    value(rest, idx, "--max-input-edges")?,
                    1,
                    "--max-input-edges",
                )?;
            }
            "--max-paths-per-pair" => {
                idx += 1;
                args.max_paths_per_pair = parse_usize(
                    value(rest, idx, "--max-paths-per-pair")?,
                    1,
                    "--max-paths-per-pair",
                )?;
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
                    "unexpected typed-association-miner flag {other}"
                )));
            }
        }
        idx += 1;
    }
    require(!args.typed_root.as_os_str().is_empty(), "--typed-root")?;
    require(
        !args.validation_report.as_os_str().is_empty(),
        "--validation-report",
    )?;
    require(!args.out_dir.as_os_str().is_empty(), "--out-dir")?;
    if args.max_pairs > 10_000 || args.max_input_edges > 1_000_000 {
        return Err(CliError::usage(
            "typed-association-miner bounds exceed hard safety limits",
        ));
    }
    args.preflight
        .validate_for_command("typed-association-miner")?;
    Ok(Subcommand::TypedAssociationMiner(args))
}

pub(crate) fn run(command: Subcommand) -> CliResult {
    let Subcommand::TypedAssociationMiner(args) = command else {
        unreachable!("non-typed-association-miner command routed here");
    };
    let report = build_report(&args)?;
    let readback = persist::persist(&args.out_dir, &report)?;
    print_json(&MinerCliSummary {
        status: "ok",
        out_dir: args.out_dir.display().to_string(),
        report: readback.report.display().to_string(),
        report_sha256: readback.report_sha256,
        hypotheses_jsonl: readback.hypotheses.display().to_string(),
        hypotheses_sha256: readback.hypotheses_sha256,
        blocked_candidates_jsonl: readback.blocked_candidates.display().to_string(),
        blocked_candidates_sha256: readback.blocked_candidates_sha256,
        score_summary_json: readback.summary.display().to_string(),
        score_summary_sha256: readback.summary_sha256,
        emitted_hypothesis_count: report.emitted_hypothesis_count,
        candidate_pair_count: report.candidate_pair_count,
        blocked_candidate_count: report.blocked_candidate_count,
        readback_hypothesis_count: readback.hypothesis_count,
        readback_blocked_candidate_count: readback.blocked_candidate_count,
        scan_limit_reached: report.scan_limit_reached,
    })?;
    if report.emitted_hypothesis_count == 0 {
        return Err(CliError::runtime(format!(
            "typed-association-miner emitted no accepted hypotheses; blocked candidates persisted at {}",
            readback.blocked_candidates.display()
        )));
    }
    Ok(())
}

fn build_report(args: &TypedAssociationMinerArgs) -> CliResult<MinerReport> {
    let validation_bytes = fs::read(&args.validation_report)?;
    preflight_input_bytes(&args.preflight, &validation_bytes)?;
    let validation_sha = sha256_hex(&validation_bytes);
    let validation: Value = serde_json::from_slice(&validation_bytes).map_err(|error| {
        CliError::runtime(format!(
            "parse --validation-report {}: {error}",
            args.validation_report.display()
        ))
    })?;
    if !validation
        .get("gate_passed")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Err(CliError::runtime(format!(
            "validation report did not pass: {}",
            args.validation_report.display()
        )));
    }
    let nodes = load::load_nodes(&args.typed_root.join("typed_nodes.jsonl"))?;
    let scan = load::scan_edges(args, &nodes)?;
    if scan.candidates.is_empty() && scan.blocked_candidates.is_empty() {
        return Err(CliError::runtime(
            "typed-association-miner found no candidates after filters",
        ));
    }
    let mut hypotheses = scan
        .candidates
        .into_iter()
        .map(|mut h| {
            h.validation_gate_report_sha256 = validation_sha.clone();
            h.score = model::score(h.support_count, scan.max_support);
            h.novelty_score = model::novelty(h.support_count);
            h
        })
        .collect::<Vec<_>>();
    hypotheses.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then(a.hypothesis_id.cmp(&b.hypothesis_id))
    });
    let candidate_pair_count = hypotheses.len();
    hypotheses.truncate(args.max_pairs);
    Ok(MinerReport {
        schema_version: SCHEMA_VERSION,
        status: "ok".to_string(),
        typed_root: args.typed_root.display().to_string(),
        validation_report: args.validation_report.display().to_string(),
        validation_report_sha256: validation_sha,
        validation_gate_passed: true,
        input_node_count: nodes.len(),
        input_edge_count: scan.input_edges,
        scan_limit_reached: scan.limit_reached,
        candidate_pair_count,
        blocked_candidate_count: scan.blocked_candidates.len(),
        emitted_hypothesis_count: hypotheses.len(),
        filters: json!({
            "source_type": args.source_type,
            "target_type": args.target_type,
            "name_contains": args.name_contains,
            "source_issue": args.source_issue,
            "min_support": args.min_support,
            "max_pairs": args.max_pairs,
            "max_input_edges": args.max_input_edges,
            "max_paths_per_pair": args.max_paths_per_pair,
            "source_date_filter": "not_available_in_typed_overlay",
            "mechanistic_direction_gate": "strict_for_target_disease_and_drug_target_pairs",
        }),
        hypotheses,
        blocked_candidates: scan.blocked_candidates,
    })
}

fn parse_u64(raw: &str) -> CliResult<u64> {
    raw.parse::<u64>()
        .map_err(|error| CliError::usage(format!("parse integer {raw}: {error}")))
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
            "typed-association-miner requires {flag} <path>"
        )))
    }
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
