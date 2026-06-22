use std::fs;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_anneal::{
    CandidateAction, GoodhartState, GradientCandidate, IntelligenceGradient, JMetricSources,
    JObjectiveContext, JWeights, ReportAvailability, ScopeId, compute_j,
    decode_intelligence_report_row, format_report, intelligence_report,
    read_intelligence_report_snapshot, report_diff, to_json, write_intelligence_report_snapshot,
};
use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::FixedClock;

mod fsv_support;
use fsv_support::vault_id;

#[test]
fn report_gradient_is_descending_by_priority() {
    let report = report_with_gradient(
        metrics(1.5),
        vec![
            candidate(label("mid", 0.9), 1),
            candidate(label("top", 2.0), 1),
            candidate(recompute("slow", 0.6), 2),
        ],
    );

    assert_eq!(report.gradient.len(), 3);
    assert!(matches!(
        &report.gradient[0].action,
        CandidateAction::LabelAnchor { anchor, .. } if anchor.as_str() == "top"
    ));
    assert!(matches!(
        &report.next_best_action,
        Some(CandidateAction::LabelAnchor { anchor, .. }) if anchor.as_str() == "top"
    ));
}

#[test]
fn format_report_includes_required_strings_and_terms() {
    let report = report_with_gradient(metrics(1.5), vec![candidate(label("top", 2.0), 1)]);
    let text = format_report(&report);

    assert!(text.contains("J = 7.200000"));
    assert!(text.contains("DPI headroom: 0.500000"));
    for label in [
        "w1_info",
        "w2_n_eff",
        "w3_sufficiency",
        "w4_kernel_recall",
        "w5_oracle_accuracy",
        "w6_mistake_rate",
        "w7_compression",
        "w8_coverage",
    ] {
        assert!(text.contains(label), "missing term label {label}");
    }
    assert!(text.contains("next_best_action: LabelAnchor"));
}

#[test]
fn report_json_roundtrips_available_value() {
    let report = report_with_gradient(metrics(1.5), vec![candidate(label("top", 2.0), 1)]);
    let json = to_json(&report);
    let roundtrip: calyx_anneal::IntelligenceReport =
        serde_json::from_value(json).expect("report json");

    assert_eq!(roundtrip.j, report.j);
    assert_eq!(roundtrip.terms.w1_info, 1.5);
    assert_eq!(roundtrip.availability, ReportAvailability::Available);
}

#[test]
fn edge_empty_gradient_no_goodhart_and_zero_provisional_are_printed() {
    let report = report_with_gradient(metrics(1.5), Vec::new());
    let text = format_report(&report);

    assert_eq!(report.provisional_excluded, 0);
    assert!(report.gradient.is_empty());
    assert!(report.next_best_action.is_none());
    assert!(report.goodhart_last.is_none());
    assert!(text.contains("provisional_excluded: 0"));
    assert!(text.contains("Gradient top 5:\n  <empty>"));
    assert!(text.contains("next_best_action: None"));
    assert!(text.contains("Goodhart: no check yet"));
}

#[test]
fn invalid_source_is_unavailable_not_silent_zero() {
    let report = report_with_gradient(invalid_metrics(), Vec::new());
    let json = to_json(&report);

    assert!(report.j.is_nan());
    assert_ne!(report.j, 0.0);
    assert!(matches!(
        report.availability,
        ReportAvailability::Unavailable { ref code, .. }
            if code == "CALYX_ANNEAL_J_INVALID_METRIC"
    ));
    assert!(json["j"].is_null());
    assert_eq!(
        json["availability"]["state"],
        serde_json::Value::String("unavailable".to_string())
    );
}

#[test]
fn report_diff_tracks_positive_delta_and_new_gradient_top() {
    let before = report_with_gradient(metrics(1.0), vec![candidate(label("before", 0.4), 1)]);
    let after = report_with_gradient(metrics(1.5), vec![candidate(label("after", 2.0), 1)]);
    let diff = report_diff(&before, &after);

    assert!((diff.delta_j - 0.5).abs() < 1e-12);
    assert!((diff.per_term_deltas.w1_info - 0.5).abs() < 1e-12);
    assert!(matches!(
        diff.new_gradient_top,
        Some(ref entry)
            if matches!(&entry.action, CandidateAction::LabelAnchor { anchor, .. } if anchor.as_str() == "after")
    ));
}

#[test]
fn report_persists_to_anneal_report_cf_and_decodes() {
    let dir = temp_dir("persist");
    let vault = AsterVault::new_durable(&dir, vault_id(), b"report-test", VaultOptions::default())
        .expect("vault");
    let report = report_with_gradient(metrics(1.5), vec![candidate(label("top", 2.0), 1)]);

    let key = write_intelligence_report_snapshot(&vault, &report).expect("write report");
    let stored = read_intelligence_report_snapshot(&vault, report.ts)
        .expect("read report")
        .expect("report row");
    let rows = vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::AnnealReport)
        .expect("scan report cf");
    let sst_dir = dir.join("cf").join("anneal_report");

    assert_eq!(key, report.ts.to_be_bytes().to_vec());
    assert_eq!(stored.j, report.j);
    assert_eq!(rows.len(), 1);
    assert_eq!(
        decode_intelligence_report_row(&rows[0].1).unwrap().j,
        report.j
    );
    assert!(fs::read_dir(sst_dir).unwrap().any(|entry| {
        entry
            .unwrap()
            .path()
            .extension()
            .and_then(|value| value.to_str())
            == Some("sst")
    }));
    cleanup(dir);
}

