use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::AssociationValidationArgs;
use super::model::{
    BenchmarkSourceRow, GateParams, MechanisticDirectionBlockedRow, SourceManifest, TimeSplitRow,
    sha256_hex,
};
use crate::cmd::mechanistic_direction::infer_required_target_modulation;
use crate::error::{CliError, CliResult};

pub(crate) struct LoadedSources {
    pub manifests: Vec<SourceManifest>,
    pub benchmark_rows: Vec<BenchmarkSourceRow>,
    pub mechanistic_direction_blocked_rows: Vec<MechanisticDirectionBlockedRow>,
    pub time_split_rows: Vec<TimeSplitRow>,
}

struct JsonlRow {
    rel: String,
    line_no: usize,
    line_sha256: String,
    value: Value,
}

pub(crate) fn load_sources(
    args: &AssociationValidationArgs,
    params: &GateParams,
) -> CliResult<LoadedSources> {
    let manifests = vec![
        manifest(
            "typed_overlay",
            &args.typed_root,
            &["typed_graph_summary.json"],
        )?,
        manifest(
            "open_targets",
            &args.open_targets_root,
            &["open_targets_validation_edges.jsonl"],
        )?,
        manifest(
            "pubtator_pubmed",
            &args.pubtator_root,
            &["parsed/association_evidence_edges.jsonl"],
        )?,
        manifest(
            "clinicaltrials",
            &args.clinicaltrials_root,
            &[
                "parsed/clinicaltrials_seed_summaries.jsonl",
                "parsed/clinicaltrials_trial_rows.jsonl",
            ],
        )?,
        manifest(
            "dgidb",
            &args.dgidb_root,
            &[
                "parsed/seed_pair_tsv_interactions.jsonl",
                "parsed/unmapped_rows.jsonl",
            ],
        )?,
    ];
    let mut rows = Vec::new();
    load_pubtator(&args.pubtator_root, &mut rows)?;
    let dgidb_positive_seeds = load_dgidb_positive(&args.dgidb_root, &mut rows)?;
    load_dgidb_negative(&args.dgidb_root, &dgidb_positive_seeds, &mut rows)?;
    load_clinical_summaries(&args.clinicaltrials_root, &mut rows)?;
    let mut blocked = Vec::new();
    load_open_targets(&args.open_targets_root, &mut rows, &mut blocked)?;
    let time_split_rows = load_time_split(&args.clinicaltrials_root, params.cutoff_year)?;
    Ok(LoadedSources {
        manifests,
        benchmark_rows: rows,
        mechanistic_direction_blocked_rows: blocked,
        time_split_rows,
    })
}

fn load_pubtator(root: &Path, rows: &mut Vec<BenchmarkSourceRow>) -> CliResult {
    let rel = "parsed/association_evidence_edges.jsonl";
    for row in read_jsonl(root, rel)? {
        let publications = number(&row.value, "relation_publication_sum");
        let exact = number(&row.value, "relation_endpoint_exact_match_count");
        let exported = number(&row.value, "export_docs_with_both_entities");
        if publications <= 0.0 || exact <= 0.0 || exported <= 0.0 {
            continue;
        }
        let seed = str_field(&row.value, "seed_id");
        rows.push(BenchmarkSourceRow {
            row_id: format!("pubtator:{seed}"),
            benchmark_kind: "known_positive_pubtator_relation".to_string(),
            source_family: "pubtator_pubmed".to_string(),
            source_path: row.rel,
            source_line: Some(row.line_no),
            source_row_sha256: row.line_sha256,
            seed_id: seed,
            subject: str_field(&row.value, "left_term"),
            object: str_field(&row.value, "right_term"),
            label: Some(true),
            source_year: None,
            features: json!({
                "relation_publication_sum": publications,
                "relation_endpoint_exact_match_count": exact,
                "export_docs_with_both_entities": exported,
            }),
            mechanistic_direction: None,
            raw: row.value,
        });
    }
    Ok(())
}

fn load_dgidb_positive(
    root: &Path,
    rows: &mut Vec<BenchmarkSourceRow>,
) -> CliResult<BTreeSet<String>> {
    let rel = "parsed/seed_pair_tsv_interactions.jsonl";
    let mut grouped: BTreeMap<String, (JsonlRow, f64)> = BTreeMap::new();
    for row in read_jsonl(root, rel)? {
        let seed = str_field(&row.value, "seed_id");
        let score = string_number(&row.value, "interaction_score");
        grouped
            .entry(seed)
            .and_modify(|(_, existing)| *existing = existing.max(score))
            .or_insert((row, score));
    }
    let mut positive_seeds = BTreeSet::new();
    for (seed, (row, max_score)) in grouped {
        positive_seeds.insert(seed.clone());
        rows.push(BenchmarkSourceRow {
            row_id: format!("dgidb:{seed}"),
            benchmark_kind: "known_positive_dgidb_interaction".to_string(),
            source_family: "dgidb".to_string(),
            source_path: row.rel,
            source_line: Some(row.line_no),
            source_row_sha256: row.line_sha256,
            seed_id: seed,
            subject: str_field(&row.value, "drug_name"),
            object: str_field(&row.value, "gene_name"),
            label: Some(true),
            source_year: Some(2024),
            features: json!({ "max_interaction_score": max_score }),
            mechanistic_direction: None,
            raw: row.value,
        });
    }
    Ok(positive_seeds)
}

