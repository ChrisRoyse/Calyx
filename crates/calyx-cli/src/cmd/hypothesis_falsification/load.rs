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
use crate::error::{CliError, CliResult};

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

#[derive(Clone)]
struct SourceClass {
    kind: &'static str,
    reason: &'static str,
    weight: f64,
    summary: String,
}

fn classify_pubtator_support(row: &Value) -> Vec<SourceClass> {
    vec![SourceClass {
        kind: "support",
        reason: "pubtator_supporting_literature",
        weight: 1.0 + f64_field(row, "relation_count").unwrap_or(0.0).min(10.0) / 10.0,
        summary: format!(
            "PMID {} relation_count {} support_basis {}",
            str_field(row, "pmid"),
            usize_field(row, "relation_count").unwrap_or(0),
            str_field(row, "support_basis")
        ),
    }]
}

fn classify_pubtator_negative(row: &Value) -> Vec<SourceClass> {
    vec![SourceClass {
        kind: "counter",
        reason: "pubtator_negative_text_signal",
        weight: 2.5,
        summary: format!(
            "PMID {} negative signal {:?}",
            str_field(row, "pmid"),
            row.get("negative_signal_match").and_then(Value::as_str)
        ),
    }]
}

fn classify_trial_summary(row: &Value) -> Vec<SourceClass> {
    let mut out = Vec::new();
    let total = usize_field(row, "total_count").unwrap_or(0);
    if total > 0 {
        out.push(SourceClass {
            kind: "support",
            reason: "clinicaltrials_registry_hits",
            weight: 0.5
                + f64_field(row, "with_results_count")
                    .unwrap_or(0.0)
                    .min(10.0)
                    / 10.0,
            summary: format!(
                "ClinicalTrials.gov total_count {} results {} exact_intervention {}",
                total,
                usize_field(row, "with_results_count").unwrap_or(0),
                usize_field(row, "exact_intervention_match_count").unwrap_or(0)
            ),
        });
    }
    let stopped = usize_field(row, "stopped_status_count").unwrap_or(0);
    if stopped > 0 {
        out.push(SourceClass {
            kind: "counter",
            reason: "clinicaltrials_stopped_status_count",
            weight: (stopped as f64 * 0.5).clamp(0.5, 3.0),
            summary: format!("ClinicalTrials.gov stopped_status_count {stopped}"),
        });
    }
    out
}

fn classify_trial_row(row: &Value) -> Vec<SourceClass> {
    let status = str_field(row, "overall_status");
    if matches!(status.as_str(), "TERMINATED" | "WITHDRAWN" | "SUSPENDED") {
        return vec![SourceClass {
            kind: "counter",
            reason: "clinicaltrials_stopped_trial",
            weight: 1.0,
            summary: format!(
                "{} {} why_stopped {:?}",
                str_field(row, "nct_id"),
                status,
                row.get("why_stopped").and_then(Value::as_str)
            ),
        }];
    }
    if row
        .get("has_results")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || status == "COMPLETED"
    {
        return vec![SourceClass {
            kind: "support",
            reason: "clinicaltrials_completed_or_results_trial",
            weight: 0.5,
            summary: format!("{} {} has_results", str_field(row, "nct_id"), status),
        }];
    }
    Vec::new()
}

fn classify_dgidb_interaction(row: &Value) -> Vec<SourceClass> {
    vec![SourceClass {
        kind: "support",
        reason: "dgidb_exact_pair_interaction",
        weight: 1.0 + f64_field(row, "interaction_score").unwrap_or(0.0).min(2.0),
        summary: format!(
            "DGIdb {}-{} interaction_score {} source_dbs {}",
            str_field(row, "drug"),
            str_field(row, "gene"),
            f64_field(row, "interaction_score").unwrap_or(0.0),
            array_len(row, "source_dbs")
        ),
    }]
}

fn classify_dgidb_unmapped(row: &Value) -> Vec<SourceClass> {
    vec![SourceClass {
        kind: "counter",
        reason: "dgidb_exact_pair_no_hit",
        weight: 1.5,
        summary: format!(
            "DGIdb no-hit {}-{} reason {}",
            str_field(row, "drug"),
            str_field(row, "gene"),
            str_field(row, "reason")
        ),
    }]
}

fn classify_open_targets_edge(row: &Value) -> Vec<SourceClass> {
    let score = f64_field(row, "score").unwrap_or(0.0);
    if score >= 0.05 {
        vec![SourceClass {
            kind: "support",
            reason: "open_targets_association_score",
            weight: score.min(1.0),
            summary: format!(
                "Open Targets {} score {} disease {} target {}",
                str_field(row, "open_targets_data_version"),
                score,
                str_field(row, "disease_name"),
                str_field(row, "target_name")
            ),
        }]
    } else {
        vec![SourceClass {
            kind: "counter",
            reason: "open_targets_low_score_exact_pair",
            weight: 0.5,
            summary: format!("Open Targets low score {score}"),
        }]
    }
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

fn str_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn usize_field(value: &Value, key: &str) -> Option<usize> {
    value
        .get(key)
        .and_then(|raw| raw.as_u64())
        .and_then(|raw| usize::try_from(raw).ok())
}

fn f64_field(value: &Value, key: &str) -> Option<f64> {
    value.get(key).and_then(|raw| {
        raw.as_f64()
            .or_else(|| raw.as_u64().map(|value| value as f64))
    })
}

fn array_len(value: &Value, key: &str) -> usize {
    value.get(key).and_then(Value::as_array).map_or(0, Vec::len)
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
