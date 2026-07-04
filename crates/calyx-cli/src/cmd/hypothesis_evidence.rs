//! `calyx assemble-hypothesis-evidence` -- materialize evaluator inputs from
//! chain-walk hypotheses and persisted Calyx evidence rows.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{CalyxError, Constellation, CxId, VaultStore};
use calyx_lodestar::{
    ChainWalkReport, EvidenceSource, HypothesisEvaluationInput,
    assemble_hypothesis_evaluation_inputs, chain_report_evidence_cx_ids,
    hypothesis_evidence_cx_ids,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::Subcommand;
use super::artifact_hash::sha256_hex;
use super::value;
use super::vault::{home_dir, resolve_vault_info, vault_salt};
use crate::error::{CliError, CliResult};
use crate::output::print_json;

const HYPOTHESIS_EVIDENCE_INPUT_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HypothesisEvidenceArgs {
    pub vault: String,
    pub chain: PathBuf,
    pub out: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct ChainWalksArtifactInput {
    #[serde(default)]
    node_metadata: BTreeMap<CxId, BTreeMap<String, String>>,
    report: ChainWalkReport,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct HypothesisEvaluateInputFile {
    schema_version: u32,
    inputs: Vec<HypothesisEvaluationInput>,
}

struct PersistedInputs {
    path: PathBuf,
    bytes: u64,
    sha256: String,
    readback_input_count: usize,
    readback_evidence_count: usize,
}

pub(crate) fn run(command: Subcommand) -> CliResult {
    let Subcommand::AssembleHypothesisEvidence(args) = command else {
        unreachable!("non-hypothesis-evidence command routed to hypothesis_evidence module");
    };
    run_assemble_hypothesis_evidence_with_home(&home_dir()?, args)
}

pub(crate) fn run_assemble_hypothesis_evidence_with_home(
    home: &Path,
    args: HypothesisEvidenceArgs,
) -> CliResult {
    let resolved = resolve_vault_info(home, &args.vault)?;
    let chain_bytes = fs::read(&args.chain)
        .map_err(|error| CliError::io(format!("read --chain {}: {error}", args.chain.display())))?;
    let artifact: ChainWalksArtifactInput =
        serde_json::from_slice(&chain_bytes).map_err(|error| {
            CliError::runtime(format!(
                "parse --chain {} as chain-walk artifact: {error}",
                args.chain.display()
            ))
        })?;
    let vault = AsterVault::new_durable(
        &resolved.path,
        resolved.vault_id,
        vault_salt(resolved.vault_id, &resolved.name),
        VaultOptions::default(),
    )?;
    let snapshot = vault.snapshot();
    let fallback_confidence = max_terminal_confidence_by_cx(&artifact.report);
    let mut sources = BTreeMap::new();
    for cx_id in chain_report_evidence_cx_ids(&artifact.report) {
        let row = vault.get(cx_id, snapshot).map_err(|error| {
            if error.code == "CALYX_STALE_DERIVED" {
                hypothesis_evidence_missing(cx_id)
            } else {
                error
            }
        })?;
        let fallback = *fallback_confidence.get(&cx_id).unwrap_or(&0.0);
        let source = source_from_constellation(row, artifact.node_metadata.get(&cx_id), fallback)?;
        sources.insert(cx_id, source);
    }
    let inputs = assemble_hypothesis_evaluation_inputs(&artifact.report, &sources)?;
    let file = HypothesisEvaluateInputFile {
        schema_version: HYPOTHESIS_EVIDENCE_INPUT_SCHEMA_VERSION,
        inputs,
    };
    let persisted = persist_inputs(&args.out, &file)?;
    print_json(&json!({
        "status": "ok",
        "vault": resolved.name,
        "vault_dir": resolved.path.display().to_string(),
        "chain": args.chain,
        "chain_bytes": chain_bytes.len(),
        "chain_sha256": sha256_hex(&chain_bytes),
        "out": persisted.path,
        "out_bytes": persisted.bytes,
        "out_sha256": persisted.sha256,
        "readback": {
            "input_count": persisted.readback_input_count,
            "evidence_count": persisted.readback_evidence_count,
        }
    }))
}

pub(crate) fn parse_assemble_hypothesis_evidence(rest: &[String]) -> CliResult<Subcommand> {
    let mut args = HypothesisEvidenceArgs {
        vault: String::new(),
        chain: PathBuf::new(),
        out: PathBuf::new(),
    };
    let mut idx = 0;
    if rest.first().is_some_and(|first| !first.starts_with('-')) {
        args.vault = rest[0].clone();
        idx = 1;
    }
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--vault" => {
                idx += 1;
                args.vault = value(rest, idx, "--vault")?.to_string();
            }
            "--chain" => {
                idx += 1;
                args.chain = PathBuf::from(value(rest, idx, "--chain")?);
            }
            "--out" => {
                idx += 1;
                args.out = PathBuf::from(value(rest, idx, "--out")?);
            }
            other => {
                return Err(CliError::usage(format!(
                    "unexpected assemble-hypothesis-evidence flag {other}"
                )));
            }
        }
        idx += 1;
    }
    if args.vault.trim().is_empty() {
        return Err(CliError::usage(
            "assemble-hypothesis-evidence requires <vault> or --vault <vault>",
        ));
    }
    if args.chain.as_os_str().is_empty() {
        return Err(CliError::usage(
            "assemble-hypothesis-evidence requires --chain <chain.json>",
        ));
    }
    if args.out.as_os_str().is_empty() {
        return Err(CliError::usage(
            "assemble-hypothesis-evidence requires --out <input.json>",
        ));
    }
    Ok(Subcommand::AssembleHypothesisEvidence(args))
}