#[test]
fn unavailable_report_persists_and_decodes_without_zero_fill() {
    let dir = temp_dir("unavailable");
    let vault = AsterVault::new_durable(
        &dir,
        vault_id(),
        b"unavailable-report-test",
        VaultOptions::default(),
    )
    .expect("vault");
    let report = report_with_gradient(invalid_metrics(), Vec::new());

    write_intelligence_report_snapshot(&vault, &report).expect("write unavailable report");
    let stored = read_intelligence_report_snapshot(&vault, report.ts)
        .expect("read unavailable report")
        .expect("report row");

    assert!(stored.j.is_nan());
    assert_ne!(stored.j, 0.0);
    assert!(matches!(
        stored.availability,
        ReportAvailability::Unavailable { ref code, .. }
            if code == "CALYX_ANNEAL_J_INVALID_METRIC"
    ));
    cleanup(dir);
}

fn report_with_gradient(
    metrics: Metrics,
    candidates: Vec<GradientCandidate>,
) -> calyx_anneal::IntelligenceReport {
    let context = context();
    let j_value = compute_j(&context, &metrics).unwrap_or_else(|_| calyx_anneal::JValue {
        j: f64::NAN,
        terms: nan_terms(),
        dpi_ceiling: f64::NAN,
        dpi_headroom: f64::NAN,
        provisional_excluded: 0,
        weights: JWeights::default(),
    });
    let clock = Arc::new(FixedClock::new(1_785_600_001));
    let mut gradient = IntelligenceGradient::new(j_value, clock.clone());
    gradient.refresh(candidates);
    intelligence_report(
        &context,
        &metrics,
        &gradient,
        &GoodhartState::default(),
        None,
        clock.as_ref(),
    )
}

fn context() -> JObjectiveContext {
    JObjectiveContext::new("fixture", 4)
}

#[derive(Clone, Copy)]
struct Metrics {
    mutual_info_panel_anchor: f64,
}

impl JMetricSources for Metrics {
    fn mutual_info_panel_anchor(&self) -> f64 {
        self.mutual_info_panel_anchor
    }

    fn n_eff(&self) -> f64 {
        3.5
    }

    fn panel_sufficiency(&self, _domain: &str) -> f64 {
        0.8
    }

    fn kernel_recall(&self) -> f64 {
        0.7
    }

    fn oracle_accuracy(&self) -> f64 {
        0.6
    }

    fn mistake_rate(&self) -> f64 {
        0.1
    }

    fn compression_yield(&self) -> f64 {
        0.4
    }

    fn coverage(&self) -> f64 {
        0.3
    }

    fn dpi_ceiling(&self) -> f64 {
        2.0
    }

    fn provisional_count(&self) -> usize {
        0
    }
}

fn metrics(mutual_info_panel_anchor: f64) -> Metrics {
    Metrics {
        mutual_info_panel_anchor,
    }
}

fn invalid_metrics() -> Metrics {
    metrics(f64::NAN)
}

fn candidate(action: CandidateAction, cost_budget_units: u64) -> GradientCandidate {
    GradientCandidate {
        action,
        cost_budget_units,
    }
}

fn label(anchor: &str, estimated_dj: f64) -> CandidateAction {
    CandidateAction::LabelAnchor {
        anchor: calyx_anneal::AnchorId::new(anchor).unwrap(),
        estimated_dj,
    }
}

fn recompute(scope: &str, estimated_dj: f64) -> CandidateAction {
    CandidateAction::RecomputeKernel {
        scope: ScopeId::new(scope),
        estimated_dj,
    }
}

fn nan_terms() -> calyx_anneal::JTerms {
    calyx_anneal::JTerms {
        w1_info: f64::NAN,
        w2_n_eff: f64::NAN,
        w3_sufficiency: f64::NAN,
        w4_kernel_recall: f64::NAN,
        w5_oracle_accuracy: f64::NAN,
        w6_mistake_rate: f64::NAN,
        w7_compression: f64::NAN,
        w8_coverage: f64::NAN,
        p_redundant: f64::NAN,
        p_ungrounded: f64::NAN,
        p_goodhart: f64::NAN,
    }
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "calyx-intelligence-report-{name}-{}-{nanos}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn cleanup(dir: std::path::PathBuf) {
    assert!(dir.starts_with(std::env::temp_dir()));
    let _ = fs::remove_dir_all(dir);
}
