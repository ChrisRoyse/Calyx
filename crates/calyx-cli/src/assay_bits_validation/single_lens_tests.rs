use std::fs;
use std::path::Path;

use super::data::AssayCorpus;
use super::test_support::{request_for, temp_root, vec_json};

const DIM: usize = 16;

#[test]
fn single_lens_corpus_loads_for_candidate_assay() {
    let root = temp_root("assay-bits-single-lens");
    let corpus = root.join("corpus");
    fs::create_dir_all(&corpus).unwrap();
    write_single_lens_corpus(&corpus, 80, true);
    let request = request_for(&root);

    let data = AssayCorpus::load(&request).unwrap();

    assert_eq!(data.lenses.len(), 1);
    assert_eq!(data.lenses[0].name, "single_real");
    assert_eq!(data.lens_vectors[0].len(), 80);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn zero_lens_manifest_still_fails_closed() {
    let root = temp_root("assay-bits-zero-lens");
    let corpus = root.join("corpus");
    fs::create_dir_all(&corpus).unwrap();
    write_single_lens_corpus(&corpus, 80, false);
    let request = request_for(&root);

    let error = AssayCorpus::load(&request).unwrap_err();

    assert!(error.contains("need >=1 lenses"), "{error}");
    let _ = fs::remove_dir_all(root);
}

fn write_single_lens_corpus(dir: &Path, rows: usize, include_lens: bool) {
    let mut lines = String::new();
    for i in 0..rows {
        let label = i % 2;
        let offset = if label == 0 { 1.0 } else { -1.0 };
        let vector: Vec<f32> = (0..DIM)
            .map(|d| if d < DIM / 2 { offset } else { 0.0 })
            .collect();
        let lenses = if include_lens {
            format!("{{\"single_real\":{}}}", vec_json(&vector))
        } else {
            "{}".to_string()
        };
        lines.push_str(&format!(
            "{{\"id\":\"single-{i}\",\"split\":\"train\",\"label\":{label},\"lenses\":{lenses}}}\n"
        ));
    }
    fs::write(dir.join("vectors.jsonl"), lines).unwrap();

    let lenses = if include_lens {
        "[{\"name\":\"single_real\",\"redundant\":false}]"
    } else {
        "[]"
    };
    fs::write(
        dir.join("manifest.json"),
        format!(
            "{{\"dataset\":\"synthetic-single\",\"embedding_model_id\":\"test-embed\",\"n_samples\":{rows},\"label_counts\":{{\"0\":{half},\"1\":{half}}},\"lenses\":{lenses},\"target_class\":0}}\n",
            half = rows / 2
        ),
    )
    .unwrap();
}
