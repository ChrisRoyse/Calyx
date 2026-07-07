use std::collections::{BTreeMap, BTreeSet};

use super::model::{
    AuditSources, AuditedHypothesis, BenchmarkExportRow, BiomedicalBlindspotAuditArgs,
    CLINICAL_BOUNDARY, Candidate, CorrelationMetric, Metrics,
};
use super::util::{
    candidate_key, contains_any, is_generic_mechanism_class, lower, norm_key, pearson,
};

pub(super) fn audit_candidate(
    candidate: &Candidate,
    sources: &AuditSources,
    args: &BiomedicalBlindspotAuditArgs,
) -> AuditedHypothesis {
    let mut blockers = BTreeSet::new();
    let mut pending = BTreeSet::new();
    let mut warnings = BTreeSet::new();

    let patient_context_status = patient_context_check(candidate, &mut blockers, &mut pending);
    let lifecycle_statuses = lifecycle_check(candidate, sources, &mut blockers, &mut pending);
    let (novelty_class, literature_count) =
        literature_check(candidate, sources, args, &mut pending, &mut warnings);
    let stability_frequency =
        stability_check(candidate, sources, args, &mut blockers, &mut pending);
    let transcriptomic_status =
        transcriptomic_check(candidate, sources, args, &mut blockers, &mut pending);

    let benchmark_exportable = !candidate.disease_names.is_empty()
        && (!candidate.target_names.is_empty() || !candidate.drug_names.is_empty());
    if !benchmark_exportable {
        blockers.insert("CALYX_BLINDSPOT_BENCHMARK_FIELDS_MISSING".to_string());
    }

    let novelty_promotion_allowed = novelty_class == "no_literature_co_mentions"
        && !pending.contains("CALYX_BLINDSPOT_LITERATURE_AUDIT_MISSING");
    let final_status = if !blockers.is_empty() {
        "blocked_by_blindspot_audit"
    } else if !pending.is_empty() {
        "pending_blindspot_evidence"
    } else {
        "ready_for_human_review_after_blindspot_audit"
    };
    let mut reason_codes = blockers.into_iter().chain(pending).collect::<Vec<_>>();
    reason_codes.sort();
    let mut warning_codes = warnings.into_iter().collect::<Vec<_>>();
    warning_codes.sort();
    AuditedHypothesis {
        hypothesis_id: candidate.hypothesis_id.clone(),
        final_status: final_status.to_string(),
        novelty_promotion_allowed,
        benchmark_exportable,
        reason_codes,
        warning_codes,
        source_name: candidate.source_name.clone(),
        source_type: candidate.source_type.clone(),
        target_name: candidate.target_name.clone(),
        target_type: candidate.target_type.clone(),
        drug_names: candidate.drug_names.clone(),
        target_names: candidate.target_names.clone(),
        disease_names: candidate.disease_names.clone(),
        score: candidate.score,
        novelty_score: candidate.novelty_score,
        external_novelty_class: novelty_class,
        literature_co_mention_count: literature_count,
        stability_frequency,
        drug_lifecycle_statuses: lifecycle_statuses,
        transcriptomic_specificity_status: transcriptomic_status,
        patient_context_status,
        clinical_boundary: CLINICAL_BOUNDARY.to_string(),
    }
}

fn patient_context_check(
    candidate: &Candidate,
    blockers: &mut BTreeSet<String>,
    pending: &mut BTreeSet<String>,
) -> String {
    if candidate.drug_names.is_empty() || candidate.disease_names.is_empty() {
        return "not_drug_disease_context".to_string();
    }
    let context = lower(&candidate.patient_context);
    let rationale = lower(&candidate.therapeutic_rationale);
    if context.is_empty() {
        pending.insert("CALYX_BLINDSPOT_PATIENT_CONTEXT_MISSING".to_string());
        return "missing_patient_context".to_string();
    }
    let germline = contains_any(
        &context,
        &[
            "germline",
            "constitutional",
            "inherited",
            "biallelic",
            "fanconi",
            "every cell",
        ],
    );
    let synthetic_lethal = contains_any(
        &rationale,
        &[
            "synthetic lethality",
            "synthetic lethal",
            "selectively kills",
            "cell killing",
            "brca deficient",
            "dna repair deficient",
        ],
    );
    if germline && synthetic_lethal {
        blockers.insert("CALYX_BLINDSPOT_GERMLINE_SYNTHETIC_LETHALITY_RISK".to_string());
        "germline_synthetic_lethality_blocked".to_string()
    } else if germline {
        "germline_context_checked".to_string()
    } else {
        "patient_context_checked".to_string()
    }
}

