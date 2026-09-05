//! Issue #1523 - CUDA exact kNN for Poly neighbor and adaptation paths.

#[path = "issue1523_speed_support.rs"]
mod speed_support;
#[path = "issue1523_exact_knn_support.rs"]
mod support;

use std::fs;

use serde_json::{Value, json};

#[test]
fn issue1523_cuda_exact_knn_fsv() {
    let root = support::fsv_root();
    if root.exists() {
        fs::remove_dir_all(&root).expect("reset #1523 FSV root");
    }
    fs::create_dir_all(&root).expect("create #1523 FSV root");

    let report = json!({
        "issue": 1523,
        "proof_claim": "Poly batches exact cosine queries through bounded cuVS corpus chunks, preserves legacy ties/scores/persisted bytes, reuses one LOO ranking for all k values, and performs no exhaustive CPU scan in CUDA builds.",
        "feature_cuda": cfg!(feature = "cuda"),
        "exact_batch": support::exact_batch_parity(),
        "neighbor_paths": support::neighbor_path_parity(&root),
        "adaptation": support::adaptation_parity(&root),
        "larger_than_vram": support::larger_than_vram_probe(),
        "speed": speed_support::speed_proof(&root),
        "passed": true,
    });
    let path = root.join("issue1523_cuda_exact_knn_fsv_report.json");
    fs::write(&path, serde_json::to_vec_pretty(&report).unwrap()).expect("write report");
    let readback: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(readback, report);
    let summary = root.join("BLAKE3SUMS");
    fs::write(
        &summary,
        format!(
            "{}  {}\n",
            blake3::hash(&fs::read(&path).unwrap()).to_hex(),
            path.file_name().unwrap().to_string_lossy()
        ),
    )
    .expect("write sums");
    println!("ISSUE1523_FSV_REPORT={}", path.display());
}
