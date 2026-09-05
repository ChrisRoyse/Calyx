//! Bounded association fan-out selection (issue #74).
//!
//! This module does not compute NMI, Spearman, edge weights, TE, dCor, or permutation tests. It is
//! the auditable fan-out boundary between cheap screens and expensive confirmers: callers provide
//! already-computed cheap scores, and Poly persists exactly which pairs are selected or dropped.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{PolyError, Result};

pub const FANOUT_SELECTION_SCHEMA_VERSION: &str = "poly.fanout_selection.v1";
pub const FANOUT_SELECTION_ARTIFACT_KIND: &str = "poly_fanout_selection";

pub const ERR_FANOUT_SELECTION_INVALID_REQUEST: &str =
    "CALYX_POLY_FANOUT_SELECTION_INVALID_REQUEST";
pub const ERR_FANOUT_SELECTION_EMPTY: &str = "CALYX_POLY_FANOUT_SELECTION_EMPTY";
pub const ERR_FANOUT_SELECTION_INVALID_CANDIDATE: &str =
    "CALYX_POLY_FANOUT_SELECTION_INVALID_CANDIDATE";
pub const ERR_FANOUT_SELECTION_DUPLICATE_PAIR: &str = "CALYX_POLY_FANOUT_SELECTION_DUPLICATE_PAIR";
pub const ERR_FANOUT_SELECTION_READBACK_MISMATCH: &str =
    "CALYX_POLY_FANOUT_SELECTION_READBACK_MISMATCH";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpensiveAssociationEstimator {
    TransferEntropy,
    DistanceCorrelation,
    PermutationConfirm,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FanoutCandidate {
    pub pair_id: String,
    pub left_key: String,
    pub right_key: String,
    pub normalized_mutual_info: f64,
    pub abs_spearman: f64,
    pub edge_weight: f64,
    pub provenance: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FanoutThresholds {
    pub min_normalized_mutual_info: f64,
    pub min_abs_spearman: f64,
    pub min_edge_weight: f64,
    pub max_expensive_candidates: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FanoutSelectionRequest {
    pub domain: String,
    pub panel_version: u32,
    pub thresholds: FanoutThresholds,
    pub expensive_estimators: Vec<ExpensiveAssociationEstimator>,
    pub candidates: Vec<FanoutCandidate>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FanoutDecisionKind {
    SelectedForExpensiveConfirm,
    Dropped,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FanoutDropReason {
    BelowCheapScreen,
    FanoutLimit,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FanoutDecision {
    pub pair_id: String,
    pub left_key: String,
    pub right_key: String,
    pub normalized_mutual_info: f64,
    pub abs_spearman: f64,
    pub edge_weight: f64,
    pub cheap_score: f64,
    pub pass_nmi: bool,
    pub pass_abs_spearman: bool,
    pub pass_edge_weight: bool,
    pub expensive_rank: Option<usize>,
    pub decision: FanoutDecisionKind,
    pub drop_reason: Option<FanoutDropReason>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FanoutSelectionReport {
    pub schema_version: String,
    pub artifact_kind: String,
    pub source_of_truth: String,
    pub domain: String,
    pub panel_version: u32,
    pub thresholds: FanoutThresholds,
    pub expensive_estimators: Vec<ExpensiveAssociationEstimator>,
    pub input_count: usize,
    pub selected_count: usize,
    pub dropped_count: usize,
    pub input_fingerprint: String,
    pub selected: Vec<FanoutDecision>,
    pub dropped: Vec<FanoutDecision>,
    pub decisions: Vec<FanoutDecision>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FanoutSelectionRun {
    pub report_path: PathBuf,
    pub report: FanoutSelectionReport,
}

pub fn run_fanout_selection_report(
    output_root: &Path,
    request: &FanoutSelectionRequest,
) -> Result<FanoutSelectionRun> {
    let report = compute_fanout_selection_report(request)?;
    let report_path = write_fanout_selection_report(output_root, &report)?;
    let expected = serde_json::to_vec_pretty(&report).map_err(|err| {
        PolyError::diagnostics(
            ERR_FANOUT_SELECTION_INVALID_REQUEST,
            format!("encode fan-out report for readback check: {err}"),
        )
    })?;
    let actual = fs::read(&report_path).map_err(|err| {
        PolyError::diagnostics(
            ERR_FANOUT_SELECTION_INVALID_REQUEST,
            format!("read fan-out report bytes {}: {err}", report_path.display()),
        )
    })?;
    if actual != expected {
        return Err(PolyError::diagnostics(
            ERR_FANOUT_SELECTION_READBACK_MISMATCH,
            format!(
                "fan-out selection report {} bytes did not read back as written",
                report_path.display()
            ),
        ));
    }
    let readback = read_fanout_selection_report(&report_path)?;
    Ok(FanoutSelectionRun {
        report_path,
        report: readback,
    })
}

pub fn compute_fanout_selection_report(
    request: &FanoutSelectionRequest,
) -> Result<FanoutSelectionReport> {
    validate_request(request)?;
    validate_candidates(&request.candidates)?;

    let mut passers = Vec::new();
    let mut decisions = Vec::new();
    for candidate in &request.candidates {
        let scored = score_candidate(candidate, request.thresholds);
        if scored.passes_any() {
            passers.push(scored);
        } else {
            decisions.push(scored.into_decision(
                None,
                FanoutDecisionKind::Dropped,
                Some(FanoutDropReason::BelowCheapScreen),
            ));
        }
    }
    passers.sort_by(|a, b| {
        b.cheap_score
            .total_cmp(&a.cheap_score)
            .then_with(|| a.candidate.pair_id.cmp(&b.candidate.pair_id))
    });

    let mut selected = Vec::new();
    let mut dropped_by_cap = Vec::new();
    for (index, scored) in passers.into_iter().enumerate() {
        let rank = index + 1;
        if selected.len() < request.thresholds.max_expensive_candidates {
            selected.push(scored.into_decision(
                Some(rank),
                FanoutDecisionKind::SelectedForExpensiveConfirm,
                None,
            ));
        } else {
            dropped_by_cap.push(scored.into_decision(
                Some(rank),
                FanoutDecisionKind::Dropped,
                Some(FanoutDropReason::FanoutLimit),
            ));
        }
    }
    decisions.extend(selected.iter().cloned());
    decisions.extend(dropped_by_cap.iter().cloned());
    decisions.sort_by(|a, b| a.pair_id.cmp(&b.pair_id));

    let mut dropped: Vec<_> = decisions
        .iter()
        .filter(|d| d.decision == FanoutDecisionKind::Dropped)
        .cloned()
        .collect();
    dropped.sort_by(|a, b| a.pair_id.cmp(&b.pair_id));

    Ok(FanoutSelectionReport {
        schema_version: FANOUT_SELECTION_SCHEMA_VERSION.to_string(),
        artifact_kind: FANOUT_SELECTION_ARTIFACT_KIND.to_string(),
        source_of_truth: "persisted fan-out selection report over cheap association-screen scores"
            .to_string(),
        domain: request.domain.clone(),
        panel_version: request.panel_version,
        thresholds: request.thresholds,
        expensive_estimators: request.expensive_estimators.clone(),
        input_count: request.candidates.len(),
        selected_count: selected.len(),
        dropped_count: dropped.len(),
        input_fingerprint: input_fingerprint(request)?,
        selected,
        dropped,
        decisions,
    })
}

pub fn write_fanout_selection_report(
    dir: &Path,
    report: &FanoutSelectionReport,
) -> Result<PathBuf> {
    let file_name = format!(
        "fanout_selection_{}_v{}.json",
        sanitize(&report.domain),
        report.panel_version
    );
    crate::diagnostics_store::write_json(dir, &file_name, report)
}

pub fn read_fanout_selection_report(path: &Path) -> Result<FanoutSelectionReport> {
    crate::diagnostics_store::read_json(path)
}

fn validate_request(request: &FanoutSelectionRequest) -> Result<()> {
    if request.domain.trim().is_empty()
        || request.panel_version == 0
        || request.expensive_estimators.is_empty()
    {
        return Err(PolyError::diagnostics(
            ERR_FANOUT_SELECTION_INVALID_REQUEST,
            "domain, positive panel_version, and expensive_estimators are required",
        ));
    }
    let t = request.thresholds;
    if t.max_expensive_candidates == 0
        || !valid_unit_threshold(t.min_normalized_mutual_info)
        || !valid_unit_threshold(t.min_abs_spearman)
        || !valid_unit_threshold(t.min_edge_weight)
        || (t.min_normalized_mutual_info == 0.0
            && t.min_abs_spearman == 0.0
            && t.min_edge_weight == 0.0)
    {
        return Err(PolyError::diagnostics(
            ERR_FANOUT_SELECTION_INVALID_REQUEST,
            "thresholds must be finite in [0,1], at least one must be enabled, and max must be > 0",
        ));
    }
    if request.candidates.is_empty() {
        return Err(PolyError::diagnostics(
            ERR_FANOUT_SELECTION_EMPTY,
            "fan-out selection requires at least one candidate pair",
        ));
    }
    Ok(())
}

fn validate_candidates(candidates: &[FanoutCandidate]) -> Result<()> {
    let mut pairs = HashSet::new();
    for (index, candidate) in candidates.iter().enumerate() {
        validate_label(index, "pair_id", &candidate.pair_id)?;
        validate_label(index, "left_key", &candidate.left_key)?;
        validate_label(index, "right_key", &candidate.right_key)?;
        validate_label(index, "provenance", &candidate.provenance)?;
        if candidate.left_key == candidate.right_key {
            return invalid_candidate(index, "left_key and right_key must differ");
        }
        if !valid_unit_score(candidate.normalized_mutual_info)
            || !valid_unit_score(candidate.abs_spearman)
            || !valid_unit_score(candidate.edge_weight)
        {
            return invalid_candidate(index, "cheap scores must be finite in [0,1]");
        }
        let key = ordered_pair(&candidate.left_key, &candidate.right_key);
        if !pairs.insert(key) {
            return Err(PolyError::diagnostics(
                ERR_FANOUT_SELECTION_DUPLICATE_PAIR,
                format!(
                    "row {index} duplicates pair {}:{}",
                    candidate.left_key, candidate.right_key
                ),
            ));
        }
    }
    Ok(())
}

fn validate_label(index: usize, field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return invalid_candidate(index, &format!("{field} is required"));
    }
    Ok(())
}

fn invalid_candidate(index: usize, message: &str) -> Result<()> {
    Err(PolyError::diagnostics(
        ERR_FANOUT_SELECTION_INVALID_CANDIDATE,
        format!("candidate row {index}: {message}"),
    ))
}

fn valid_unit_threshold(value: f64) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}

fn valid_unit_score(value: f64) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}

fn ordered_pair(left: &str, right: &str) -> (String, String) {
    if left <= right {
        (left.to_string(), right.to_string())
    } else {
        (right.to_string(), left.to_string())
    }
}

struct ScoredCandidate<'a> {
    candidate: &'a FanoutCandidate,
    cheap_score: f64,
    pass_nmi: bool,
    pass_abs_spearman: bool,
    pass_edge_weight: bool,
}

impl ScoredCandidate<'_> {
    fn passes_any(&self) -> bool {
        self.pass_nmi || self.pass_abs_spearman || self.pass_edge_weight
    }

    fn into_decision(
        self,
        expensive_rank: Option<usize>,
        decision: FanoutDecisionKind,
        drop_reason: Option<FanoutDropReason>,
    ) -> FanoutDecision {
        FanoutDecision {
            pair_id: self.candidate.pair_id.clone(),
            left_key: self.candidate.left_key.clone(),
            right_key: self.candidate.right_key.clone(),
            normalized_mutual_info: self.candidate.normalized_mutual_info,
            abs_spearman: self.candidate.abs_spearman,
            edge_weight: self.candidate.edge_weight,
            cheap_score: self.cheap_score,
            pass_nmi: self.pass_nmi,
            pass_abs_spearman: self.pass_abs_spearman,
            pass_edge_weight: self.pass_edge_weight,
            expensive_rank,
            decision,
            drop_reason,
        }
    }
}

fn score_candidate(
    candidate: &FanoutCandidate,
    thresholds: FanoutThresholds,
) -> ScoredCandidate<'_> {
    let pass_nmi = enabled_pass(
        candidate.normalized_mutual_info,
        thresholds.min_normalized_mutual_info,
    );
    let pass_abs_spearman = enabled_pass(candidate.abs_spearman, thresholds.min_abs_spearman);
    let pass_edge_weight = enabled_pass(candidate.edge_weight, thresholds.min_edge_weight);
    let cheap_score = ratio(
        candidate.normalized_mutual_info,
        thresholds.min_normalized_mutual_info,
    )
    .max(ratio(candidate.abs_spearman, thresholds.min_abs_spearman))
    .max(ratio(candidate.edge_weight, thresholds.min_edge_weight));
    ScoredCandidate {
        candidate,
        cheap_score,
        pass_nmi,
        pass_abs_spearman,
        pass_edge_weight,
    }
}

fn enabled_pass(value: f64, threshold: f64) -> bool {
    threshold > 0.0 && value >= threshold
}

fn ratio(value: f64, threshold: f64) -> f64 {
    if threshold > 0.0 {
        value / threshold
    } else {
        0.0
    }
}

fn input_fingerprint(request: &FanoutSelectionRequest) -> Result<String> {
    let bytes = serde_json::to_vec(request).map_err(|err| {
        PolyError::diagnostics(
            ERR_FANOUT_SELECTION_INVALID_REQUEST,
            format!("encode fan-out selection input fingerprint: {err}"),
        )
    })?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

fn sanitize(domain: &str) -> String {
    domain
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}
