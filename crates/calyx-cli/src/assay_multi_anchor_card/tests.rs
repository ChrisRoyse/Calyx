use std::fs;
use std::path::{Path, PathBuf};

use super::engine::evaluate;
use super::request::{Mode, Request};
use super::write::write_outputs;
use super::{CODE_INVALID_REPORT, CODE_OUTPUT_EXISTS};

#[test]
fn aggregates_lens_success_across_targets() {
    let root = temp_root("multi-anchor-pass");
    fs::create_dir_all(&root).unwrap();
    let names = lens_names(false);
    let report_a = root.join("target0.json");
    let report_b = root.join("target1.json");
    write_report(&report_a, 0, &names, |idx| if idx < 5 { 0.06 } else { 0.0 });
    write_report(
        &report_b,
        1,
        &names,
        |idx| if idx >= 5 { 0.07 } else { 0.0 },
    );
    let request = request_for(&root, &[report_a, report_b], Mode::Gate);

    let report = evaluate(&request).unwrap();
    let evidence = write_outputs(&request, &report).unwrap();

    assert!(report.gate_passed);
    assert_eq!(report.status, calyx_assay::A37_DIVERSITY_GATE_PASSED);
    assert_eq!(report.passing_lens_count, 10);
    assert!(evidence.readback_matches);
    assert!(Path::new(&evidence.report_path).is_file());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn refuses_when_any_lens_has_no_anchor() {
    let root = temp_root("multi-anchor-refuse");
    fs::create_dir_all(&root).unwrap();
    let names = lens_names(false);
    let report_a = root.join("target0.json");
    let report_b = root.join("target1.json");
    write_report(&report_a, 0, &names, |idx| if idx < 5 { 0.06 } else { 0.0 });
    write_report(&report_b, 1, &names, |idx| if idx > 5 { 0.07 } else { 0.0 });
    let request = request_for(&root, &[report_a, report_b], Mode::Diagnostic);

    let report = evaluate(&request).unwrap();

    assert!(!report.gate_passed);
    assert_eq!(report.status, calyx_assay::A37_DIVERSITY_DIAGNOSTIC_ONLY);
    assert_eq!(report.passing_lens_count, 9);
    assert_eq!(report.weakest_lens, "semantic-lens-5");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn mismatched_rosters_fail_closed() {
    let root = temp_root("multi-anchor-roster-mismatch");
    fs::create_dir_all(&root).unwrap();
    let report_a = root.join("target0.json");
    let report_b = root.join("target1.json");
    write_report(&report_a, 0, &lens_names(false), |_| 0.06);
    write_report(&report_b, 1, &lens_names(true), |_| 0.06);
    let request = request_for(&root, &[report_a, report_b], Mode::Diagnostic);

    let error = evaluate(&request).unwrap_err();

    assert!(error.starts_with(CODE_INVALID_REPORT));
    assert!(error.contains("lens roster differs"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn existing_output_dir_fails_closed() {
    let root = temp_root("multi-anchor-output-exists");
    fs::create_dir_all(root.join("out")).unwrap();
    let report_a = root.join("target0.json");
    let report_b = root.join("target1.json");
    write_report(&report_a, 0, &lens_names(false), |_| 0.06);
    write_report(&report_b, 1, &lens_names(false), |_| 0.06);
    let request = request_for(&root, &[report_a, report_b], Mode::Diagnostic);

    let error = request.ensure_fresh_output().unwrap_err();

    assert!(error.starts_with(CODE_OUTPUT_EXISTS));
    let _ = fs::remove_dir_all(root);
}

fn request_for(root: &Path, reports: &[PathBuf], mode: Mode) -> Request {
    Request {
        reports: reports.to_vec(),
        out_dir: root.join("out"),
        min_lenses: 10,
        min_marginal_bits: 0.05,
        max_redundancy: 0.6,
        mode,
    }
}

fn lens_names(mismatch: bool) -> Vec<String> {
    let mut names = vec!["splade-lens-0".to_string(), "colbert-lens-1".to_string()];
    for idx in 2..10 {
        names.push(format!("semantic-lens-{idx}"));
    }
    if mismatch {
        names[9] = "semantic-lens-99".to_string();
    }
    names
}

fn write_report(
    path: &Path,
    target_class: usize,
    names: &[String],
    marginal: impl Fn(usize) -> f32,
) {
    let lenses = names
        .iter()
        .enumerate()
        .map(|(idx, name)| {
            let bits = marginal(idx);
            serde_json::json!({
                "name": name,
                "slot": idx,
                "solo_bits": 0.2,
                "solo_ci": [0.1, 0.3],
                "panel_without_bits": 0.3,
                "marginal_bits": bits,
                "marginal_ci": [bits, bits],
                "pid": {"unique_bits": bits, "redundant_bits": 0.1, "synergistic_bits": 0.0},
                "max_pairwise_corr": 0.1,
                "max_pairwise_nmi": 0.1,
                "decision": if bits >= 0.05 { "keep" } else { "park" },
                "decision_reason": "test"
            })
        })
        .collect::<Vec<_>>();
    let report = serde_json::json!({
        "target_class": target_class,
        "domain": "unit",
        "card": {
            "schema_version": 1,
            "source": "unit",
            "pid_method": "unit",
            "panel_lens_count": names.len(),
            "n_samples": 80,
            "anchor_entropy_bits": 1.0,
            "panel_bits": 0.5,
            "panel_ci": [0.4, 0.6],
            "n_eff": 9.0,
            "sufficient": false,
            "deficit_bits": 0.5,
            "a37_diversity": {
                "schema_version": 1,
                "role": "a37_associational_diversity_gate",
                "status": "diagnostic_only",
                "content_lens_count": names.len(),
                "temporal_sidecar_count": 0,
                "temporal_counts_toward_content_floor": false,
                "temporal_lane_role": "time_manipulation_walk_forward_backward_as_of_sidecar",
                "association_family_count": 3,
                "association_families": {},
                "temporal_sidecar_slots": [],
                "family_span_pass": true,
                "redundancy_bound_pass": true,
                "no_collapse_pass": false,
                "n_eff": 9.0,
                "n_eff_floor": 6.0,
                "mean_pairwise_corr": 0.1,
                "mean_pairwise_nmi": 0.1,
                "max_redundancy": 0.6,
                "sum_unique_pid_bits": 0.0,
                "min_marginal_bits": 0.05,
                "verdict": "unit"
            },
            "sufficiency": {
                "panel_bits": 0.5,
                "sufficiency_basis_bits": 1.0,
                "anchor_entropy_bits": 1.0,
                "sufficient": false,
                "deficit_bits": 0.5,
                "deficits": [],
                "trust": "provisional",
                "estimate_bound": "lower_bound"
            },
            "lenses": lenses,
            "pairs": [],
            "keep_count": 0,
            "park_count": names.len(),
            "retire_count": 0
        }
    });
    fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
}

fn temp_root(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "calyx-{name}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}