fn load_dgidb_negative(
    root: &Path,
    positive_seeds: &BTreeSet<String>,
    rows: &mut Vec<BenchmarkSourceRow>,
) -> CliResult {
    let rel = "parsed/unmapped_rows.jsonl";
    for row in read_jsonl(root, rel)? {
        let seed = str_field(&row.value, "seed_id");
        if positive_seeds.contains(&seed) {
            continue;
        }
        if !str_field(&row.value, "reason").contains("no_dgidb_graphql_interaction") {
            continue;
        }
        rows.push(BenchmarkSourceRow {
            row_id: format!("dgidb_no_hit:{seed}"),
            benchmark_kind: "known_negative_dgidb_exact_no_hit".to_string(),
            source_family: "dgidb".to_string(),
            source_path: row.rel,
            source_line: Some(row.line_no),
            source_row_sha256: row.line_sha256,
            seed_id: seed,
            subject: str_field(&row.value, "drug"),
            object: str_field(&row.value, "gene"),
            label: Some(false),
            source_year: Some(2024),
            features: json!({ "exact_pair_total_count": number(&row.value, "total_count") }),
            mechanistic_direction: None,
            raw: row.value,
        });
    }
    Ok(())
}

fn load_clinical_summaries(root: &Path, rows: &mut Vec<BenchmarkSourceRow>) -> CliResult {
    let rel = "parsed/clinicaltrials_seed_summaries.jsonl";
    for row in read_jsonl(root, rel)? {
        let total = number(&row.value, "total_count");
        let exact = number(&row.value, "exact_intervention_match_count");
        if total <= 0.0 || exact <= 0.0 {
            continue;
        }
        let seed = str_field(&row.value, "seed_id");
        rows.push(BenchmarkSourceRow {
            row_id: format!("clinicaltrials:{seed}"),
            benchmark_kind: "known_positive_clinical_trial_registry".to_string(),
            source_family: "clinicaltrials".to_string(),
            source_path: row.rel,
            source_line: Some(row.line_no),
            source_row_sha256: row.line_sha256,
            seed_id: seed,
            subject: str_field(&row.value, "intervention"),
            object: str_field(&row.value, "condition"),
            label: Some(true),
            source_year: None,
            features: json!({
                "total_count": total,
                "exact_intervention_match_count": exact,
                "with_results_count": number(&row.value, "with_results_count"),
                "stopped_status_count": number(&row.value, "stopped_status_count"),
                "max_trial_evidence_score": number(&row.value, "max_trial_evidence_score"),
            }),
            mechanistic_direction: None,
            raw: row.value,
        });
    }
    Ok(())
}

fn load_open_targets(
    root: &Path,
    rows: &mut Vec<BenchmarkSourceRow>,
    blocked: &mut Vec<MechanisticDirectionBlockedRow>,
) -> CliResult {
    let rel = "open_targets_validation_edges.jsonl";
    for row in read_jsonl(root, rel)? {
        let target_mapped = array_len(&row.value, "overlay_target_concepts") > 0;
        let disease_mapped = array_len(&row.value, "overlay_disease_concepts") > 0;
        let score = number(&row.value, "score");
        if !target_mapped || !disease_mapped || score <= 0.05 {
            continue;
        }
        let direction = infer_required_target_modulation(&row.value);
        if !direction.is_required_direction_known() {
            blocked.push(MechanisticDirectionBlockedRow {
                source_system: "open_targets".to_string(),
                source_path: row.rel,
                source_line: Some(row.line_no),
                source_row_sha256: row.line_sha256,
                target_name: str_field(&row.value, "target_name"),
                disease_name: str_field(&row.value, "disease_name"),
                reason_codes: direction.reason_codes.clone(),
                mechanistic_direction: direction,
                raw: row.value,
            });
            continue;
        }
        let seed = format!(
            "{}_{}",
            str_field(&row.value, "target_name"),
            str_field(&row.value, "disease_name")
        );
        rows.push(BenchmarkSourceRow {
            row_id: format!("open_targets:{}", str_field(&row.value, "edge_id")),
            benchmark_kind: "known_positive_open_targets_target_disease".to_string(),
            source_family: "open_targets".to_string(),
            source_path: row.rel,
            source_line: Some(row.line_no),
            source_row_sha256: row.line_sha256,
            seed_id: seed,
            subject: str_field(&row.value, "target_name"),
            object: str_field(&row.value, "disease_name"),
            label: Some(true),
            source_year: Some(2026),
            features: json!({
                "open_targets_score": score,
                "required_target_modulation": direction.required_target_modulation_name(),
            }),
            mechanistic_direction: Some(direction),
            raw: row.value,
        });
    }
    Ok(())
}

