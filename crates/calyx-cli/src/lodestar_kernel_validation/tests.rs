use std::fs;

use super::data::CorpusSet;
use super::engine::evaluate_corpora;
use super::metrics::write_metric_outputs;
use super::request::LodestarKernelRequest;

#[test]
fn synthetic_three_corpora_persist_metric_files() {
    let root = temp_root("lodestar-kernel-pass");
    let corpora = root.join("corpora");
    fs::create_dir_all(&corpora).unwrap();
    write_dag_corpus(&corpora, "alpha", 30);
    write_dag_corpus(&corpora, "beta", 31);
    write_dag_corpus(&corpora, "gamma", 32);
    let request = request_for(&root, 0.95);
    let data = CorpusSet::load(&request.corpora_dir).unwrap();
    let report = evaluate_corpora(&data, &request).unwrap();
    let evidence = write_metric_outputs(&request, &report).unwrap();

    assert_eq!(evidence.corpora_passed, 3);
    assert!(evidence.min_observed_ratio >= 0.95);
    for path in evidence.ratio_files {
        assert!(
            fs::read_to_string(path)
                .unwrap()
                .trim()
                .parse::<f32>()
                .unwrap()
                >= 0.95
        );
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn insufficient_corpora_fails_closed_before_metrics() {
    let root = temp_root("lodestar-kernel-insufficient");
    let corpora = root.join("corpora");
    fs::create_dir_all(&corpora).unwrap();
    write_dag_corpus(&corpora, "alpha", 30);
    write_dag_corpus(&corpora, "beta", 30);
    let request = request_for(&root, 0.95);
    let data = CorpusSet::load(&request.corpora_dir).unwrap();

    let err = evaluate_corpora(&data, &request).unwrap_err();

    assert!(err.starts_with("CALYX_FSV_LODESTAR_INSUFFICIENT_CORPORA"));
    assert!(!request.metrics_dir.exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn too_small_corpus_fails_closed() {
    let root = temp_root("lodestar-kernel-small");
    let corpora = root.join("corpora");
    fs::create_dir_all(&corpora).unwrap();
    write_dag_corpus(&corpora, "alpha", 2);
    write_dag_corpus(&corpora, "beta", 30);
    write_dag_corpus(&corpora, "gamma", 30);
    let request = request_for(&root, 0.95);
    let data = CorpusSet::load(&request.corpora_dir).unwrap();

    let err = evaluate_corpora(&data, &request).unwrap_err();

    assert!(err.starts_with("CALYX_KERNEL_CORPUS_TOO_SMALL"));
    assert!(!request.metrics_dir.exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn missing_corpora_dir_reports_dataset_not_found() {
    let root = temp_root("lodestar-kernel-missing");
    let request = request_for(&root, 0.95);

    let err = CorpusSet::load(&request.corpora_dir).unwrap_err();

    assert!(err.starts_with("CALYX_DATASET_NOT_FOUND"));
    let _ = fs::remove_dir_all(root);
}

fn request_for(root: &std::path::Path, min_ratio: f32) -> LodestarKernelRequest {
    LodestarKernelRequest {
        corpora_dir: root.join("corpora"),
        metrics_dir: root.join("metrics"),
        min_ratio,
        query_limit: 20,
        top_k: 5,
    }
}

fn write_dag_corpus(dir: &std::path::Path, name: &str, rows: usize) {
    let nodes = (0..rows)
        .map(|idx| {
            let anchor = idx % 10 == 0;
            format!(
                "{{\"id\":\"n{idx}\",\"text\":\"{name} node {idx}\",\"anchor\":{anchor},\"features\":[{:.3},{:.3},{:.3}]}}",
                idx as f32,
                (idx % 7) as f32,
                (idx % 5) as f32
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let edges = (0..rows.saturating_sub(1))
        .map(|idx| format!("[\"n{idx}\",\"n{}\"]", idx + 1))
        .collect::<Vec<_>>()
        .join(",");
    let body = format!(
        "{{\"name\":\"{name}\",\"source_path\":\"synthetic\",\"source_sha256\":\"{:064x}\",\"nodes\":[{nodes}],\"edges\":[{edges}]}}\n",
        rows
    );
    fs::write(dir.join(format!("{name}.json")), body).unwrap();
}

fn temp_root(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("{name}-{}", std::process::id()))
}
