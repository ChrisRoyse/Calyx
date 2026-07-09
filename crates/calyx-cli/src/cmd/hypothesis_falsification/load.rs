use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::Path;

use serde_json::Value;
use sha2::{Digest, Sha256};

use super::matching::{
    asserted_relation, needs_safety_triage, relation_matches, source_applicable,
};
use super::model::{
    EvidenceRow, HypothesisFalsificationArgs, HypothesisFlag, InputHypothesis, LoadedSources,
    RawQueryManifestRow, SkippedEvidenceRow,
};
use crate::cmd::discovery_run_preflight::{PreflightInput, preflight_input_files};
use crate::cmd::mechanistic_direction::{MutationConsequence, TargetModulation};
use crate::error::{CliError, CliResult};

mod classify;

use classify::{
    MechanismClassDecision, SourceClass, classify_dgidb_interaction, classify_dgidb_unmapped,
    classify_open_targets_edge, classify_pubtator_negative, classify_pubtator_support,
    classify_trial_row, classify_trial_summary, mechanism_checked_classes,
};

pub(super) struct HypothesisLoad {
    pub input_count: usize,
    pub hypotheses: Vec<InputHypothesis>,
}

pub(super) fn load_hypotheses(args: &HypothesisFalsificationArgs) -> CliResult<HypothesisLoad> {
    let mut input_count = 0_usize;
    let mut deduped = BTreeMap::<String, InputHypothesis>::new();
    let mut report_bytes = Vec::new();
    for report_path in &args.hypotheses_reports {
        let bytes = fs::read(report_path)?;
        report_bytes.push((report_path.clone(), bytes));
    }
    let preflight_inputs = report_bytes
        .iter()
        .map(|(path, bytes)| PreflightInput::new(path, bytes))
        .collect::<Vec<_>>();
    preflight_input_files(&args.preflight, &preflight_inputs)?;
    for (report_path, bytes) in report_bytes {
        let report: Value = serde_json::from_slice(&bytes).map_err(|error| {
            CliError::runtime(format!(
                "parse hypotheses report {}: {error}",
                report_path.display()
            ))
        })?;
        let rows = report
            .get("hypotheses")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                CliError::runtime(format!(
                    "hypotheses report missing hypotheses array: {}",
                    report_path.display()
                ))
            })?;
        for row in rows {
            input_count += 1;
            let hypothesis = InputHypothesis {
                hypothesis_id: str_field(row, "hypothesis_id"),
                source_id: str_field(row, "source_id"),
                source_name: str_field(row, "source_name"),
                source_type: str_field(row, "source_type"),
                target_id: str_field(row, "target_id"),
                target_name: str_field(row, "target_name"),
                target_type: str_field(row, "target_type"),
                support_count: usize_field(row, "support_count").unwrap_or(0),
                score: f64_field(row, "score").unwrap_or(0.0),
                mechanistic_direction_status: str_field(row, "mechanistic_direction_status"),
                required_target_modulation: optional_target_modulation(
                    row,
                    "required_target_modulation",
                    &report_path,
                )?,
                observed_target_modulation: optional_target_modulation(
                    row,
                    "observed_target_modulation",
                    &report_path,
                )?,
                mutation_consequence: optional_mutation_consequence(
                    row,
                    "mutation_consequence",
                    &report_path,
                )?,
                direction_reason_codes: string_array(row, "direction_reason_codes", usize::MAX),
            };
            if hypothesis.hypothesis_id.is_empty() {
                return Err(CliError::runtime(format!(
                    "hypothesis row missing hypothesis_id in {}",
                    report_path.display()
                )));
            }
            deduped
                .entry(hypothesis.hypothesis_id.clone())
                .or_insert(hypothesis);
        }
    }
    Ok(HypothesisLoad {
        input_count,
        hypotheses: deduped.into_values().collect(),
    })
}

pub(super) fn load_sources(
    args: &HypothesisFalsificationArgs,
    hypotheses: &[InputHypothesis],
) -> CliResult<LoadedSources> {
    let mut out = LoadedSources::default();
    scan_source(
        &args
            .pubtator_root
            .join("parsed")
            .join("supporting_literature.jsonl"),
        "pubtator",
        "supporting_literature",
        hypotheses,
        &mut out,
        classify_pubtator_support,
    )?;
    scan_source(
        &args
            .pubtator_root
            .join("parsed")
            .join("contradicting_or_negative_literature.jsonl"),
        "pubtator",
        "negative_literature",
        hypotheses,
        &mut out,
        classify_pubtator_negative,
    )?;
    scan_source(
        &args
            .clinicaltrials_root
            .join("parsed")
            .join("clinicaltrials_seed_summaries.jsonl"),
        "clinicaltrials",
        "seed_summaries",
        hypotheses,
        &mut out,
        classify_trial_summary,
    )?;
    scan_source(
        &args
            .clinicaltrials_root
            .join("parsed")
            .join("clinicaltrials_trial_rows.jsonl"),
        "clinicaltrials",
        "trial_rows",
        hypotheses,
        &mut out,
        classify_trial_row,
    )?;
    scan_source(
        &args
            .dgidb_root
            .join("parsed")
            .join("seed_pair_graphql_interactions.jsonl"),
        "dgidb",
        "seed_pair_interactions",
        hypotheses,
        &mut out,
        classify_dgidb_interaction,
    )?;
    scan_source(
        &args.dgidb_root.join("parsed").join("unmapped_rows.jsonl"),
        "dgidb",
        "unmapped_no_hit_rows",
        hypotheses,
        &mut out,
        classify_dgidb_unmapped,
    )?;
    scan_source(
        &args
            .open_targets_root
            .join("open_targets_validation_edges.jsonl"),
        "open_targets",
        "validation_edges",
        hypotheses,
        &mut out,
        classify_open_targets_edge,
    )?;
    Ok(out)
}

