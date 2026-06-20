use std::fs;
use std::path::Path;

use calyx_assay::{PanelResourceBudget, admit_lens, logistic_probe_mi};

use super::cost::LensCostMap;
use super::data::AssayCorpus;
use super::engine::evaluate_corpus;
use super::metrics::write_metric_outputs;
use super::selection::compute_signal_density;
use super::test_support::{request_for, temp_root, write_synthetic_corpus};

const DIM: usize = 16;

#[test]
fn cli_preserves_assay_runtime_code_and_detail() {
    let error = super::assay_cli_error(
        "CALYX_ASSAY_UNRESOLVED: anchor group g000 mixes positive and negative labels".to_string(),
    );

    assert_eq!(error.code(), "CALYX_ASSAY_UNRESOLVED");
    assert!(
        error.message().contains("anchor group g000"),
        "{}",
        error.message()
    );
    assert!(!error.remediation().contains("help"));
}

#[test]
fn cli_keeps_unknown_args_as_usage_errors() {
    let error = super::assay_cli_error("unknown assay bits-validate arg: --bogus".to_string());

    assert_eq!(error.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(error.remediation().contains("help"));
}

#[test]
fn synthetic_three_lens_admits_real_rejects_redundant() {
    let root = temp_root("assay-bits-pass");
    let corpus = root.join("corpus");
    fs::create_dir_all(&corpus).unwrap();
    write_synthetic_corpus(&corpus, 200);
    let request = request_for(&root);
    let data = AssayCorpus::load(&request).unwrap();
    let report = evaluate_corpus(&data, &request, None, None).unwrap();
    let evidence = write_metric_outputs(&request, &report).unwrap();

    let real_a = report
        .lenses
        .iter()
        .find(|lens| lens.name == "real_a")
        .unwrap();
    assert!(
        real_a.bits_about > 0.05,
        "real_a bits {}",
        real_a.bits_about
    );
    assert!(real_a.admitted);

    let redundant = report
        .lenses
        .iter()
        .find(|lens| lens.name == "redundant")
        .unwrap();
    assert!(
        redundant.max_pairwise_corr > 0.6,
        "redundant corr {}",
        redundant.max_pairwise_corr
    );
    assert!(!redundant.admitted);
    assert_eq!(
        redundant.rejection_reason.as_deref(),
        Some("CALYX_ASSAY_REDUNDANT")
    );

    assert!(report.panel.i_panel_anchor.is_finite());
    assert!(report.panel.ci_95[0].is_finite());
    assert!(report.panel.ci_95[1].is_finite());
    assert!(report.panel.ci_95[1] >= report.panel.ci_95[0]);
    assert_eq!(report.panel.estimate_bound, "lower_bound");
    assert_eq!(
        report.panel.power_calibration_status.as_deref(),
        Some("passed")
    );
    let panel_recovery = report
        .panel
        .power_recovery_ratio
        .expect("panel power recovery");
    assert!(panel_recovery >= 0.5, "panel recovery {panel_recovery}");
    for lens in &report.lenses {
        assert_eq!(lens.estimate_bound, "lower_bound");
        assert_eq!(
            lens.power_calibration_status.as_deref(),
            Some("passed"),
            "{}",
            lens.name
        );
        let recovery = lens.power_recovery_ratio.expect("lens power recovery");
        assert!(recovery >= 0.5, "{} recovery {recovery}", lens.name);
    }
    assert!(!report.strata.is_empty());
    assert_eq!(report.assay_cf_rows_persisted, 3);
    assert_eq!(report.assay_cf_rows_readback, 3);
    assert!(Path::new(&evidence.abundance_path).exists());
    assert!(Path::new(&evidence.bits_per_lens_path).exists());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn leaked_anchor_bits_report_is_flagged_but_still_measurable_control() {
    let root = temp_root("assay-bits-leaked-anchor");
    let corpus = root.join("corpus");
    fs::create_dir_all(&corpus).unwrap();
    write_synthetic_corpus(&corpus, 200);
    let manifest_path = corpus.join("manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["anchor_audit"] = serde_json::json!({
        "anchor_leaks_into_input": true,
        "trivial_anchor": true,
        "grounded_gate_eligible": false,
        "label_recoverable_from_input": true,
        "reason": "fixture label is recoverable from text"
    });
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    let request = request_for(&root);
    let data = AssayCorpus::load(&request).unwrap();

    let report = evaluate_corpus(&data, &request, None, None).unwrap();
    let evidence = write_metric_outputs(&request, &report).unwrap();

    assert!(report.anchor_leaks_into_input);
    assert!(report.trivial_anchor);
    assert!(!report.grounded_gate_eligible);
    assert!(
        report
            .lenses
            .iter()
            .all(|lens| lens.anchor_leaks_into_input)
    );
    let readback: serde_json::Value =
        serde_json::from_slice(&fs::read(&evidence.abundance_path).unwrap()).unwrap();
    assert_eq!(readback["anchor_audit"]["grounded_gate_eligible"], false);
    assert_eq!(readback["lenses"][0]["anchor_leaks_into_input"], true);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn low_signal_lens_rejected_by_admit_lens() {
    for step in 0..50_u32 {
        let bits = (step as f32) / 1000.0; // 0.000 .. 0.049, all < 0.05
        assert!(bits < 0.05);
        let error = admit_lens(bits, 0.0).unwrap_err();
        assert_eq!(error.code, "CALYX_ASSAY_LOW_SIGNAL", "bits={bits}");
    }

    let samples: Vec<Vec<f32>> = (0..120).map(|_| vec![1.0_f32; DIM]).collect();
    let labels: Vec<bool> = (0..120).map(|i| i % 2 == 0).collect();
    let bits = logistic_probe_mi(&samples, &labels).unwrap().estimate.bits;
    assert!(bits < 0.05, "constant-lens bits {bits}");
    assert_eq!(
        admit_lens(bits, 0.0).unwrap_err().code,
        "CALYX_ASSAY_LOW_SIGNAL"
    );
}

#[test]
fn empty_corpus_dir_reports_not_found() {
    let root = temp_root("assay-bits-missing");
    let request = request_for(&root);
    let error = AssayCorpus::load(&request).unwrap_err();
    assert!(
        error.starts_with("CALYX_FSV_ASSAY_CORPUS_NOT_FOUND"),
        "{error}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn single_class_anchor_fails_closed_without_panic() {
    let root = temp_root("assay-bits-single-class");
    let corpus = root.join("corpus");
    fs::create_dir_all(&corpus).unwrap();
    write_synthetic_corpus(&corpus, 200);
    let mut request = request_for(&root);
    request.target_class = 9;
    let data = AssayCorpus::load(&request).unwrap();
    let error = evaluate_corpus(&data, &request, None, None).unwrap_err();
    assert!(
        error.starts_with("CALYX_FSV_ASSAY_SINGLE_CLASS_ANCHOR"),
        "single-class anchor must fail closed, got {error}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn too_few_samples_surface_insufficient_samples() {
    let root = temp_root("assay-bits-small");
    let corpus = root.join("corpus");
    fs::create_dir_all(&corpus).unwrap();
    write_synthetic_corpus(&corpus, 40);
    let request = request_for(&root);
    let error = AssayCorpus::load(&request).unwrap_err();
    assert!(
        error.starts_with("CALYX_FSV_ASSAY_INVALID_CORPUS"),
        "{error}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn signal_density_uses_budget_density_without_cpu_pre_rank() {
    let root = temp_root("assay-bits-density");
    let corpus = root.join("corpus");
    fs::create_dir_all(&corpus).unwrap();
    write_synthetic_corpus(&corpus, 200);

    let cost_path = root.join("cost.json");
    fs::write(
        &cost_path,
        r#"{
          "real_a":    {"placement":"cpu", "vram_mb": 0.0,   "ms_per_input": 0.10, "ram_mb": 64.0},
          "real_b":    {"placement":"cpu", "vram_mb": 0.0,   "ms_per_input": 2.0,  "ram_mb": 64.0},
          "redundant": {"placement":"gpu", "vram_mb": 500.0, "ms_per_input": 4.0,  "ram_mb": 0.0}
        }"#,
    )
    .unwrap();

    let mut request = request_for(&root);
    request.cost_json = Some(cost_path.clone());
    let data = AssayCorpus::load(&request).unwrap();
    let cost = LensCostMap::load(&cost_path).unwrap();
    let report = evaluate_corpus(&data, &request, Some(&cost), None).unwrap();
    let evidence = write_metric_outputs(&request, &report).unwrap();

    // Density is attached to every lens and arithmetically correct.
    let real_a = report.lenses.iter().find(|l| l.name == "real_a").unwrap();
    let d_a = real_a.density.expect("real_a density");
    assert!(d_a.zero_vram, "real_a is CPU-only");
    assert!(
        d_a.bits_per_vram_mb.is_none(),
        "zero-VRAM => no GPU density"
    );
    let expect_a_ms = real_a.bits_about.max(0.0) / 0.10;
    assert!(
        (d_a.bits_per_ms.unwrap() - expect_a_ms).abs() < 1e-4,
        "real_a bits/ms {} != {expect_a_ms}",
        d_a.bits_per_ms.unwrap()
    );

    let real_b = report.lenses.iter().find(|l| l.name == "real_b").unwrap();
    let d_b = real_b.density.expect("real_b density");
    assert!(d_b.zero_vram);
    assert!(
        d_a.bits_per_ms.unwrap() > d_b.bits_per_ms.unwrap(),
        "fixture must put real_a ahead of real_b on CPU bits/ms"
    );

    let redundant = report
        .lenses
        .iter()
        .find(|l| l.name == "redundant")
        .unwrap();
    let d_redundant = redundant.density.expect("redundant density");
    assert!(!d_redundant.zero_vram);
    let expect_b_vram = redundant.bits_about.max(0.0) / 500.0;
    assert!(
        (d_redundant.bits_per_vram_mb.unwrap() - expect_b_vram).abs() < 1e-6,
        "redundant bits/VRAM-MB {:?} != {expect_b_vram}",
        d_redundant.bits_per_vram_mb
    );

    let density = report.signal_density.as_ref().expect("density report");
    assert!(
        density.note.contains("bits per dominant budget fraction"),
        "{}",
        density.note
    );
    assert!(
        density.note.contains("no CPU-only pre-rank"),
        "{}",
        density.note
    );
    assert!(density.ranked.iter().any(|r| !r.zero_vram));
    for pair in density.ranked.windows(2) {
        let left = pair[0]
            .bits_per_budget_fraction
            .expect("left budget density");
        let right = pair[1]
            .bits_per_budget_fraction
            .expect("right budget density");
        assert!(
            left + 1e-6 >= right,
            "{} density {left} sorted before {} density {right}",
            pair[0].name,
            pair[1].name
        );
    }

    let mut fixed_budget_lenses = report.lenses.clone();
    let fixed_budget_density = compute_signal_density(
        &mut fixed_budget_lenses,
        &cost,
        PanelResourceBudget {
            max_vram_mb: 1_000_000.0,
            max_ram_mb: 128.0,
            max_ms_per_input: 1_000_000.0,
        },
    )
    .unwrap();
    assert_eq!(
        fixed_budget_density.ranked.first().map(|r| r.zero_vram),
        Some(false),
        "GPU lens must be able to outrank CPU lenses by budget density"
    );

    let path = evidence
        .signal_density_path
        .as_ref()
        .expect("signal_density_path present");
    let readback: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(readback["ranked"][0]["name"], density.ranked[0].name);
    assert_eq!(
        readback["ranked"][0]["zero_vram"],
        density.ranked[0].zero_vram
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn panel_budget_packs_density_panel_and_writes_readback_artifact() {
    let root = temp_root("assay-bits-packed-panel");
    let corpus = root.join("corpus");
    fs::create_dir_all(&corpus).unwrap();
    write_synthetic_corpus(&corpus, 200);

    let cost_path = root.join("cost.json");
    fs::write(
        &cost_path,
        r#"{
          "real_a":    {"placement":"cpu", "vram_mb": 0.0,   "ms_per_input": 0.10, "ram_mb": 64.0},
          "real_b":    {"placement":"gpu", "vram_mb": 500.0, "ms_per_input": 4.0,  "ram_mb": 0.0},
          "redundant": {"placement":"gpu", "vram_mb": 500.0, "ms_per_input": 4.0,  "ram_mb": 0.0}
        }"#,
    )
    .unwrap();

    let mut request = request_for(&root);
    request.cost_json = Some(cost_path.clone());
    let data = AssayCorpus::load(&request).unwrap();
    let cost = LensCostMap::load(&cost_path).unwrap();
    let budget = PanelResourceBudget {
        max_vram_mb: 400.0,
        max_ram_mb: 128.0,
        max_ms_per_input: 5.0,
    };
    let report = evaluate_corpus(&data, &request, Some(&cost), Some(budget)).unwrap();
    let evidence = write_metric_outputs(&request, &report).unwrap();
    let packed = report.packed_panel.as_ref().expect("packed panel");

    assert_eq!(packed.used.vram_mb, 0.0);
    assert!(
        packed
            .selected
            .iter()
            .any(|decision| decision.lens == "real_a")
    );
    assert!(packed.rejected.iter().any(|decision| {
        decision.lens == "real_b"
            && decision
                .rejection_reason
                .as_deref()
                .is_some_and(|reason| reason == "CALYX_ASSAY_RESOURCE_BUDGET_EXCEEDED")
    }));
    let path = evidence
        .packed_panel_path
        .as_ref()
        .expect("packed panel path present");
    let readback: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(readback["selected"][0]["lens"], "real_a");
    assert_eq!(readback["remaining"]["vram_mb"], 400.0);
    let comparison_path = evidence
        .panel_comparison_path
        .as_ref()
        .expect("panel comparison path present");
    let comparison: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(comparison_path).unwrap()).unwrap();
    assert_eq!(
        comparison["density_panel"]["lenses"][0],
        serde_json::json!("real_a")
    );
    assert_eq!(comparison["control_lens_limit"], 2);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn missing_cost_entry_fails_closed() {
    let root = temp_root("assay-bits-density-missing");
    let corpus = root.join("corpus");
    fs::create_dir_all(&corpus).unwrap();
    write_synthetic_corpus(&corpus, 200);

    let cost_path = root.join("cost.json");
    fs::write(
        &cost_path,
        r#"{
          "real_a": {"placement":"cpu", "vram_mb": 0.0,   "ms_per_input": 0.10},
          "real_b": {"placement":"gpu", "vram_mb": 500.0, "ms_per_input": 4.0}
        }"#,
    )
    .unwrap();

    let mut request = request_for(&root);
    request.cost_json = Some(cost_path.clone());
    let data = AssayCorpus::load(&request).unwrap();
    let cost = LensCostMap::load(&cost_path).unwrap();
    let error = evaluate_corpus(&data, &request, Some(&cost), None).unwrap_err();
    assert!(
        error.starts_with("CALYX_FSV_ASSAY_MISSING_COST"),
        "missing cost must fail closed, got {error}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn invalid_cost_rejected() {
    let root = temp_root("assay-bits-cost-invalid");
    fs::create_dir_all(&root).unwrap();
    let bad = root.join("bad.json");
    fs::write(
        &bad,
        r#"{"real_a": {"placement":"cpu", "vram_mb": 0.0, "ms_per_input": 0.0}}"#,
    )
    .unwrap();
    let error = LensCostMap::load(&bad).unwrap_err();
    assert!(error.starts_with("CALYX_FSV_ASSAY_INVALID_COST"), "{error}");
    let bad2 = root.join("bad2.json");
    fs::write(
        &bad2,
        r#"{"real_a": {"placement":"cpu", "vram_mb": -1.0, "ms_per_input": 1.0}}"#,
    )
    .unwrap();
    assert!(
        LensCostMap::load(&bad2)
            .unwrap_err()
            .starts_with("CALYX_FSV_ASSAY_INVALID_COST")
    );
    let _ = fs::remove_dir_all(root);
}
