use std::fs;
use std::path::{Path, PathBuf};

use super::engine::evaluate;
use super::metrics::write_outputs;
use super::request::EnsembleCardRequest;

const DIM: usize = 6;

#[test]
fn ensemble_card_command_persists_payload_and_writes_artifacts() {
    let root = temp_root("assay-ensemble-card-pass");
    let corpus = root.join("corpus");
    fs::create_dir_all(&corpus).unwrap();
    write_corpus(&corpus, 120, 10);
    let request = request_for(&root);
    let report = evaluate(&request).unwrap();
    let evidence = write_outputs(&request, &report).unwrap();

    assert_eq!(report.card.panel_lens_count, 10);
    assert_eq!(report.card.pairs.len(), 45);
    assert_eq!(report.assay_cf_rows_persisted, 58);
    assert_eq!(report.assay_cf_subject_counts["ensemble_card"], 1);
    assert_eq!(report.assay_cf_subject_counts["lens"], 10);
    assert_eq!(report.assay_cf_subject_counts["pair"], 45);
    assert!(report.ensemble_card_row_present);
    assert!(report.ensemble_card_payload_readback);
    assert!(Path::new(&evidence.ensemble_card_path).exists());

    let card_json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&evidence.ensemble_card_path).unwrap()).unwrap();
    assert_eq!(card_json["panel_lens_count"], 10);
    assert_eq!(
        card_json["pid_method"],
        calyx_assay::ENSEMBLE_CARD_PID_METHOD
    );
    assert!(
        fs::read_to_string(&evidence.lens_values_path)
            .unwrap()
            .contains("marginal=")
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn sub_ten_command_fails_closed_before_verdicts() {
    let root = temp_root("assay-ensemble-card-small");
    let corpus = root.join("corpus");
    fs::create_dir_all(&corpus).unwrap();
    write_corpus(&corpus, 120, 9);
    let request = request_for(&root);
    let error = evaluate(&request).unwrap_err();

    assert!(error.starts_with(calyx_assay::CALYX_ASSAY_PANEL_TOO_SMALL));
    assert!(error.contains("at least 10"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn sample_quorum_errors_keep_assay_code() {
    let error = super::ensemble_cli_error(
        "CALYX_FSV_ASSAY_INVALID_CORPUS: need >=50 samples, got 49".to_string(),
    );

    assert_eq!(error.code(), "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    assert!(error.message().contains("got 49"));
}

fn request_for(root: &Path) -> EnsembleCardRequest {
    let metrics = root.join("metrics");
    EnsembleCardRequest {
        corpus_dir: root.join("corpus"),
        metrics_dir: metrics.clone(),
        cf_root: metrics.join("assay_cf"),
        target_class: 0,
        domain: "ensemble_card_test".to_string(),
        min_lenses: 10,
        min_marginal_bits: 0.05,
        max_redundancy: 0.6,
    }
}

fn write_corpus(dir: &Path, rows: usize, lens_count: usize) {
    let mut lines = String::new();
    for row in 0..rows {
        let label = row % 2;
        let mut lens_json = Vec::new();
        for lens in 0..lens_count {
            let name = lens_name(lens);
            let values = if lens == lens_count - 1 && lens_count == 10 {
                vector(row, label == 0, 0, 1.0, 0.02)
            } else {
                vector(row, label == 0, lens as u64, weight(lens), noise(lens))
            };
            lens_json.push(format!("\"{name}\":{}", vec_json(&values)));
        }
        lines.push_str(&format!(
            "{{\"id\":\"s{row}\",\"label\":{label},\"lenses\":{{{}}}}}\n",
            lens_json.join(",")
        ));
    }
    fs::write(dir.join("vectors.jsonl"), lines).unwrap();
    let lenses = (0..lens_count)
        .map(|idx| {
            let redundant = idx == lens_count - 1 && lens_count == 10;
            format!(
                "{{\"name\":\"{}\",\"redundant\":{}}}",
                lens_name(idx),
                redundant
            )
        })
        .collect::<Vec<_>>();
    let manifest = format!(
        "{{\"dataset\":\"synthetic-ensemble\",\"embedding_model_id\":\"fixture\",\"n_samples\":{rows},\"lenses\":[{}],\"target_class\":0}}\n",
        lenses.join(",")
    );
    fs::write(dir.join("manifest.json"), manifest).unwrap();
}

fn lens_name(idx: usize) -> String {
    if idx == 9 {
        "redundant_a".to_string()
    } else {
        format!("lens_{idx}")
    }
}

fn vector(row: usize, positive: bool, seed: u64, weight: f32, noise: f32) -> Vec<f32> {
    let signal = if positive { 1.0 } else { -1.0 };
    (0..DIM)
        .map(|dim| signal * weight + noise * jitter(seed, row, dim))
        .collect()
}

fn weight(idx: usize) -> f32 {
    if idx.is_multiple_of(2) { 0.62 } else { -0.54 }
}

fn noise(idx: usize) -> f32 {
    0.38 + (idx as f32 % 4.0) * 0.06
}

fn jitter(seed: u64, row: usize, dim: usize) -> f32 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&seed.to_be_bytes());
    hasher.update(&(row as u64).to_be_bytes());
    hasher.update(&(dim as u64).to_be_bytes());
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
    let parts = values
        .iter()
        .map(|value| format!("{value:.6}"))
        .collect::<Vec<_>>();
    format!("[{}]", parts.join(","))
}

fn temp_root(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("{name}-{}", std::process::id()))
}
