use super::load::LoadedSources;
use super::model::{
    AssociationValidationReport, BenchmarkCounts, GateDecision, GateParams,
    MechanisticDirectionCounts, MetricBlock, REPORT_SCHEMA_VERSION, ScoredOutput,
    ValidationMetrics,
};
use crate::error::{CliError, CliResult};

pub(crate) fn score_and_report(
    params: GateParams,
    loaded: LoadedSources,
) -> CliResult<AssociationValidationReport> {
    let mut scored = Vec::new();
    for row in &loaded.benchmark_rows {
        if let Some(label) = row.label {
            let (score, basis) = source_score(row);
            scored.push(ScoredOutput {
                row_id: row.row_id.clone(),
                benchmark_kind: row.benchmark_kind.clone(),
                seed_id: row.seed_id.clone(),
                label,
                score,
                score_basis: basis,
                source_row_ids: vec![format!(
                    "{}:{}",
                    row.source_path,
                    row.source_line.unwrap_or_default()
                )],
            });
        }
    }
    for row in &loaded.time_split_rows {
        let score = (row.early_max_score / 5.6).clamp(0.0, 1.0);
        scored.push(ScoredOutput {
            row_id: format!("time_split:{}", row.seed_id),
            benchmark_kind: "time_split_clinicaltrials_later_evidence".to_string(),
            seed_id: row.seed_id.clone(),
            label: row.later_positive,
            score,
            score_basis: vec![
                "early_max_trial_evidence_score / 5.6".to_string(),
                format!("cutoff_year={}", row.cutoff_year),
            ],
            source_row_ids: row.source_row_ids.clone(),
        });
    }
    let known_rows = scored
        .iter()
        .filter(|row| !row.benchmark_kind.starts_with("time_split_"))
        .cloned()
        .collect::<Vec<_>>();
    let time_rows = scored
        .iter()
        .filter(|row| row.benchmark_kind.starts_with("time_split_"))
        .cloned()
        .collect::<Vec<_>>();
    let known_metrics = metric_block(&known_rows, params.score_threshold)?;
    let time_metrics = metric_block(&time_rows, params.score_threshold)?;
    let combined_metrics = metric_block(&scored, params.score_threshold)?;
    let counts = BenchmarkCounts {
        known_positive: known_rows.iter().filter(|row| row.label).count(),
        known_negative: known_rows.iter().filter(|row| !row.label).count(),
        time_split: time_rows.len(),
        source_rows: loaded.benchmark_rows.len(),
    };
    let direction_counts = MechanisticDirectionCounts {
        inferred_required_direction_rows: loaded
            .benchmark_rows
            .iter()
            .filter(|row| row.mechanistic_direction.is_some())
            .count(),
        blocked_direction_rows: loaded.mechanistic_direction_blocked_rows.len(),
    };
    let decision = gate_decision(
        &params,
        &counts,
        &direction_counts,
        &known_metrics,
        &time_metrics,
    );
    Ok(AssociationValidationReport {
        schema_version: REPORT_SCHEMA_VERSION,
        status: if decision.passed { "ok" } else { "failed" }.to_string(),
        gate_passed: decision.passed,
        params,
        source_manifests: loaded.manifests,
        benchmark_counts: counts,
        mechanistic_direction_counts: direction_counts,
        benchmark_source_rows: loaded.benchmark_rows,
        mechanistic_direction_blocked_rows: loaded.mechanistic_direction_blocked_rows,
        train_test_split: loaded.time_split_rows,
        scored_outputs: scored,
        metrics: ValidationMetrics {
            known_positive_negative: known_metrics,
            time_split: time_metrics,
            combined: combined_metrics,
        },
        gate_decision: decision,
        clinical_boundary: "Validation gates rank association-mining instruments; they do not prove treatment efficacy, safety, clinical actionability, or cures.".to_string(),
    })
}

