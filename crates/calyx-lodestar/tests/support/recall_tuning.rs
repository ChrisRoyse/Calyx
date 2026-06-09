use std::collections::BTreeSet;

use calyx_core::{CxId, content_address};
use calyx_lodestar::RecallReport;
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecallPassMode {
    Raw,
    Tuned,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RecallTuningReport {
    pub acceptance_metric: &'static str,
    pub min_recall_ratio: f32,
    pub raw_recall: Option<RecallReport>,
    pub tuned_recall: RecallReport,
    pub added_member_count: usize,
    pub added_member_ids: Vec<CxId>,
    pub added_member_hash: String,
    pub raw_passed: bool,
    pub tuned_passed: bool,
    pub pass_mode: RecallPassMode,
}

pub fn tuning_report(
    raw_recall: Option<&RecallReport>,
    tuned_recall: &RecallReport,
    raw_members: &[CxId],
    tuned_members: &[CxId],
    min_recall_ratio: f32,
) -> RecallTuningReport {
    let added_member_ids = added_members(raw_members, tuned_members);
    let raw_passed = raw_recall
        .map(|report| passes_gate(report, min_recall_ratio))
        .unwrap_or(false);
    let tuned_passed = passes_gate(tuned_recall, min_recall_ratio);
    let pass_mode = if raw_passed {
        RecallPassMode::Raw
    } else if tuned_passed {
        RecallPassMode::Tuned
    } else {
        RecallPassMode::Failed
    };
    RecallTuningReport {
        acceptance_metric: "tuned_recall.ratio",
        min_recall_ratio,
        raw_recall: raw_recall.cloned(),
        tuned_recall: tuned_recall.clone(),
        added_member_count: added_member_ids.len(),
        added_member_hash: member_hash(&added_member_ids),
        added_member_ids,
        raw_passed,
        tuned_passed,
        pass_mode,
    }
}

pub fn passes_gate(report: &RecallReport, min_recall_ratio: f32) -> bool {
    report.warning.is_none() && report.ratio >= min_recall_ratio
}

fn added_members(raw_members: &[CxId], tuned_members: &[CxId]) -> Vec<CxId> {
    let raw: BTreeSet<_> = raw_members.iter().copied().collect();
    tuned_members
        .iter()
        .copied()
        .filter(|member| !raw.contains(member))
        .collect()
}

fn member_hash(members: &[CxId]) -> String {
    let parts = members.iter().map(|member| member.as_bytes().to_vec());
    hex(&content_address(parts))
}

fn hex(bytes: &[u8; 16]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
