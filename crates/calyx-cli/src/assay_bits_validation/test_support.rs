use std::fs;
use std::path::{Path, PathBuf};

use super::request::AssayBitsRequest;

const DIM: usize = 16;

pub(super) fn request_for(root: &Path) -> AssayBitsRequest {
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

/// Writes a deterministic three-lens fixture (seed=42).
pub(super) fn write_synthetic_corpus(dir: &Path, rows: usize) {
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

pub(super) fn temp_root(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("{name}-{}", std::process::id()))
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

pub(super) fn vec_json(values: &[f32]) -> String {
    let parts: Vec<String> = values.iter().map(|v| format!("{v:.6}")).collect();
    format!("[{}]", parts.join(","))
}
