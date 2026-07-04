use std::cmp::Ordering;
use std::collections::BTreeMap;

use serde_json::Value;

use super::model::{
    CLINICAL_BOUNDARY, CalibrationFlag, EvidenceShape, LoadedAtlas, SCHEMA_VERSION,
    ScoreComponents, SlimRow, SplitRow,
};
use super::row_access::{
    all_text, candidate_id, display_names, endpoint_type, falsification_summary,
    has_dgidb_clinical_source, max_named_score, original_rank, score_float, source_class,
    str_field,
};
use crate::error::CliResult;

pub(super) fn build_split_rows(atlases: &[LoadedAtlas]) -> CliResult<Vec<SplitRow>> {
    let mut rows = Vec::new();
    for atlas in atlases {
        let total = atlas.rows.len().max(1);
        for (idx, row) in atlas.rows.iter().enumerate() {
            let source_row_index = idx + 1;
            let original_rank = original_rank(row).unwrap_or(source_row_index);
            let percentile = original_percentile(original_rank, total);
            let flags = detect_calibration(row);
            let shape = evidence_shape(row);
            let class = source_class(row);
            let components = score_components(row, percentile, !flags.is_empty(), &shape, &class);
            rows.push(SplitRow {
                schema_version: SCHEMA_VERSION,
                issue: atlas.arg.issue.clone(),
                domain: atlas.arg.domain.clone(),
                source_row_index,
                source_artifact: atlas.arg.path.display().to_string(),
                source_artifact_sha256: atlas.sha256.clone(),
                candidate_id: candidate_id(&atlas.arg.issue, source_row_index, row),
                display_names: display_names(row),
                source_class: class,
                original_rank,
                original_rank_score: score_float(row),
                original_rank_percentile_within_domain: round9(percentile),
                calibration_known_positive: !flags.is_empty(),
                calibration_flags: flags,
                evidence_shape: shape,
                novelty_priority_score: novelty_score(&components),
                score_components: components,
                falsification_summary: falsification_summary(row)?,
                combined_original_order: None,
                novelty_rank: None,
                calibration_rank: None,
                original_row: row.clone(),
                clinical_boundary: CLINICAL_BOUNDARY.to_string(),
            });
        }
    }
    Ok(rows)
}

pub(super) fn combined_original(mut rows: Vec<SplitRow>) -> Vec<SplitRow> {
    rows.sort_by(|a, b| {
        cmp_f64_desc(
            a.original_rank_percentile_within_domain,
            b.original_rank_percentile_within_domain,
        )
        .then(a.issue.cmp(&b.issue))
        .then(a.source_row_index.cmp(&b.source_row_index))
    });
    for (idx, row) in rows.iter_mut().enumerate() {
        row.combined_original_order = Some(idx + 1);
    }
    rows
}

pub(super) fn split_views(rows: &[SplitRow]) -> (Vec<SplitRow>, Vec<SplitRow>) {
    let mut calibration = rows
        .iter()
        .filter(|row| row.calibration_known_positive)
        .cloned()
        .collect::<Vec<_>>();
    let mut novelty = rows
        .iter()
        .filter(|row| !row.calibration_known_positive)
        .cloned()
        .collect::<Vec<_>>();
    novelty.sort_by(|a, b| {
        cmp_f64_desc(a.novelty_priority_score, b.novelty_priority_score)
            .then(a.issue.cmp(&b.issue))
            .then(a.source_row_index.cmp(&b.source_row_index))
    });
    for (idx, row) in novelty.iter_mut().enumerate() {
        row.novelty_rank = Some(idx + 1);
    }
    for (idx, row) in calibration.iter_mut().enumerate() {
        row.calibration_rank = Some(idx + 1);
    }
    (calibration, novelty)
}