fn load_time_split(root: &Path, cutoff_year: i32) -> CliResult<Vec<TimeSplitRow>> {
    let rel = "parsed/clinicaltrials_trial_rows.jsonl";
    let mut grouped: BTreeMap<String, (usize, f64, usize, bool, Vec<String>)> = BTreeMap::new();
    for row in read_jsonl(root, rel)? {
        let seed = str_field(&row.value, "seed_id");
        let score = number(&row.value, "trial_evidence_score");
        let year = str_field(&row.value, "start_date")
            .get(0..4)
            .and_then(|raw| raw.parse::<i32>().ok());
        let entry = grouped
            .entry(seed)
            .or_insert((0, 0.0, 0, false, Vec::new()));
        entry.4.push(format!("{}:{}", row.rel, row.line_no));
        match year {
            Some(year) if year <= cutoff_year => {
                entry.0 += 1;
                entry.1 = entry.1.max(score);
            }
            Some(year) if year > cutoff_year => {
                entry.2 += 1;
                if bool_field(&row.value, "exact_intervention_match")
                    && bool_field(&row.value, "condition_match")
                    && (bool_field(&row.value, "has_results")
                        || str_field(&row.value, "overall_status") == "COMPLETED")
                {
                    entry.3 = true;
                }
            }
            _ => {}
        }
    }
    Ok(grouped
        .into_iter()
        .filter(|(_, (early_count, _, later_count, _, _))| *early_count > 0 || *later_count > 0)
        .map(
            |(seed_id, (early_count, early_max, later_count, later_positive, source_rows))| {
                TimeSplitRow {
                    seed_id,
                    split: "clinicaltrials_start_year".to_string(),
                    cutoff_year,
                    early_evidence_count: early_count,
                    early_max_score: early_max,
                    later_evidence_count: later_count,
                    later_positive,
                    source_row_ids: source_rows,
                }
            },
        )
        .collect())
}

fn manifest(family: &str, root: &Path, required: &[&str]) -> CliResult<SourceManifest> {
    if !root.is_dir() {
        return Err(CliError::runtime(format!(
            "{family} source root is not a directory: {}",
            root.display()
        )));
    }
    let mut files = Vec::new();
    collect_files(root, root, &mut files)?;
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let mut aggregate = Sha256::new();
    let mut byte_count = 0_u64;
    for (rel, path) in &files {
        let bytes = fs::read(path)?;
        byte_count += bytes.len() as u64;
        aggregate.update(rel.as_bytes());
        aggregate.update([0]);
        aggregate.update(sha256_hex(&bytes).as_bytes());
        aggregate.update([0]);
    }
    let mut present = Vec::new();
    for rel in required {
        let path = root.join(rel);
        if !path.is_file() {
            return Err(CliError::runtime(format!(
                "{family} required source file missing: {}",
                path.display()
            )));
        }
        present.push((*rel).to_string());
    }
    Ok(SourceManifest {
        family: family.to_string(),
        root: root.display().to_string(),
        file_count: files.len(),
        byte_count,
        aggregate_sha256: super::model::sha256_hex(&aggregate.finalize()),
        required_files_present: present,
    })
}

fn collect_files(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) -> CliResult {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_files(root, &path, out)?;
        } else if path.is_file() {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            out.push((rel, path));
        }
    }
    Ok(())
}

fn read_jsonl(root: &Path, rel: &str) -> CliResult<Vec<JsonlRow>> {
    let path = root.join(rel);
    let text = fs::read_to_string(&path)
        .map_err(|error| CliError::io(format!("read {}: {error}", path.display())))?;
    let mut rows = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let value = serde_json::from_str::<Value>(line).map_err(|error| {
            CliError::runtime(format!(
                "parse {} line {}: {error}",
                path.display(),
                idx + 1
            ))
        })?;
        rows.push(JsonlRow {
            rel: rel.to_string(),
            line_no: idx + 1,
            line_sha256: sha256_hex(line.as_bytes()),
            value,
        });
    }
    Ok(rows)
}

fn str_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn bool_field(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn number(value: &Value, key: &str) -> f64 {
    value
        .get(key)
        .and_then(|raw| raw.as_f64().or_else(|| raw.as_i64().map(|v| v as f64)))
        .unwrap_or(0.0)
}

fn string_number(value: &Value, key: &str) -> f64 {
    value
        .get(key)
        .and_then(Value::as_str)
        .and_then(|raw| raw.parse::<f64>().ok())
        .unwrap_or_else(|| number(value, key))
}

fn array_len(value: &Value, key: &str) -> usize {
    value.get(key).and_then(Value::as_array).map_or(0, Vec::len)
}