fn lifecycle_check(
    candidate: &Candidate,
    sources: &AuditSources,
    blockers: &mut BTreeSet<String>,
    pending: &mut BTreeSet<String>,
) -> Vec<String> {
    if candidate.drug_names.is_empty() {
        return vec!["no_drug_candidate".to_string()];
    }
    let mut statuses = Vec::new();
    for drug in &candidate.drug_names {
        let Some(row) = sources.lifecycle_by_drug.get(&norm_key(drug)) else {
            pending.insert("CALYX_BLINDSPOT_DRUG_LIFECYCLE_MISSING".to_string());
            statuses.push(format!("{drug}:missing_lifecycle"));
            continue;
        };
        let status_text = format!(
            "{} {} {}",
            row.lifecycle_status, row.trial_status, row.integrity_status
        );
        if row.withdrawn_flag == Some(true)
            || contains_any(
                &status_text,
                &[
                    "withdrawn",
                    "discontinued",
                    "terminated",
                    "suspended",
                    "failed",
                    "fraud",
                    "discredited",
                    "unavailable",
                    "revoked",
                ],
            )
        {
            blockers.insert("CALYX_BLINDSPOT_DRUG_NOT_VIABLE".to_string());
        } else if row.lifecycle_status.is_empty() && row.trial_status.is_empty() {
            pending.insert("CALYX_BLINDSPOT_DRUG_LIFECYCLE_INCOMPLETE".to_string());
        }
        statuses.push(format!(
            "{}:{}:{}:{}",
            drug,
            row.lifecycle_status,
            row.trial_status,
            row.max_phase
                .map(|value| value.to_string())
                .unwrap_or_else(|| "phase_unknown".to_string())
        ));
    }
    statuses
}

fn literature_check(
    candidate: &Candidate,
    sources: &AuditSources,
    args: &BiomedicalBlindspotAuditArgs,
    pending: &mut BTreeSet<String>,
    warnings: &mut BTreeSet<String>,
) -> (String, Option<u64>) {
    let evidence = sources
        .literature_by_id
        .get(&candidate.hypothesis_id)
        .or_else(|| sources.literature_by_key.get(&candidate_key(candidate)));
    let Some(evidence) = evidence else {
        pending.insert("CALYX_BLINDSPOT_LITERATURE_AUDIT_MISSING".to_string());
        return ("literature_audit_missing".to_string(), None);
    };
    if evidence.co_mention_count >= args.known_literature_threshold {
        warnings.insert("CALYX_BLINDSPOT_LITERATURE_KNOWN_OR_REDISCOVERY".to_string());
        (
            "known_literature_co_mention".to_string(),
            Some(evidence.co_mention_count),
        )
    } else if evidence.co_mention_count > 0 {
        (
            "weak_literature_co_mention".to_string(),
            Some(evidence.co_mention_count),
        )
    } else {
        (
            "no_literature_co_mentions".to_string(),
            Some(evidence.co_mention_count),
        )
    }
}

fn stability_check(
    candidate: &Candidate,
    sources: &AuditSources,
    args: &BiomedicalBlindspotAuditArgs,
    blockers: &mut BTreeSet<String>,
    pending: &mut BTreeSet<String>,
) -> Option<f64> {
    let Some(evidence) = sources.stability_by_id.get(&candidate.hypothesis_id) else {
        pending.insert("CALYX_BLINDSPOT_STABILITY_AUDIT_MISSING".to_string());
        return None;
    };
    if evidence.run_count < 2 {
        pending.insert("CALYX_BLINDSPOT_STABILITY_REPLICATES_INSUFFICIENT".to_string());
    } else if evidence.frequency < args.min_stability_frequency {
        blockers.insert("CALYX_BLINDSPOT_REPRODUCIBILITY_LOW".to_string());
    }
    Some(evidence.frequency)
}