fn gate_decision(
    params: &GateParams,
    counts: &BenchmarkCounts,
    direction_counts: &MechanisticDirectionCounts,
    known: &MetricBlock,
    time: &MetricBlock,
) -> GateDecision {
    let mut reasons = Vec::new();
    if counts.known_positive == 0 {
        reasons.push("no known-positive benchmark rows".to_string());
    }
    if counts.known_negative == 0 {
        reasons.push("no known-negative benchmark rows".to_string());
    }
    if counts.time_split == 0 {
        reasons.push("no time-split benchmark rows".to_string());
    }
    if direction_counts.blocked_direction_rows > 0 {
        reasons.push(format!(
            "{} Open Targets rows lacked usable mechanistic direction",
            direction_counts.blocked_direction_rows
        ));
    }
    if known.auroc < params.min_auroc {
        reasons.push(format!(
            "known-positive/negative AUROC {:.3} below {:.3}",
            known.auroc, params.min_auroc
        ));
    }
    if known.positive_recall < params.min_positive_recall {
        reasons.push(format!(
            "known-positive recall {:.3} below {:.3}",
            known.positive_recall, params.min_positive_recall
        ));
    }
    if known.negative_suppression < params.min_negative_suppression {
        reasons.push(format!(
            "known-negative suppression {:.3} below {:.3}",
            known.negative_suppression, params.min_negative_suppression
        ));
    }
    if time.auroc < params.min_auroc {
        reasons.push(format!(
            "time-split AUROC {:.3} below {:.3}",
            time.auroc, params.min_auroc
        ));
    }
    if reasons.is_empty() {
        reasons.push("all configured validation gates passed".to_string());
    }
    GateDecision {
        passed: reasons.len() == 1 && reasons[0] == "all configured validation gates passed",
        reasons,
    }
}

fn source_score(row: &super::model::BenchmarkSourceRow) -> (f64, Vec<String>) {
    match row.benchmark_kind.as_str() {
        "known_positive_pubtator_relation" => {
            let publications = feature(&row.features, "relation_publication_sum");
            let exact = feature(&row.features, "relation_endpoint_exact_match_count");
            let exported = feature(&row.features, "export_docs_with_both_entities");
            let pub_score = (publications.ln_1p() / 1000.0_f64.ln_1p()).clamp(0.0, 1.0);
            let support_score = ((exact + exported) / 12.0).clamp(0.0, 1.0);
            (
                (0.75 * pub_score + 0.25 * support_score).clamp(0.0, 1.0),
                vec![
                    "log1p(relation_publication_sum)/log1p(1000)".to_string(),
                    "(exact_relations + export_docs_with_both)/12".to_string(),
                ],
            )
        }
        "known_positive_dgidb_interaction" => (
            (feature(&row.features, "max_interaction_score") / 6.0).clamp(0.0, 1.0),
            vec!["max_interaction_score / 6".to_string()],
        ),
        "known_negative_dgidb_exact_no_hit" => (
            0.0,
            vec!["exact DGIdb no-hit control scored as suppressed".to_string()],
        ),
        "known_positive_clinical_trial_registry" => {
            let max_score = feature(&row.features, "max_trial_evidence_score") / 5.6;
            let total = feature(&row.features, "total_count");
            let count_score = (total.ln_1p() / 750.0_f64.ln_1p()).clamp(0.0, 1.0);
            (
                (0.7 * max_score + 0.3 * count_score).clamp(0.0, 1.0),
                vec![
                    "max_trial_evidence_score / 5.6".to_string(),
                    "log1p(total_count)/log1p(750)".to_string(),
                ],
            )
        }
        "known_positive_open_targets_target_disease" => (
            feature(&row.features, "open_targets_score").clamp(0.0, 1.0),
            vec!["Open Targets association score".to_string()],
        ),
        _ => (
            0.0,
            vec![format!("unknown benchmark kind {}", row.benchmark_kind)],
        ),
    }
}

