use std::fs;
use std::path::{Path, PathBuf};

use calyx_assay::{PanelResourceBudget, admit_lens, logistic_probe_mi};

use super::cost::LensCostMap;
use super::data::AssayCorpus;
use super::engine::evaluate_corpus;
use super::metrics::write_metric_outputs;
use super::request::AssayBitsRequest;

const DIM: usize = 16;

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
    assert!(!report.strata.is_empty());
    assert_eq!(report.assay_cf_rows_persisted, 3);
    assert_eq!(report.assay_cf_rows_readback, 3);
    assert!(Path::new(&evidence.abundance_path).exists());
    assert!(Path::new(&evidence.bits_per_lens_path).exists());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn low_signal_lens_rejected_by_admit_lens() {
    // Deterministic loop over every sub-threshold bits value: the admission
    // contract must reject each with CALYX_ASSAY_LOW_SIGNAL.
    for step in 0..50_u32 {
        let bits = (step as f32) / 1000.0; // 0.000 .. 0.049, all < 0.05
        assert!(bits < 0.05);
        let error = admit_lens(bits, 0.0).unwrap_err();
        assert_eq!(error.code, "CALYX_ASSAY_LOW_SIGNAL", "bits={bits}");
    }

    // A genuinely uninformative lens (constant vectors) measures ~0 bits and is
    // therefore rejected by the admission contract end-to-end.
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
    // target_class 9 never appears in labels -> anchor all-false -> single-class.
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
fn signal_density_computed_and_ranked_cpu_first() {
    let root = temp_root("assay-bits-density");
    let corpus = root.join("corpus");
    fs::create_dir_all(&corpus).unwrap();
    write_synthetic_corpus(&corpus, 200);

    // Cost sidecar: real_a + real_b are CPU-only (zero VRAM) with different
    // latency densities; redundant stays on GPU so both ranking branches run.
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

    // Ranking: CPU-only lenses sort first by descending bits/ms; GPU follows.
    let density = report.signal_density.as_ref().expect("density report");
    assert_eq!(
        density.ranked.first().map(|r| r.name.as_str()),
        Some("real_a"),
        "CPU-only lens must rank first by signal density"
    );
    assert!(density.ranked.iter().any(|r| !r.zero_vram));

    // The density artifact is written and reads back identically.
    let path = evidence
        .signal_density_path
        .as_ref()
        .expect("signal_density_path present");
    let readback: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(readback["ranked"][0]["name"], "real_a");
    assert_eq!(readback["ranked"][0]["zero_vram"], true);

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

    let _ = fs::remove_dir_all(root);
}

#[test]
fn missing_cost_entry_fails_closed() {
    let root = temp_root("assay-bits-density-missing");
    let corpus = root.join("corpus");
    fs::create_dir_all(&corpus).unwrap();
    write_synthetic_corpus(&corpus, 200);

    // Omit "redundant" from the cost sidecar -> hard error, no silent default.
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
    // ms_per_input == 0 is a divisor -> must be rejected.
    let bad = root.join("bad.json");
    fs::write(
        &bad,
        r#"{"real_a": {"placement":"cpu", "vram_mb": 0.0, "ms_per_input": 0.0}}"#,
    )
    .unwrap();
    let error = LensCostMap::load(&bad).unwrap_err();
    assert!(error.starts_with("CALYX_FSV_ASSAY_INVALID_COST"), "{error}");
    // Negative VRAM rejected.
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

fn request_for(root: &Path) -> AssayBitsRequest {
    let metrics = root.join("metrics");
    AssayBitsRequest {
        corpus_dir: root.join("corpus"),
        metrics_dir: metrics.clone(),
        cf_root: metrics.join("assay_cf"),
        min_bits: 0.05,
        max_corr: 0.6,
        target_class: 0,
        domain: "ag_news_test".to_string(),
        cost_json: None,
        panel_budget_json: None,
    }
}

/// Writes a deterministic 3-lens fixture (seed=42):
/// - `real_a`: informative, vectors separable by binary label.
/// - `real_b`: informative + independent (different separation axis).
/// - `redundant`: `real_a` + tiny deterministic noise (corr > 0.6), redundant.
fn write_synthetic_corpus(dir: &Path, rows: usize) {
    let seed = 42_u64;
    let mut lines = String::new();
    for i in 0..rows {
        let label = i % 2; // binary anchor class 0 vs 1
        let is_zero = label == 0;
        let real_a = lens_real_a(seed, i as u64, is_zero);
        let redundant: Vec<f32> = real_a
            .iter()
            .enumerate()
            .map(|(d, v)| v + 0.001 * jitter(seed ^ 0xAB, i as u64, d as u64))
            .collect();
        let real_b = lens_real_b(seed, i as u64, is_zero);
        lines.push_str(&format!(
            "{{\"id\":\"s{i}\",\"split\":\"train\",\"label\":{label},\"lenses\":{{\"real_a\":{},\"real_b\":{},\"redundant\":{}}}}}\n",
            vec_json(&real_a),
            vec_json(&real_b),
            vec_json(&redundant)
        ));
    }
    fs::write(dir.join("vectors.jsonl"), lines).unwrap();

    let manifest = format!(
        "{{\"dataset\":\"synthetic\",\"embedding_model_id\":\"test-embed\",\"n_samples\":{rows},\"label_counts\":{{\"0\":{half},\"1\":{half}}},\"lenses\":[{{\"name\":\"real_a\",\"redundant\":false}},{{\"name\":\"real_b\",\"redundant\":false}},{{\"name\":\"redundant\",\"redundant\":true}}],\"target_class\":0}}\n",
        half = rows / 2
    );
    fs::write(dir.join("manifest.json"), manifest).unwrap();
}

fn lens_real_a(seed: u64, i: u64, is_zero: bool) -> Vec<f32> {
    let offset = if is_zero { 1.0 } else { -1.0 };
    (0..DIM)
        .map(|d| {
            let base = if d < DIM / 2 { offset } else { 0.0 };
            base + 0.15 * jitter(seed, i, d as u64)
        })
        .collect()
}

fn lens_real_b(seed: u64, i: u64, is_zero: bool) -> Vec<f32> {
    let offset = if is_zero { -1.0 } else { 1.0 };
    (0..DIM)
        .map(|d| {
            // Separation lives in the second half -> independent axis from real_a.
            let base = if d >= DIM / 2 { offset } else { 0.0 };
            base + 0.15 * jitter(seed ^ 0xCD, i, d as u64)
        })
        .collect()
}

/// Deterministic pseudo-random jitter in [-1, 1] from a hashed seed/index/dim.
fn jitter(seed: u64, i: u64, d: u64) -> f32 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&seed.to_be_bytes());
    hasher.update(&i.to_be_bytes());
    hasher.update(&d.to_be_bytes());
    let bytes = hasher.finalize();
    let raw = u32::from_be_bytes([
        bytes.as_bytes()[0],
        bytes.as_bytes()[1],
        bytes.as_bytes()[2],
        bytes.as_bytes()[3],
    ]);
    (raw as f32 / u32::MAX as f32) * 2.0 - 1.0
}

fn vec_json(values: &[f32]) -> String {
    let parts: Vec<String> = values.iter().map(|v| format!("{v:.6}")).collect();
    format!("[{}]", parts.join(","))
}

fn temp_root(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("{name}-{}", std::process::id()))
}