pub(super) fn flag_hypotheses(
    hypotheses: &[InputHypothesis],
    sources: &LoadedSources,
) -> Vec<HypothesisFlag> {
    hypotheses
        .iter()
        .map(|hypothesis| {
            let support = evidence_for(&sources.support_evidence, &hypothesis.hypothesis_id);
            let counter = evidence_for(&sources.counter_evidence, &hypothesis.hypothesis_id);
            let support_weight = support.iter().map(|row| row.weight).sum::<f64>();
            let counter_weight = counter.iter().map(|row| row.weight).sum::<f64>();
            let mut reason_codes = counter
                .iter()
                .map(|row| row.reason_code.clone())
                .collect::<BTreeSet<_>>();
            if reason_codes.is_empty() {
                reason_codes.insert("no_counter_evidence_found_in_current_sources".to_string());
            }
            if needs_safety_triage(hypothesis) {
                reason_codes.insert("safety_toxicity_triage_pending_issue_1181".to_string());
            }
            for code in &hypothesis.direction_reason_codes {
                reason_codes.insert(code.clone());
            }
            let mechanistic_status = hypothesis_mechanistic_status(hypothesis);
            if mechanistic_status != "not_mechanistic" && mechanistic_status != "direction_ready" {
                reason_codes.insert(format!("mechanistic_direction_status:{mechanistic_status}"));
            }
            let score = counter_weight / (counter_weight + support_weight + 1.0);
            let rounded_score = (score * 1000.0).round() / 1000.0;
            HypothesisFlag {
                hypothesis_id: hypothesis.hypothesis_id.clone(),
                source_name: hypothesis.source_name.clone(),
                source_type: hypothesis.source_type.clone(),
                target_name: hypothesis.target_name.clone(),
                target_type: hypothesis.target_type.clone(),
                support_evidence_count: support.len(),
                counter_evidence_count: counter.len(),
                support_weight,
                counter_weight,
                falsification_score: if rounded_score == 0.0 {
                    0.0
                } else {
                    rounded_score
                },
                reason_codes: reason_codes.into_iter().collect(),
                mechanistic_direction_status: mechanistic_status,
                required_target_modulation: hypothesis.required_target_modulation,
                observed_target_modulation: hypothesis.observed_target_modulation,
                mutation_consequence: hypothesis.mutation_consequence,
                sweep_status: if counter.is_empty() {
                    "complete_no_counterevidence_found_in_current_sources".to_string()
                } else {
                    "complete_counterevidence_found".to_string()
                },
                human_review_atlas_status: "falsification_sweep_complete".to_string(),
                clinical_boundary:
                    "Hypothesis triage only; not efficacy, safety, actionability, or cure evidence."
                        .to_string(),
            }
        })
        .collect()
}

fn scan_source(
    path: &Path,
    system: &str,
    role: &str,
    hypotheses: &[InputHypothesis],
    out: &mut LoadedSources,
    classify: fn(&Value) -> Vec<SourceClass>,
) -> CliResult {
    let bytes = fs::read(path)?;
    let source_sha = sha256_hex(&bytes);
    out.raw_query_manifest.push(RawQueryManifestRow {
        source_system: system.to_string(),
        source_path: path.display().to_string(),
        source_sha256: source_sha.clone(),
        bytes: bytes.len() as u64,
        role: role.to_string(),
    });
    for (idx, row) in read_jsonl(path)?.into_iter().enumerate() {
        let classes = classify(&row);
        if classes.is_empty() {
            continue;
        }
        let relation = match asserted_relation(system, role, &row) {
            Ok(relation) => relation,
            Err(reason_code) => {
                out.skipped_evidence.push(SkippedEvidenceRow {
                    source_system: system.to_string(),
                    role: role.to_string(),
                    reason_code: reason_code.to_string(),
                    source_path: path.display().to_string(),
                    source_sha256: source_sha.clone(),
                    source_row_index: idx + 1,
                    summary: format!("{system} {role} row lacks asserted relation endpoints"),
                    mechanistic_direction: None,
                });
                continue;
            }
        };
        for hypothesis in hypotheses {
            if !source_applicable(system, hypothesis) {
                continue;
            }
            if !relation_matches(hypothesis, &relation) {
                continue;
            }
            let classes = match mechanism_checked_classes(system, role, &row, hypothesis, &classes)
            {
                MechanismClassDecision::Use(classes) => classes,
                MechanismClassDecision::Skip {
                    reason_code,
                    summary,
                    direction,
                } => {
                    out.skipped_evidence.push(SkippedEvidenceRow {
                        source_system: system.to_string(),
                        role: role.to_string(),
                        reason_code,
                        source_path: path.display().to_string(),
                        source_sha256: source_sha.clone(),
                        source_row_index: idx + 1,
                        summary,
                        mechanistic_direction: Some(direction),
                    });
                    continue;
                }
            };
            for class in &classes {
                let evidence = EvidenceRow {
                    hypothesis_id: hypothesis.hypothesis_id.clone(),
                    evidence_kind: class.kind.to_string(),
                    source_system: system.to_string(),
                    reason_code: class.reason.to_string(),
                    source_path: path.display().to_string(),
                    source_sha256: source_sha.clone(),
                    source_row_index: idx + 1,
                    weight: class.weight,
                    summary: class.summary.clone(),
                    mechanistic_direction: class.mechanistic_direction.clone(),
                };
                if class.kind == "support" {
                    out.support_evidence.push(evidence);
                } else {
                    out.counter_evidence.push(evidence);
                }
            }
        }
    }
    Ok(())
}

