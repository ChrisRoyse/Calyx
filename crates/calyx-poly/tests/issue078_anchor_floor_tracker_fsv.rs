//! Issue #78 - resolved-anchor floor tracker FSV.
//!
//! Source of truth: local outcome-anchor JSON row files plus the persisted tracker report.

use std::path::{Path, PathBuf};

use calyx_core::{Anchor, AnchorKind, AnchorValue};
use calyx_poly::admission::{
    AdmissionDecision, AdmissionInputs, AdmissionParams, evaluate_admission,
};
use calyx_poly::anchor_floor::{
    AnchorFloorReport, AnchorFloorRequest, AnchorFloorRow, MIN_RESOLVED_ANCHOR_FLOOR,
    read_anchor_floor_report, run_anchor_floor_tracker,
};
use serde_json::{Value, json};

#[path = "fsv_support.rs"]
mod support;
use support::{
    collect_files, known_healthy_market_integrity, known_healthy_oracle_risk,
    known_healthy_wash_trade, named_fsv_root, reset_dir, write_blake3sums, write_json,
};

const DOMAIN: &str = "crypto";
const CROSS_DOMAIN: &str = "sports";
const AXIS: &str = "outcome_yes";

#[test]
fn issue078_anchor_floor_tracker_fsv() {
    let (root, _keep) = named_fsv_root("POLY_ISSUE078_FSV_ROOT", "poly-issue078-anchor-floor");
    reset_dir(&root);

    let happy = happy_exact_floor_passes(&root);
    let zero = edge_zero_anchors_refuses(&root);
    let duplicate = edge_duplicate_anchor_excluded(&root);
    let cross_domain = edge_cross_domain_anchor_excluded(&root);

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let readback = json!({
        "issue": 78,
        "proof_claim": "The anchor-floor tracker counts unique resolved UMA boolean outcome anchors for one domain/outcome axis from local row files and feeds admission refusal when the qualified count is below 50.",
        "minimum_sufficient_corpus": {
            "happy_unique_target_rows": MIN_RESOLVED_ANCHOR_FLOOR,
            "zero_edge_rows": 0,
            "duplicate_edge_source_rows": MIN_RESOLVED_ANCHOR_FLOOR,
            "duplicate_edge_qualified_rows": MIN_RESOLVED_ANCHOR_FLOOR - 1,
            "cross_domain_edge_source_rows": MIN_RESOLVED_ANCHOR_FLOOR,
            "cross_domain_edge_qualified_rows": MIN_RESOLVED_ANCHOR_FLOOR - 1,
            "why_this_is_sufficient": "50 unique target rows is the exact configured domain/axis floor, while 49 qualified rows proves the refusal side after duplicate or cross-domain exclusion.",
            "why_smaller_is_insufficient": "fewer than 50 unique target anchors cannot prove the passing side of the >=50 floor.",
            "why_larger_is_wasteful": "more than 50 unique target anchors would repeat the same row readback, filter, deduplication, and admission-count paths without adding proof for #78."
        },
        "happy_path": happy,
        "edge_cases": {
            "zero_anchors": zero,
            "duplicate_anchor": duplicate,
            "cross_domain_anchor": cross_domain
        },
        "physical_files": files
    });
    let readback_path = root.join("readback.json");
    write_json(&readback_path, &readback);
    write_blake3sums(&root);
    println!("ISSUE078_ANCHOR_FLOOR_READBACK={}", readback_path.display());
}

fn happy_exact_floor_passes(root: &Path) -> Value {
    let rows = (0..MIN_RESOLVED_ANCHOR_FLOOR)
        .map(|idx| target_row(idx))
        .collect::<Vec<_>>();
    let paths = write_rows(root, "happy/rows", &rows);
    let run = run_tracker(root, "happy/report", paths);
    assert_eq!(run.qualified_unique_count, MIN_RESOLVED_ANCHOR_FLOOR);
    assert_eq!(run.duplicate_excluded_count, 0);
    assert_eq!(run.cross_domain_excluded_count, 0);
    assert!(run.passed);
    let decision = admission_from_report(&run);
    assert!(decision.admitted, "reason: {}", decision.reason);
    assert_eq!(decision.code, "CALYX_POLY_ADMISSION_ADMITTED");
    json!({
        "qualified_unique_count": run.qualified_unique_count,
        "source_row_count": run.source_row_count,
        "passed": run.passed,
        "report_path": run_path(root, "happy/report").display().to_string(),
        "admission": decision_json(&decision)
    })
}

fn edge_zero_anchors_refuses(root: &Path) -> Value {
    let run = run_tracker(root, "edge-zero/report", Vec::new());
    assert_eq!(run.qualified_unique_count, 0);
    assert!(!run.passed);
    let decision = admission_from_report(&run);
    assert!(!decision.admitted);
    assert_eq!(
        decision.code,
        "CALYX_POLY_ADMISSION_INSUFFICIENT_GROUNDING_ANCHORS"
    );
    json!({
        "qualified_unique_count": run.qualified_unique_count,
        "source_row_count": run.source_row_count,
        "passed": run.passed,
        "admission": decision_json(&decision)
    })
}