fn source_from_constellation(
    row: Constellation,
    node_metadata: Option<&BTreeMap<String, String>>,
    fallback_confidence: f32,
) -> CliResult<EvidenceSource> {
    let combined = combined_metadata(&row.metadata, node_metadata);
    let (title, title_key) = first_value(
        &combined,
        &[
            "title",
            "source_title",
            "name",
            "source_id",
            "id",
            "question",
        ],
    )
    .ok_or_else(|| {
        hypothesis_evidence_invalid(format!(
            "evidence row {} has no title/source_id metadata",
            row.cx_id
        ))
    })?;
    let (abstract_text, text_key) = first_value(
        &combined,
        &[
            "abstract_text",
            "abstract",
            "text_snippet",
            "source_text",
            "text",
            "question",
        ],
    )
    .ok_or_else(|| hypothesis_evidence_empty_abstract(row.cx_id))?;
    if abstract_text.trim().is_empty() {
        return Err(hypothesis_evidence_empty_abstract(row.cx_id).into());
    }
    let (confidence, confidence_source) =
        grounding_confidence(&combined, fallback_confidence, row.cx_id)?;
    let mut provenance = provenance_from_constellation(&row, &combined);
    provenance.push(format!("title_metadata_key={title_key}"));
    provenance.push(format!("abstract_metadata_key={text_key}"));
    provenance.push(format!("grounding_confidence_source={confidence_source}"));
    Ok(EvidenceSource {
        cx_id: row.cx_id,
        title: title.to_string(),
        abstract_text: abstract_text.to_string(),
        grounding_confidence: confidence,
        provenance,
    })
}

fn combined_metadata(
    base: &BTreeMap<String, String>,
    node: Option<&BTreeMap<String, String>>,
) -> BTreeMap<String, String> {
    let mut combined = node.cloned().unwrap_or_default();
    combined.extend(base.iter().map(|(key, value)| (key.clone(), value.clone())));
    combined
}