fn evidence_for<'a>(rows: &'a [EvidenceRow], hypothesis_id: &str) -> Vec<&'a EvidenceRow> {
    rows.iter()
        .filter(|row| row.hypothesis_id == hypothesis_id)
        .collect()
}

fn read_jsonl(path: &Path) -> CliResult<Vec<Value>> {
    let file = File::open(path)
        .map_err(|error| CliError::io(format!("read {}: {error}", path.display())))?;
    let mut out = Vec::new();
    for (idx, line) in BufReader::new(file).lines().enumerate() {
        let line =
            line.map_err(|error| CliError::io(format!("read {}: {error}", path.display())))?;
        if line.trim().is_empty() {
            continue;
        }
        out.push(serde_json::from_str(&line).map_err(|error| {
            CliError::runtime(format!(
                "parse {} line {}: {error}",
                path.display(),
                idx + 1
            ))
        })?);
    }
    Ok(out)
}

pub(super) fn str_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

pub(super) fn usize_field(value: &Value, key: &str) -> Option<usize> {
    value
        .get(key)
        .and_then(|raw| raw.as_u64())
        .and_then(|raw| usize::try_from(raw).ok())
}

pub(super) fn f64_field(value: &Value, key: &str) -> Option<f64> {
    value.get(key).and_then(|raw| {
        raw.as_f64()
            .or_else(|| raw.as_u64().map(|value| value as f64))
    })
}

pub(super) fn array_len(value: &Value, key: &str) -> usize {
    value.get(key).and_then(Value::as_array).map_or(0, Vec::len)
}

fn string_array(value: &Value, key: &str, max: usize) -> Vec<String> {
    match value.get(key) {
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(Value::as_str)
            .take(max)
            .map(str::to_string)
            .collect(),
        Some(Value::String(value)) => vec![value.clone()],
        _ => Vec::new(),
    }
}

fn optional_target_modulation(
    value: &Value,
    key: &str,
    report_path: &Path,
) -> CliResult<Option<TargetModulation>> {
    let Some(raw) = value.get(key) else {
        return Ok(None);
    };
    serde_json::from_value(raw.clone()).map_err(|error| {
        CliError::runtime(format!(
            "CALYX_FALSIFY_MALFORMED_DIRECTION_FIELD: report={} field={key} error={error}",
            report_path.display()
        ))
    })
}

fn optional_mutation_consequence(
    value: &Value,
    key: &str,
    report_path: &Path,
) -> CliResult<Option<MutationConsequence>> {
    let Some(raw) = value.get(key) else {
        return Ok(None);
    };
    serde_json::from_value(raw.clone()).map_err(|error| {
        CliError::runtime(format!(
            "CALYX_FALSIFY_MALFORMED_DIRECTION_FIELD: report={} field={key} error={error}",
            report_path.display()
        ))
    })
}

pub(super) fn is_target_disease_hypothesis(hypothesis: &InputHypothesis) -> bool {
    matches!(hypothesis.source_type.as_str(), "gene" | "gene_protein")
        && hypothesis.target_type == "disease"
}

pub(super) fn is_drug_target_hypothesis(hypothesis: &InputHypothesis) -> bool {
    hypothesis.source_type == "chemical"
        && matches!(hypothesis.target_type.as_str(), "gene" | "gene_protein")
}

fn hypothesis_mechanistic_status(hypothesis: &InputHypothesis) -> String {
    if is_target_disease_hypothesis(hypothesis) {
        if hypothesis.required_target_modulation.is_some() {
            "direction_ready".to_string()
        } else {
            "required_direction_missing".to_string()
        }
    } else if is_drug_target_hypothesis(hypothesis) {
        if hypothesis.observed_target_modulation.is_some() {
            "action_direction_ready".to_string()
        } else {
            "action_direction_missing".to_string()
        }
    } else {
        "not_mechanistic".to_string()
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}