fn transcriptomic_check(
    candidate: &Candidate,
    sources: &AuditSources,
    args: &BiomedicalBlindspotAuditArgs,
    blockers: &mut BTreeSet<String>,
    pending: &mut BTreeSet<String>,
) -> String {
    let is_reversal = contains_any(
        &lower(&format!(
            "{} {}",
            candidate.candidate_type, candidate.evidence_type
        )),
        &["transcriptomic", "reversal", "lincs", "cmap"],
    );
    if !is_reversal {
        return "not_transcriptomic_reversal".to_string();
    }
    let evidence = sources
        .transcriptomic_by_id
        .get(&candidate.hypothesis_id)
        .or_else(|| sources.transcriptomic_by_key.get(&candidate_key(candidate)));
    let Some(evidence) = evidence else {
        pending.insert("CALYX_BLINDSPOT_TRANSCRIPTOMIC_AUDIT_MISSING".to_string());
        return "transcriptomic_audit_missing".to_string();
    };
    let mut blocked = false;
    if evidence.perturbagen_id.is_empty()
        || evidence.signature_id.is_empty()
        || evidence.cell_context.is_empty()
    {
        blockers.insert("CALYX_BLINDSPOT_TRANSCRIPTOMIC_CONTEXT_MISSING".to_string());
        blocked = true;
    }
    if evidence.is_gold != Some(true)
        || evidence.reproducible != Some(true)
        || evidence.self_connected != Some(true)
    {
        blockers.insert("CALYX_BLINDSPOT_TRANSCRIPTOMIC_NOT_REPRODUCIBLE_GOLD".to_string());
        blocked = true;
    }
    if evidence
        .class_breadth
        .is_some_and(|breadth| breadth > args.max_transcriptomic_class_breadth)
        || is_generic_mechanism_class(&evidence.mechanism_class)
    {
        blockers.insert("CALYX_BLINDSPOT_TRANSCRIPTOMIC_LOW_SPECIFICITY".to_string());
        blocked = true;
    }
    if blocked {
        "transcriptomic_reversal_blocked_low_specificity".to_string()
    } else {
        "transcriptomic_reversal_specificity_passed".to_string()
    }
}

pub(super) fn benchmark_row(
    candidate: &Candidate,
    audit: &AuditedHypothesis,
) -> BenchmarkExportRow {
    BenchmarkExportRow {
        hypothesis_id: candidate.hypothesis_id.clone(),
        disease_names: candidate.disease_names.clone(),
        target_names: candidate.target_names.clone(),
        drug_names: candidate.drug_names.clone(),
        final_status: audit.final_status.clone(),
        novelty_score: candidate.novelty_score,
        external_novelty_class: audit.external_novelty_class.clone(),
        stability_frequency: audit.stability_frequency,
        reason_codes: audit.reason_codes.clone(),
        source_name: candidate.source_name.clone(),
        source_type: candidate.source_type.clone(),
        target_name: candidate.target_name.clone(),
        target_type: candidate.target_type.clone(),
    }
}

pub(super) fn metrics(rows: &[AuditedHypothesis]) -> Metrics {
    let mut status_counts = BTreeMap::new();
    let mut reason_code_counts = BTreeMap::new();
    let mut external_novelty_counts = BTreeMap::new();
    let mut pairs = Vec::new();
    for row in rows {
        *status_counts.entry(row.final_status.clone()).or_insert(0) += 1;
        *external_novelty_counts
            .entry(row.external_novelty_class.clone())
            .or_insert(0) += 1;
        for code in &row.reason_codes {
            *reason_code_counts.entry(code.clone()).or_insert(0) += 1;
        }
        if let (Some(novelty), Some(count)) = (row.novelty_score, row.literature_co_mention_count) {
            pairs.push((novelty, 1.0 / (1.0 + count as f64)));
        }
    }
    let correlation = pearson(&pairs).map(|value| CorrelationMetric {
        n: pairs.len(),
        pearson: value,
    });
    Metrics {
        status_counts,
        reason_code_counts,
        external_novelty_counts,
        novelty_score_external_literature_correlation: correlation,
        novelty_score_external_literature_correlation_status: if pairs.len() >= 2 {
            "computed".to_string()
        } else {
            "insufficient_rows".to_string()
        },
    }
}
