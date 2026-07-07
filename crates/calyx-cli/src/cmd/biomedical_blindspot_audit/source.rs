use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use serde_json::Value;

use crate::error::{CliError, CliResult};

use super::io::{read_jsonl, sha256_hex};
use super::model::{
    AuditSources, BiomedicalBlindspotAuditArgs, Candidate, LifecycleEvidence, LiteratureEvidence,
    SourceManifest, StabilityEvidence, TranscriptomicEvidence,
};
use super::util::{
    bool_field, collect_named_entities, first_text, is_drug_type, is_target_type, lower, nonempty,
    norm_key, number_field, push_unique, should_use_typed_name, source_key, str_field,
    string_array, u64_field, uniq,
};

pub(super) fn load_sources(args: &BiomedicalBlindspotAuditArgs) -> CliResult<AuditSources> {
    let mut manifests = Vec::new();
    let (candidates, input_hypothesis_count, candidate_manifests) =
        load_candidates(&args.hypotheses_reports)?;
    manifests.extend(candidate_manifests);

    let (literature_rows, manifest) = read_jsonl("literature_audit", &args.literature_audit)?;
    manifests.push(manifest);
    let (literature_by_id, literature_by_key) = index_literature(&literature_rows);

    let (stability_rows, manifest) = read_jsonl("stability_audit", &args.stability_audit)?;
    manifests.push(manifest);
    let stability_by_id = index_stability(&stability_rows)?;

    let (lifecycle_rows, manifest) = read_jsonl("drug_lifecycle", &args.drug_lifecycle)?;
    manifests.push(manifest);
    let lifecycle_by_drug = index_lifecycle(&lifecycle_rows)?;

    let (transcriptomic_rows, manifest) =
        read_jsonl("transcriptomic_audit", &args.transcriptomic_audit)?;
    manifests.push(manifest);
    let (transcriptomic_by_id, transcriptomic_by_key) = index_transcriptomic(&transcriptomic_rows);

    Ok(AuditSources {
        manifests,
        candidates,
        literature_by_id,
        literature_by_key,
        stability_by_id,
        lifecycle_by_drug,
        transcriptomic_by_id,
        transcriptomic_by_key,
        input_hypothesis_count,
    })
}

fn load_candidates(paths: &[PathBuf]) -> CliResult<(Vec<Candidate>, usize, Vec<SourceManifest>)> {
    let mut manifests = Vec::new();
    let mut input_count = 0;
    let mut deduped = BTreeMap::new();
    for path in paths {
        let bytes = fs::read(path)
            .map_err(|error| CliError::io(format!("read {}: {error}", path.display())))?;
        let sha = sha256_hex(&bytes);
        let parsed: Value = serde_json::from_slice(&bytes).map_err(|error| {
            CliError::runtime(format!(
                "parse hypotheses report {}: {error}",
                path.display()
            ))
        })?;
        let rows = parsed
            .get("hypotheses")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                CliError::runtime(format!(
                    "hypotheses report missing hypotheses array: {}",
                    path.display()
                ))
            })?;
        manifests.push(SourceManifest {
            label: "hypotheses_report".to_string(),
            path: path.display().to_string(),
            bytes: bytes.len() as u64,
            rows: Some(rows.len()),
            sha256: sha,
        });
        for row in rows {
            input_count += 1;
            let candidate = normalize_candidate(row)?;
            if candidate.hypothesis_id.is_empty() {
                return Err(CliError::runtime(format!(
                    "hypothesis row missing hypothesis_id in {}",
                    path.display()
                )));
            }
            deduped
                .entry(candidate.hypothesis_id.clone())
                .or_insert(candidate);
        }
    }
    Ok((deduped.into_values().collect(), input_count, manifests))
}

fn normalize_candidate(row: &Value) -> CliResult<Candidate> {
    let source_name = str_field(row, "source_name");
    let source_type = lower(&str_field(row, "source_type"));
    let target_name = str_field(row, "target_name");
    let target_type = lower(&str_field(row, "target_type"));
    let mut drugs = Vec::new();
    let mut targets = Vec::new();
    let mut diseases = Vec::new();
    collect_named_entities(row, "drug_names", &mut drugs);
    collect_named_entities(row, "target_names", &mut targets);
    collect_named_entities(row, "disease_names", &mut diseases);
    collect_named_entities(row, "therapies", &mut drugs);
    collect_named_entities(row, "drug", &mut drugs);
    collect_named_entities(row, "drug_name", &mut drugs);
    collect_named_entities(row, "disease_name", &mut diseases);
    collect_named_entities(row, "condition", &mut diseases);
    if is_drug_type(&source_type) && should_use_typed_name(&source_name, &drugs) {
        push_unique(&mut drugs, source_name.clone());
    }
    if is_drug_type(&target_type) && should_use_typed_name(&target_name, &drugs) {
        push_unique(&mut drugs, target_name.clone());
    }
    if is_target_type(&source_type) {
        push_unique(&mut targets, source_name.clone());
    }
    if is_target_type(&target_type) {
        push_unique(&mut targets, target_name.clone());
    }
    if source_type == "disease" {
        push_unique(&mut diseases, source_name.clone());
    }
    if target_type == "disease" {
        push_unique(&mut diseases, target_name.clone());
    }
    Ok(Candidate {
        hypothesis_id: str_field(row, "hypothesis_id"),
        source_name,
        source_type,
        target_name,
        target_type,
        drug_names: uniq(drugs),
        target_names: uniq(targets),
        disease_names: uniq(diseases),
        candidate_type: first_text(row, &["candidate_type", "hypothesis_class", "source_class"]),
        evidence_type: first_text(row, &["evidence_type", "source_system", "novelty_class"]),
        score: number_field(row, "score").or_else(|| number_field(row, "rank_score")),
        novelty_score: number_field(row, "novelty_score")
            .or_else(|| number_field(row, "novelty_priority_score")),
        patient_context: first_text(
            row,
            &[
                "patient_context",
                "variant_origin",
                "disease_context",
                "genetic_context",
            ],
        ),
        therapeutic_rationale: first_text(
            row,
            &[
                "therapeutic_rationale",
                "mechanism",
                "mechanism_of_action",
                "rationale",
            ],
        ),
        clinical_boundary: str_field(row, "clinical_boundary"),
        raw: row.clone(),
    })
}

