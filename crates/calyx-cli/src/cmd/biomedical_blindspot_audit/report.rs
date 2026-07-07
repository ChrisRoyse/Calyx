use crate::error::{CliError, CliResult};

use super::checks::{audit_candidate, benchmark_row, metrics};
use super::model::{
    AuditParams, AuditReport, BiomedicalBlindspotAuditArgs, CLINICAL_BOUNDARY, SCHEMA_VERSION,
};
use super::source::load_sources;

pub(super) fn build_report(args: &BiomedicalBlindspotAuditArgs) -> CliResult<AuditReport> {
    let sources = load_sources(args)?;
    let mut audited = Vec::new();
    let mut benchmark = Vec::new();
    for candidate in &sources.candidates {
        let row = audit_candidate(candidate, &sources, args);
        benchmark.push(benchmark_row(candidate, &row));
        audited.push(row);
    }
    if audited.is_empty() {
        return Err(CliError::runtime(
            "biomedical-blindspot-audit found no hypotheses to audit",
        ));
    }
    let metrics = metrics(&audited);
    let ready_count = audited
        .iter()
        .filter(|row| row.final_status == "ready_for_human_review_after_blindspot_audit")
        .count();
    let blocked_count = audited
        .iter()
        .filter(|row| row.final_status == "blocked_by_blindspot_audit")
        .count();
    let pending_count = audited
        .iter()
        .filter(|row| row.final_status == "pending_blindspot_evidence")
        .count();
    Ok(AuditReport {
        schema_version: SCHEMA_VERSION,
        status: "ok".to_string(),
        clinical_boundary: CLINICAL_BOUNDARY.to_string(),
        params: AuditParams {
            known_literature_threshold: args.known_literature_threshold,
            min_stability_frequency: args.min_stability_frequency,
            max_transcriptomic_class_breadth: args.max_transcriptomic_class_breadth,
        },
        source_manifests: sources.manifests,
        input_hypothesis_count: sources.input_hypothesis_count,
        deduped_hypothesis_count: sources.candidates.len(),
        audited_count: audited.len(),
        ready_count,
        blocked_count,
        pending_count,
        audited_hypotheses: audited,
        benchmark_export: benchmark,
        metrics,
    })
}