fn first_value<'a>(
    metadata: &'a BTreeMap<String, String>,
    keys: &[&str],
) -> Option<(&'a str, String)> {
    keys.iter().find_map(|key| {
        metadata
            .get(*key)
            .map(String::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(|value| (value, (*key).to_string()))
    })
}

fn grounding_confidence(
    metadata: &BTreeMap<String, String>,
    fallback: f32,
    cx_id: CxId,
) -> CliResult<(f32, String)> {
    for key in [
        "grounding_confidence",
        "kernel_groundedness",
        "groundedness",
        "groundedness_score",
    ] {
        if let Some(raw) = metadata.get(key).filter(|value| !value.trim().is_empty()) {
            let value = raw.parse::<f32>().map_err(|error| {
                hypothesis_evidence_invalid(format!(
                    "parse {key} for evidence row {cx_id}: {error}"
                ))
            })?;
            if !score_is_valid(value) {
                return Err(hypothesis_evidence_invalid(format!(
                    "{key} for evidence row {cx_id} must be finite and in [0,1]"
                ))
                .into());
            }
            return Ok((value, format!("metadata:{key}")));
        }
    }
    if !score_is_valid(fallback) {
        return Err(hypothesis_evidence_invalid(format!(
            "fallback terminal confidence for evidence row {cx_id} must be finite and in [0,1]"
        ))
        .into());
    }
    Ok((fallback, "chain_terminal_confidence_max".to_string()))
}

fn provenance_from_constellation(
    row: &Constellation,
    metadata: &BTreeMap<String, String>,
) -> Vec<String> {
    let mut provenance = vec![
        format!("source_cx_id={}", row.cx_id),
        format!("base_panel_version={}", row.panel_version),
        format!("base_provenance_seq={}", row.provenance.seq),
        format!("base_provenance_hash={}", hex_lower(&row.provenance.hash)),
        format!("input_hash={}", hex_lower(&row.input_ref.hash)),
    ];
    if let Some(pointer) = &row.input_ref.pointer {
        provenance.push(format!("input_pointer={pointer}"));
    }
    for key in [
        "source_dataset",
        "source_id",
        "source_sha256",
        "source_url",
        "download_uri",
        "doi",
        "pmid",
        "pmcid",
        "license",
        "retrieval_ts",
        "source_path",
        "text_sha256",
    ] {
        if let Some(value) = metadata.get(key).filter(|value| !value.trim().is_empty()) {
            provenance.push(format!("{key}={value}"));
        }
    }
    provenance
}

fn max_terminal_confidence_by_cx(report: &ChainWalkReport) -> BTreeMap<CxId, f32> {
    let mut out = BTreeMap::new();
    for result in &report.results {
        for hypothesis in &result.hypotheses {
            for cx_id in hypothesis_evidence_cx_ids(hypothesis) {
                let entry = out.entry(cx_id).or_insert(0.0_f32);
                *entry = entry.max(hypothesis.terminal_confidence);
            }
        }
    }
    out
}

fn persist_inputs(path: &Path, file: &HypothesisEvaluateInputFile) -> CliResult<PersistedInputs> {
    let bytes = serde_json::to_vec_pretty(file).map_err(|error| {
        CliError::runtime(format!("serialize hypothesis evidence input file: {error}"))
    })?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if path.exists() {
        let existing = fs::read(path)?;
        if existing != bytes {
            return Err(CliError::usage(format!(
                "refusing to overwrite existing different hypothesis evidence input {}",
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
            "hypothesis evidence input readback mismatch at {}",
            path.display()
        )));
    }
    let decoded: HypothesisEvaluateInputFile =
        serde_json::from_slice(&readback).map_err(|error| {
            CliError::runtime(format!(
                "parse hypothesis evidence input readback {}: {error}",
                path.display()
            ))
        })?;
    let readback_evidence_count = decoded
        .inputs
        .iter()
        .map(|input| input.retrieved_evidence.len())
        .sum();
    Ok(PersistedInputs {
        path: path.to_path_buf(),
        bytes: readback.len() as u64,
        sha256: sha256_hex(&readback),
        readback_input_count: decoded.inputs.len(),
        readback_evidence_count,
    })
}

fn score_is_valid(score: f32) -> bool {
    score.is_finite() && (0.0..=1.0).contains(&score)
}

fn hypothesis_evidence_missing(cx_id: CxId) -> CalyxError {
    CalyxError {
        code: "CALYX_HYPOTHESIS_EVIDENCE_MISSING_PROVENANCE",
        message: format!("no Base CF evidence provenance row for {cx_id}"),
        remediation: "materialize or repair the missing evidence provenance row in Calyx",
    }
}

fn hypothesis_evidence_empty_abstract(cx_id: CxId) -> CalyxError {
    CalyxError {
        code: "CALYX_HYPOTHESIS_EVIDENCE_EMPTY_ABSTRACT",
        message: format!("evidence row {cx_id} has no persisted abstract/text snippet"),
        remediation: "store grounded source text/abstract metadata in Calyx before evaluation",
    }
}

fn hypothesis_evidence_invalid(detail: impl Into<String>) -> CalyxError {
    CalyxError {
        code: "CALYX_HYPOTHESIS_EVIDENCE_INVALID",
        message: detail.into(),
        remediation: "repair the named hypothesis evidence metadata and rerun",
    }
}

fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
pub(crate) fn tokens(args: &HypothesisEvidenceArgs) -> Vec<String> {
    vec![
        "assemble-hypothesis-evidence".to_string(),
        args.vault.clone(),
        "--chain".to_string(),
        args.chain.to_string_lossy().into_owned(),
        "--out".to_string(),
        args.out.to_string_lossy().into_owned(),
    ]
}

#[cfg(test)]
mod tests;