fn index_literature(
    rows: &[Value],
) -> (
    BTreeMap<String, LiteratureEvidence>,
    BTreeMap<String, LiteratureEvidence>,
) {
    let mut by_id = BTreeMap::new();
    let mut by_key = BTreeMap::new();
    for row in rows {
        let evidence = LiteratureEvidence {
            source_system: first_text(row, &["source_system", "source"]),
            publication_count: u64_field(row, "publication_count")
                .or_else(|| u64_field(row, "hit_count"))
                .unwrap_or(0),
            co_mention_count: u64_field(row, "co_mention_count")
                .or_else(|| u64_field(row, "co_mentions"))
                .or_else(|| u64_field(row, "publication_count"))
                .unwrap_or(0),
            query: first_text(row, &["query", "query_string"]),
            source_ids: string_array(row, "source_ids"),
            raw: row.clone(),
        };
        if let Some(id) = nonempty(row, "hypothesis_id") {
            by_id.insert(id, evidence.clone());
        }
        let key = source_key(row);
        if !key.is_empty() {
            by_key.insert(key, evidence);
        }
    }
    (by_id, by_key)
}

fn index_stability(rows: &[Value]) -> CliResult<BTreeMap<String, StabilityEvidence>> {
    let mut out = BTreeMap::new();
    for (idx, row) in rows.iter().enumerate() {
        let id = nonempty(row, "hypothesis_id").ok_or_else(|| {
            CliError::runtime(format!(
                "stability_audit line {} missing hypothesis_id",
                idx + 1
            ))
        })?;
        let run_count = u64_field(row, "run_count").unwrap_or(0);
        let present_count = u64_field(row, "present_count").unwrap_or(0);
        let frequency = number_field(row, "frequency").unwrap_or_else(|| {
            if run_count == 0 {
                0.0
            } else {
                present_count as f64 / run_count as f64
            }
        });
        out.insert(
            id,
            StabilityEvidence {
                run_count,
                present_count,
                frequency,
                corpus_count: u64_field(row, "corpus_count"),
                seed_count: u64_field(row, "seed_count"),
                raw: row.clone(),
            },
        );
    }
    Ok(out)
}

fn index_lifecycle(rows: &[Value]) -> CliResult<BTreeMap<String, LifecycleEvidence>> {
    let mut out = BTreeMap::new();
    for (idx, row) in rows.iter().enumerate() {
        let name = first_text(row, &["drug_name", "pref_name", "molecule_name", "name"]);
        if name.is_empty() {
            return Err(CliError::runtime(format!(
                "drug_lifecycle line {} missing drug_name/name",
                idx + 1
            )));
        }
        let evidence = LifecycleEvidence {
            drug_name: name.clone(),
            max_phase: number_field(row, "max_phase"),
            lifecycle_status: lower(&first_text(
                row,
                &["lifecycle_status", "development_status"],
            )),
            trial_status: lower(&first_text(row, &["trial_status", "overall_status"])),
            integrity_status: lower(&first_text(row, &["integrity_status", "scientific_status"])),
            withdrawn_flag: bool_field(row, "withdrawn_flag"),
            source_system: first_text(row, &["source_system", "source"]),
            raw: row.clone(),
        };
        out.insert(norm_key(&name), evidence.clone());
        if let Some(id) = nonempty(row, "chembl_id").or_else(|| nonempty(row, "drug_id")) {
            out.insert(norm_key(&id), evidence);
        }
    }
    Ok(out)
}

fn index_transcriptomic(
    rows: &[Value],
) -> (
    BTreeMap<String, TranscriptomicEvidence>,
    BTreeMap<String, TranscriptomicEvidence>,
) {
    let mut by_id = BTreeMap::new();
    let mut by_key = BTreeMap::new();
    for row in rows {
        let evidence = TranscriptomicEvidence {
            perturbagen_id: first_text(row, &["perturbagen_id", "pert_id", "drug_id"]),
            signature_id: first_text(row, &["signature_id", "sig_id"]),
            cell_context: first_text(row, &["cell_context", "cell_id", "cell_line"]),
            mechanism_class: lower(&first_text(row, &["mechanism_class", "moa_class"])),
            class_breadth: u64_field(row, "class_breadth"),
            is_gold: bool_field(row, "is_gold").or_else(|| bool_field(row, "gold")),
            reproducible: bool_field(row, "reproducible"),
            self_connected: bool_field(row, "self_connected"),
            raw: row.clone(),
        };
        if let Some(id) = nonempty(row, "hypothesis_id") {
            by_id.insert(id, evidence.clone());
        }
        let key = source_key(row);
        if !key.is_empty() {
            by_key.insert(key, evidence);
        }
    }
    (by_id, by_key)
}