pub(super) fn slim(row: &SplitRow) -> SlimRow {
    SlimRow {
        issue: row.issue.clone(),
        domain: row.domain.clone(),
        candidate_id: row.candidate_id.clone(),
        display_names: row.display_names.clone(),
        source_class: row.source_class.clone(),
        original_rank: row.original_rank,
        original_rank_score: row.original_rank_score,
        original_rank_percentile_within_domain: row.original_rank_percentile_within_domain,
        calibration_known_positive: row.calibration_known_positive,
        calibration_flag_codes: row
            .calibration_flags
            .iter()
            .map(|flag| flag.code.clone())
            .collect(),
        novelty_priority_score: row.novelty_priority_score,
        score_components: row.score_components.clone(),
    }
}

pub(super) fn flag_counts(rows: &[SplitRow]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for flag in rows.iter().flat_map(|row| row.calibration_flags.iter()) {
        *counts.entry(flag.code.clone()).or_insert(0) += 1;
    }
    counts
}

pub(super) fn issue_counts(rows: &[SplitRow]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for row in rows {
        *counts.entry(row.issue.clone()).or_insert(0) += 1;
    }
    counts
}

pub(super) fn text_blob(row: &SplitRow) -> String {
    let mut text = row.candidate_id.clone();
    text.push(' ');
    text.push_str(&row.display_names.join(" "));
    text.to_lowercase()
}

