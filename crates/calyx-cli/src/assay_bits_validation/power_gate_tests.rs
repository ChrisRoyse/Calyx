use std::fs;
use std::path::Path;

use calyx_assay::{CALYX_ASSAY_DEGENERATE_TARGET_ENTROPY, CALYX_ASSAY_ESTIMATOR_UNDERPOWERED};

use super::test_support::{temp_root, vec_json};

#[test]
fn bits_validate_maps_entropy_floor_to_assay_code() {
    let root = temp_root("assay-bits-entropy-floor");
    let _ = fs::remove_dir_all(&root);
    let corpus = root.join("corpus");
    fs::create_dir_all(&corpus).unwrap();
    write_imbalanced_corpus(&corpus, 200);

    let args = vec![
        "--corpus-dir".to_string(),
        corpus.display().to_string(),
        "--metrics-dir".to_string(),
        root.join("metrics").display().to_string(),
        "--target-class".to_string(),
        "0".to_string(),
        "--domain".to_string(),
        "issue874_entropy_floor".to_string(),
    ];

    let error = super::run(&args).unwrap_err();

    assert_eq!(error.code(), CALYX_ASSAY_DEGENERATE_TARGET_ENTROPY);
    assert!(!root.join("metrics").exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn cli_maps_underpowered_runtime_code_without_downgrading_it() {
    let error = super::assay_cli_error(format!(
        "{CALYX_ASSAY_ESTIMATOR_UNDERPOWERED}: n=64 dim=4096 recovered planted signal below floor"
    ));

    assert_eq!(error.code(), CALYX_ASSAY_ESTIMATOR_UNDERPOWERED);
    assert!(error.message().contains("n=64"));
}

fn write_imbalanced_corpus(dir: &Path, rows: usize) {
    let mut lines = String::new();
    for i in 0..rows {
        let label = usize::from(i != 0);
        let value = if label == 0 { 1.0 } else { -1.0 };
        let vector = vec![value, i as f32 / rows as f32, 0.25, -0.25];
        lines.push_str(&format!(
            "{{\"id\":\"s{i}\",\"label\":{label},\"lenses\":{{\"weak_lens\":{}}}}}\n",
            vec_json(&vector)
        ));
    }
    fs::write(dir.join("vectors.jsonl"), lines).unwrap();
    fs::write(
        dir.join("manifest.json"),
        format!(
            "{{\"dataset\":\"entropy_floor\",\"embedding_model_id\":\"test-embed\",\"n_samples\":{rows},\"label_counts\":{{\"0\":1,\"1\":{negatives}}},\"lenses\":[{{\"name\":\"weak_lens\",\"redundant\":false}}],\"target_class\":0}}\n",
            negatives = rows - 1
        ),
    )
    .unwrap();
}