fn metric_block(rows: &[ScoredOutput], threshold: f64) -> CliResult<MetricBlock> {
    let positives = rows.iter().filter(|row| row.label).count();
    let negatives = rows.len().saturating_sub(positives);
    if positives == 0 || negatives == 0 {
        return Err(CliError::runtime(format!(
            "validation metric requires both classes; positives={positives} negatives={negatives}"
        )));
    }
    let mut tp = 0;
    let mut fp = 0;
    let mut tn = 0;
    let mut fn_ = 0;
    for row in rows {
        match (row.score >= threshold, row.label) {
            (true, true) => tp += 1,
            (true, false) => fp += 1,
            (false, false) => tn += 1,
            (false, true) => fn_ += 1,
        }
    }
    let precision = ratio(tp, tp + fp);
    let recall = ratio(tp, tp + fn_);
    let suppression = ratio(tn, tn + fp);
    let auroc = auroc(rows);
    let auroc_ci = bootstrap_auroc_ci(rows);
    Ok(MetricBlock {
        n: rows.len(),
        positives,
        negatives,
        threshold,
        tp,
        fp,
        tn,
        fn_,
        precision,
        precision_ci: wilson(tp, tp + fp),
        positive_recall: recall,
        positive_recall_ci: wilson(tp, tp + fn_),
        negative_suppression: suppression,
        negative_suppression_ci: wilson(tn, tn + fp),
        auroc,
        auroc_ci,
    })
}

fn auroc(rows: &[ScoredOutput]) -> f64 {
    let positives = rows
        .iter()
        .filter(|row| row.label)
        .map(|row| row.score)
        .collect::<Vec<_>>();
    let negatives = rows
        .iter()
        .filter(|row| !row.label)
        .map(|row| row.score)
        .collect::<Vec<_>>();
    auroc_from_scores(&positives, &negatives)
}

fn auroc_from_scores(positives: &[f64], negatives: &[f64]) -> f64 {
    let mut wins = 0.0;
    let mut total = 0.0;
    for pos in positives {
        for neg in negatives {
            total += 1.0;
            if pos > neg {
                wins += 1.0;
            } else if (pos - neg).abs() <= f64::EPSILON {
                wins += 0.5;
            }
        }
    }
    if total == 0.0 { 0.0 } else { wins / total }
}

fn bootstrap_auroc_ci(rows: &[ScoredOutput]) -> [f64; 2] {
    let positives = rows
        .iter()
        .filter(|row| row.label)
        .map(|row| row.score)
        .collect::<Vec<_>>();
    let negatives = rows
        .iter()
        .filter(|row| !row.label)
        .map(|row| row.score)
        .collect::<Vec<_>>();
    let mut values = Vec::with_capacity(200);
    for b in 0..200_usize {
        let pos = resample(&positives, b.wrapping_mul(17).wrapping_add(3));
        let neg = resample(&negatives, b.wrapping_mul(31).wrapping_add(7));
        values.push(auroc_from_scores(&pos, &neg));
    }
    values.sort_by(|a, b| a.total_cmp(b));
    [values[4], values[194]]
}

fn resample(values: &[f64], seed: usize) -> Vec<f64> {
    (0..values.len())
        .map(|i| values[(seed.wrapping_add(i.wrapping_mul(1103515245))) % values.len()])
        .collect()
}

fn wilson(success: usize, total: usize) -> [f64; 2] {
    if total == 0 {
        return [0.0, 0.0];
    }
    let n = total as f64;
    let phat = success as f64 / n;
    let z = 1.96;
    let denom = 1.0 + z * z / n;
    let center = (phat + z * z / (2.0 * n)) / denom;
    let margin = z * ((phat * (1.0 - phat) + z * z / (4.0 * n)) / n).sqrt() / denom;
    [(center - margin).max(0.0), (center + margin).min(1.0)]
}

fn ratio(num: usize, den: usize) -> f64 {
    if den == 0 {
        0.0
    } else {
        num as f64 / den as f64
    }
}

fn feature(value: &serde_json::Value, key: &str) -> f64 {
    value
        .get(key)
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0)
}