fn edge_duplicate_anchor_excluded(root: &Path) -> Value {
    let mut rows = (0..(MIN_RESOLVED_ANCHOR_FLOOR - 1))
        .map(|idx| target_row(idx))
        .collect::<Vec<_>>();
    rows.push(target_row(0));
    let paths = write_rows(root, "edge-duplicate/rows", &rows);
    let run = run_tracker(root, "edge-duplicate/report", paths);
    assert_eq!(run.source_row_count, MIN_RESOLVED_ANCHOR_FLOOR);
    assert_eq!(run.qualified_unique_count, MIN_RESOLVED_ANCHOR_FLOOR - 1);
    assert_eq!(run.duplicate_excluded_count, 1);
    assert!(!run.passed);
    let decision = admission_from_report(&run);
    assert!(!decision.admitted);
    assert_eq!(
        decision.code,
        "CALYX_POLY_ADMISSION_INSUFFICIENT_GROUNDING_ANCHORS"
    );
    json!({
        "qualified_unique_count": run.qualified_unique_count,
        "duplicate_excluded_count": run.duplicate_excluded_count,
        "passed": run.passed,
        "admission": decision_json(&decision)
    })
}

fn edge_cross_domain_anchor_excluded(root: &Path) -> Value {
    let mut rows = (0..(MIN_RESOLVED_ANCHOR_FLOOR - 1))
        .map(|idx| target_row(idx))
        .collect::<Vec<_>>();
    rows.push(cross_domain_row(999));
    let paths = write_rows(root, "edge-cross-domain/rows", &rows);
    let run = run_tracker(root, "edge-cross-domain/report", paths);
    assert_eq!(run.source_row_count, MIN_RESOLVED_ANCHOR_FLOOR);
    assert_eq!(run.qualified_unique_count, MIN_RESOLVED_ANCHOR_FLOOR - 1);
    assert_eq!(run.cross_domain_excluded_count, 1);
    assert!(!run.passed);
    let decision = admission_from_report(&run);
    assert!(!decision.admitted);
    assert_eq!(
        decision.code,
        "CALYX_POLY_ADMISSION_INSUFFICIENT_GROUNDING_ANCHORS"
    );
    json!({
        "qualified_unique_count": run.qualified_unique_count,
        "cross_domain_excluded_count": run.cross_domain_excluded_count,
        "passed": run.passed,
        "admission": decision_json(&decision)
    })
}

fn run_tracker(root: &Path, dir: &str, paths: Vec<PathBuf>) -> AnchorFloorReport {
    let request = AnchorFloorRequest {
        target_domain: DOMAIN.to_string(),
        target_outcome_axis: AXIS.to_string(),
        min_resolved_anchors: MIN_RESOLVED_ANCHOR_FLOOR,
        row_paths: paths,
    };
    let run = run_anchor_floor_tracker(&request, &root.join(dir)).expect("anchor floor run");
    let readback = read_anchor_floor_report(&run.report_path).expect("read report");
    assert_eq!(readback, run.report);
    readback
}

fn write_rows(root: &Path, dir: &str, rows: &[AnchorFloorRow]) -> Vec<PathBuf> {
    let row_dir = root.join(dir);
    rows.iter()
        .enumerate()
        .map(|(idx, row)| {
            calyx_poly::diagnostics_store::write_json(
                &row_dir,
                &format!("anchor-row-{idx:03}.json"),
                row,
            )
            .expect("write anchor row")
        })
        .collect()
}

fn run_path(root: &Path, dir: &str) -> PathBuf {
    root.join(dir).join("anchor-floor-report.json")
}

fn target_row(idx: usize) -> AnchorFloorRow {
    row(DOMAIN, idx)
}

fn cross_domain_row(idx: usize) -> AnchorFloorRow {
    row(CROSS_DOMAIN, idx)
}

fn row(domain: &str, idx: usize) -> AnchorFloorRow {
    AnchorFloorRow {
        anchor_id: format!("{domain}-{AXIS}-{idx:03}"),
        domain: domain.to_string(),
        outcome_axis: AXIS.to_string(),
        anchor: resolved_anchor(idx),
    }
}

fn resolved_anchor(idx: usize) -> Anchor {
    Anchor {
        kind: AnchorKind::TestPass,
        value: AnchorValue::Bool(idx % 2 == 0),
        source: format!("uma:{DOMAIN}:{AXIS}:{idx:03}"),
        observed_at: 1_785_800_000 + idx as u64,
        confidence: 1.0,
    }
}

fn admission_from_report(report: &AnchorFloorReport) -> AdmissionDecision {
    let mut inputs = good_inputs();
    inputs.grounding_anchor_count = report.qualified_unique_count as u32;
    evaluate_admission(&AdmissionParams::default(), &inputs)
}

fn good_inputs() -> AdmissionInputs {
    AdmissionInputs {
        p_win: 0.94,
        confidence: 0.74,
        sufficiency_ok: true,
        evidence_count: 2,
        source_derived_evidence_count: 2,
        stale_evidence_count: 0,
        circular_evidence_count: 0,
        super_intel_pass: true,
        guard_calibrated: true,
        grounding_anchor_count: MIN_RESOLVED_ANCHOR_FLOOR as u32,
        guard_pass: true,
        liquidity_ok: true,
        market_integrity: known_healthy_market_integrity(),
        oracle_risk: known_healthy_oracle_risk(),
        wash_trade: known_healthy_wash_trade(),
        kill_switch_active: false,
        daily_error_score: 0.0,
    }
}

fn decision_json(decision: &AdmissionDecision) -> Value {
    serde_json::to_value(decision).expect("decision JSON")
}