fn detect_calibration(row: &Value) -> Vec<CalibrationFlag> {
    let mut flags = Vec::new();
    let source_type = endpoint_type(row, "source");
    let target_type = endpoint_type(row, "target");
    if let (Some(source), Some(target)) = (source_type.as_deref(), target_type.as_deref())
        && source != "disease"
        && target != "disease"
    {
        flags.push(flag(
            "non_disease_typed_pair_not_novelty_lead",
            "medium",
            Some(format!("{source} to {target} pair has no disease endpoint")),
            None,
        ));
    }
    let level = str_field(row, "evidence_level")
        .unwrap_or_default()
        .to_ascii_uppercase();
    let external_source = row
        .get("external_validation")
        .and_then(|value| value.get("source"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if external_source == "civic" && matches!(level.as_str(), "A" | "B") {
        flags.push(flag(
            "civic_level_a_b_external_evidence",
            "high",
            Some(format!("CIViC evidence level {level}")),
            None,
        ));
    }
    if external_source == "civic"
        && matches!(level.as_str(), "A" | "B")
        && row
            .get("nct_ids")
            .and_then(Value::as_array)
            .is_some_and(|ids| !ids.is_empty())
    {
        flags.push(flag(
            "civic_level_a_b_trial_id_present",
            "high",
            Some("CIViC high-level row includes trial identifiers".to_string()),
            None,
        ));
    }
    add_score_flag(
        &mut flags,
        row,
        &["clinical_precedence"],
        0.75,
        "open_targets_clinical_precedence_ge_0_75",
        "high",
        false,
    );
    add_score_flag(
        &mut flags,
        row,
        &["clinical", "known_drug"],
        0.75,
        "open_targets_clinical_datatype_ge_0_75",
        "high",
        false,
    );
    add_score_flag(
        &mut flags,
        row,
        &[
            "genetic_association",
            "somatic_mutation",
            "genetic_literature",
            "uniprot_variants",
            "gene_burden",
        ],
        0.85,
        "open_targets_genetic_or_somatic_validation_ge_0_85",
        "medium",
        source_class(row).contains("open_targets"),
    );
    let clinical_precedence = max_named_score(row, &["clinical_precedence"]);
    let clinical_datatype = max_named_score(row, &["clinical", "known_drug"]);
    if has_dgidb_clinical_source(row) && (clinical_precedence >= 0.50 || clinical_datatype >= 0.50)
    {
        flags.push(flag(
            "dgidb_clinical_source_plus_open_targets_context",
            "medium",
            None,
            Some(clinical_precedence.max(clinical_datatype)),
        ));
    }
    flags
}

fn add_score_flag(
    flags: &mut Vec<CalibrationFlag>,
    row: &Value,
    ids: &[&str],
    threshold: f64,
    code: &str,
    strength: &str,
    enabled: bool,
) {
    if !enabled && code.contains("genetic") {
        return;
    }
    let score = max_named_score(row, ids);
    if score >= threshold {
        flags.push(flag(code, strength, None, Some(score)));
    }
}

fn evidence_shape(row: &Value) -> EvidenceShape {
    let text = all_text(row).to_ascii_lowercase();
    EvidenceShape {
        has_open_targets: text.contains("open_targets") || text.contains("open targets"),
        has_dgidb: text.contains("dgidb"),
        has_civic: text.contains("civic"),
        has_trial_context: text.contains("clinicaltrials")
            || text.contains("clinicaltrials.gov")
            || row
                .get("nct_ids")
                .and_then(Value::as_array)
                .is_some_and(|ids| !ids.is_empty()),
        has_openfda_context: text.contains("openfda") || text.contains("fda"),
        has_generated_after_falsification: text.contains("not_run_for_generated")
            || text.contains("generated_after_issue1184"),
        has_counterevidence: text.contains("counter")
            && !text.contains("no_counter")
            && !text.contains("counterevidence_found_in_current_sources"),
    }
}

fn score_components(
    row: &Value,
    percentile: f64,
    calibration: bool,
    shape: &EvidenceShape,
    class: &str,
) -> ScoreComponents {
    let mut bridge_bonus = 0.0;
    if contains_any(class, &["bridge", "molecular", "dgidb"]) {
        bridge_bonus += 0.08;
    }
    if contains_any(class, &["normalized_same_source", "comention", "cluster"]) {
        bridge_bonus += 0.05;
    }
    if shape.has_open_targets {
        bridge_bonus += 0.025;
    }
    if shape.has_dgidb {
        bridge_bonus += 0.025;
    }
    let mut falsification_penalty = 0.0;
    if shape.has_generated_after_falsification {
        falsification_penalty += 0.08;
    }
    if shape.has_counterevidence {
        falsification_penalty += 0.20;
    }
    let text = all_text(row).to_ascii_lowercase();
    let mut safety_penalty = 0.0;
    if text.contains("pending_live_triage") || text.contains("safety_triage_pending") {
        safety_penalty += 0.04;
    }
    if text.contains("source unavailable fail-closed")
        || text.contains("label unavailable fail-closed")
    {
        safety_penalty += 0.10;
    }
    ScoreComponents {
        original_rank_percentile_weighted: round9(0.72 * percentile),
        bridge_external_context_bonus: round9(bridge_bonus),
        generated_or_unfalsified_penalty: round9(falsification_penalty),
        safety_trial_gap_penalty: round9(safety_penalty),
        calibration_view_exclusion_penalty: if calibration { 0.45 } else { 0.0 },
    }
}

fn novelty_score(components: &ScoreComponents) -> f64 {
    round9(
        (components.original_rank_percentile_weighted + components.bridge_external_context_bonus
            - components.generated_or_unfalsified_penalty
            - components.safety_trial_gap_penalty
            - components.calibration_view_exclusion_penalty)
            .max(0.0),
    )
}

fn original_percentile(rank: usize, total: usize) -> f64 {
    if total == 1 {
        1.0
    } else {
        (1.0 - ((rank.saturating_sub(1)) as f64 / (total - 1) as f64)).clamp(0.0, 1.0)
    }
}

fn flag(code: &str, strength: &str, detail: Option<String>, score: Option<f64>) -> CalibrationFlag {
    CalibrationFlag {
        code: code.to_string(),
        strength: strength.to_string(),
        detail,
        score: score.map(round9),
    }
}

fn contains_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}

fn cmp_f64_desc(left: f64, right: f64) -> Ordering {
    right.total_cmp(&left)
}

fn round9(value: f64) -> f64 {
    (value * 1_000_000_000.0).round() / 1_000_000_000.0
}
