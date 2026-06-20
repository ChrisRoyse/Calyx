use std::fs;
use std::path::{Path, PathBuf};

use super::data::OracleCorpus;
use super::engine::evaluate_corpus;
use super::metrics::write_metric_outputs;
use super::request::OracleSufficiencyRequest;

const DIM: usize = 16;

#[test]
fn form_only_panel_is_insufficient_and_refusal_fires() {
    // Labels are RANDOM w.r.t. the lens vectors: the form-only panel cannot
    // recover the oracle, so I(panel;oracle) < H(Y), refusal fires, deficit > 0.
    let root = temp_root("oracle-suff-refused");
    let corpus_dir = root.join("corpus");
    fs::create_dir_all(&corpus_dir).unwrap();
    write_random_corpus(&corpus_dir, 200);
    let request = request_for(&root);
    let data = OracleCorpus::load(&request).unwrap();
    let report = evaluate_corpus(&data, &request).unwrap();
    let evidence = write_metric_outputs(&request, &report).unwrap();

    assert!(report.refused, "refusal must fire on a form-only panel");
    assert!(!report.sufficient, "form-only panel must be insufficient");
    assert!(
        report.i_panel_oracle < report.h_y,
        "i_panel={} must be below h_y={}",
        report.i_panel_oracle,
        report.h_y
    );
    assert!(report.deficit > 0.0, "deficit {}", report.deficit);

    // Per-lens + Panel + OutcomeEntropy rows persist and read back durably.
    assert_eq!(report.rows_persisted, data.lenses.len() + 2);
    assert_eq!(report.rows_readback, report.rows_persisted);

    assert!(Path::new(&evidence.sufficiency_json_path).exists());
    assert!(Path::new(&evidence.i_panel_path).exists());
    assert!(Path::new(&evidence.entropy_path).exists());
    assert!(Path::new(&evidence.deficit_path).exists());
    assert!(Path::new(&evidence.refused_path).exists());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn separable_panel_refuses_when_lower_bound_does_not_prove_sufficiency() {
    // Lens vectors CLEANLY separate the label: the panel recovers the oracle,
    // but lower-bound honesty only proves sufficiency when ci_low >= H(Y).
    let root = temp_root("oracle-suff-separable");
    let corpus_dir = root.join("corpus");
    fs::create_dir_all(&corpus_dir).unwrap();
    write_separable_corpus(&corpus_dir, 200);
    let request = request_for(&root);
    let data = OracleCorpus::load(&request).unwrap();
    let report = evaluate_corpus(&data, &request).unwrap();
    assert!(
        report.i_panel_oracle >= report.h_y,
        "fixture should recover the point MI: i_panel={} h_y={}",
        report.i_panel_oracle,
        report.h_y
    );
    assert!(
        report.sufficiency_basis_bits < report.h_y,
        "lower-bound basis must remain below H(Y): basis={} h_y={}",
        report.sufficiency_basis_bits,
        report.h_y
    );
    assert!(
        report.refused,
        "refusal remains honest until ci_low >= H(Y)"
    );
    assert_eq!(report.estimate_bound, "lower_bound");
    assert_eq!(report.power_calibration_status.as_deref(), Some("passed"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn missing_corpus_dir_reports_not_found() {
    let root = temp_root("oracle-suff-missing");
    let request = request_for(&root);
    let error = OracleCorpus::load(&request).unwrap_err();
    assert!(
        error.starts_with("CALYX_FSV_ORACLE_CORPUS_NOT_FOUND"),
        "{error}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn single_label_corpus_fails_closed() {
    let root = temp_root("oracle-suff-single-label");
    let corpus_dir = root.join("corpus");
    fs::create_dir_all(&corpus_dir).unwrap();
    write_single_label_corpus(&corpus_dir, 200);
    let request = request_for(&root);
    let error = OracleCorpus::load(&request).unwrap_err();
    assert!(
        error.starts_with("CALYX_FSV_ORACLE_INVALID_CORPUS"),
        "single-label corpus must fail closed, got {error}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn too_few_samples_surface_invalid_corpus() {
    let root = temp_root("oracle-suff-small");
    let corpus_dir = root.join("corpus");
    fs::create_dir_all(&corpus_dir).unwrap();
    write_random_corpus(&corpus_dir, 40);
    let request = request_for(&root);
    let error = OracleCorpus::load(&request).unwrap_err();
    assert!(
        error.starts_with("CALYX_FSV_ORACLE_INVALID_CORPUS"),
        "{error}"
    );
    let _ = fs::remove_dir_all(root);
}

fn request_for(root: &Path) -> OracleSufficiencyRequest {
    let metrics = root.join("metrics");
    OracleSufficiencyRequest {
        corpus_dir: root.join("corpus"),
        metrics_dir: metrics.clone(),
        cf_root: metrics.join("oracle_cf"),
        domain: "swebench_lite_test".to_string(),
    }
}

/// Writes a 2-lens fixture (seed=42) whose labels are RANDOM with respect to
/// the lens vectors: ~25% positive, vectors independent of the label.
fn write_random_corpus(dir: &Path, rows: usize) {
    let seed = 42_u64;
    let mut lines = String::new();
    let mut resolved = 0_usize;
    for i in 0..rows {
        // Deterministic ~25% positive rate, uncorrelated with the vectors below.
        let label = u8::from(label_bucket(seed ^ 0x5151, i as u64) < 0.25);
        resolved += usize::from(label == 1);
        // Vectors derived from the index only — no dependence on the label.
        let problem = independent_lens(seed, i as u64);
        let hints = independent_lens(seed ^ 0xCD, i as u64);
        lines.push_str(&format!(
            "{{\"id\":\"s{i}\",\"split\":\"train\",\"label\":{label},\"lenses\":{{\"problem\":{},\"hints\":{}}}}}\n",
            vec_json(&problem),
            vec_json(&hints)
        ));
    }
    fs::write(dir.join("vectors.jsonl"), lines).unwrap();
    write_manifest(dir, rows, resolved);
}

/// Writes a 2-lens fixture whose lens vectors CLEANLY separate the binary label
/// (label fully recoverable from the surface form).
fn write_separable_corpus(dir: &Path, rows: usize) {
    let seed = 7_u64;
    let mut lines = String::new();
    let mut resolved = 0_usize;
    for i in 0..rows {
        let label = (i % 2) as u8;
        let is_one = label == 1;
        resolved += usize::from(label == 1);
        let problem = separable_lens(seed, i as u64, is_one);
        let hints = separable_lens(seed ^ 0xCD, i as u64, is_one);
        lines.push_str(&format!(
            "{{\"id\":\"s{i}\",\"split\":\"train\",\"label\":{label},\"lenses\":{{\"problem\":{},\"hints\":{}}}}}\n",
            vec_json(&problem),
            vec_json(&hints)
        ));
    }
    fs::write(dir.join("vectors.jsonl"), lines).unwrap();
    write_manifest(dir, rows, resolved);
}

/// Writes a corpus where every instance is resolved (single label).
fn write_single_label_corpus(dir: &Path, rows: usize) {
    let seed = 99_u64;
    let mut lines = String::new();
    for i in 0..rows {
        let problem = independent_lens(seed, i as u64);
        let hints = independent_lens(seed ^ 0xCD, i as u64);
        lines.push_str(&format!(
            "{{\"id\":\"s{i}\",\"split\":\"train\",\"label\":1,\"lenses\":{{\"problem\":{},\"hints\":{}}}}}\n",
            vec_json(&problem),
            vec_json(&hints)
        ));
    }
    fs::write(dir.join("vectors.jsonl"), lines).unwrap();
    write_manifest(dir, rows, rows);
}

fn write_manifest(dir: &Path, rows: usize, resolved: usize) {
    let manifest = format!(
        "{{\"oracle_model\":\"test-model\",\"dataset\":\"synthetic\",\"anchor\":\"test_pass_fail\",\"n\":{rows},\"resolved\":{resolved},\"embedding_model_id\":\"test-embed\",\"lenses\":[{{\"name\":\"problem\"}},{{\"name\":\"hints\"}}]}}\n"
    );
    fs::write(dir.join("manifest.json"), manifest).unwrap();
}

/// Lens vectors that depend on the index only (independent of any label).
fn independent_lens(seed: u64, i: u64) -> Vec<f32> {
    (0..DIM).map(|d| jitter(seed, i, d as u64)).collect()
}

/// Lens vectors whose first half encodes the label cleanly.
fn separable_lens(seed: u64, i: u64, is_one: bool) -> Vec<f32> {
    let offset = if is_one { 1.0 } else { -1.0 };
    (0..DIM)
        .map(|d| {
            let base = if d < DIM / 2 { offset } else { 0.0 };
            base + 0.05 * jitter(seed, i, d as u64)
        })
        .collect()
}

/// Deterministic pseudo-random value in [0, 1) from a hashed seed/index.
fn label_bucket(seed: u64, i: u64) -> f32 {
    (jitter(seed, i, 0) + 1.0) / 2.0
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
