use std::fs;

use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::VaultStore;

use super::data::ValidationData;
use super::engine::evaluate_emotion;
use super::metrics::write_metric_outputs;
use super::request::{DEFAULT_VAULT_ID, EmotionRequest};

#[test]
fn synthetic_emotion_bits_persist_assay_rows() {
    let root = temp_root("media-emotion-known");
    let request = request_for(&root, 0.05);
    write_samples(&request.samples, synthetic_rows(60));
    let data = ValidationData::load(&request.samples).unwrap();
    let report = evaluate_emotion(&data, &request).unwrap();
    assert!(report.emotion_bits.bits >= 0.05);
    assert_eq!(report.emotion_label_count, 3);

    let vault = vault_for(&request);
    let evidence = write_metric_outputs(&vault, &request, report).unwrap();
    assert_eq!(evidence.assay_rows_persisted, 2);
    assert_eq!(evidence.assay_rows_loaded, 2);
    assert!(vault.snapshot() > 0);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn empty_sample_file_fails_closed() {
    let root = temp_root("media-emotion-empty");
    let samples = root.join("samples.jsonl");
    fs::create_dir_all(&root).unwrap();
    fs::write(&samples, b"\n").unwrap();

    assert_eq!(
        ValidationData::load(&samples).unwrap_err(),
        "CALYX_FSV_MEDIA_EMOTION_EMPTY_DATASET"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn invalid_feature_fails_closed_before_metrics() {
    let root = temp_root("media-emotion-invalid");
    let samples = root.join("samples.jsonl");
    write_samples(
        &samples,
        r#"{"sample_id":"bad","dataset":"synthetic","audio_features":[],"emotion_label":0}"#
            .to_string(),
    );

    let err = ValidationData::load(&samples).unwrap_err();
    assert!(err.contains("CALYX_FSV_MEDIA_EMOTION_INVALID_FEATURE"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn duplicate_sample_id_fails_closed_before_metrics() {
    let root = temp_root("media-emotion-duplicate-id");
    let samples = root.join("samples.jsonl");
    write_samples(
        &samples,
        [
            r#"{"sample_id":"dup","dataset":"synthetic","audio_features":[1.0,2.0],"emotion_label":0,"source_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#,
            r#"{"sample_id":"dup","dataset":"synthetic","audio_features":[3.0,4.0],"emotion_label":1,"source_sha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}"#,
        ]
        .join("\n")
            + "\n",
    );

    assert!(
        ValidationData::load(&samples)
            .unwrap_err()
            .contains("CALYX_FSV_MEDIA_EMOTION_DUPLICATE_SAMPLE_ID")
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn single_class_domain_fails_closed() {
    let root = temp_root("media-emotion-one-class");
    let request = request_for(&root, 0.05);
    write_samples(&request.samples, one_class_rows(60));
    let data = ValidationData::load(&request.samples).unwrap();

    let err = evaluate_emotion(&data, &request).unwrap_err();
    assert!(err.contains("CALYX_FSV_MEDIA_EMOTION_LABEL_DOMAIN_MISMATCH"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn threshold_gate_fails_closed() {
    let root = temp_root("media-emotion-threshold");
    let request = request_for(&root, 9.0);
    write_samples(&request.samples, synthetic_rows(60));
    let data = ValidationData::load(&request.samples).unwrap();

    let err = evaluate_emotion(&data, &request).unwrap_err();
    assert!(err.contains("CALYX_FSV_MEDIA_EMOTION_BITS_BELOW_THRESHOLD"));
    let _ = fs::remove_dir_all(root);
}

fn synthetic_rows(count: usize) -> String {
    rows(count, |idx| idx % 3)
}

fn one_class_rows(count: usize) -> String {
    rows(count, |_| 0)
}

fn rows(count: usize, label_for: impl Fn(usize) -> usize) -> String {
    let mut out = String::new();
    for idx in 0..count {
        let label = label_for(idx);
        let base = label as f32 * 10.0;
        let jitter = idx as f32 * 0.001;
        let features = vec![base + jitter, base * 0.5 + jitter, base * 0.25 + jitter];
        out.push_str(&format!(
            "{{\"sample_id\":\"s{idx}\",\"dataset\":\"synthetic\",\"audio_features\":{:?},\"emotion_label\":{label},\"source_sha256\":\"{:064x}\"}}\n",
            features, idx
        ));
    }
    out
}

fn request_for(root: &std::path::Path, min_bits: f32) -> EmotionRequest {
    let _ = fs::remove_dir_all(root);
    fs::create_dir_all(root).unwrap();
    EmotionRequest {
        samples: root.join("samples.jsonl"),
        metrics_dir: root.join("metrics"),
        vault: root.join("vault"),
        min_bits,
        k: 3,
        vault_id: DEFAULT_VAULT_ID.to_string(),
        vault_salt: "calyx-test-media-emotion".to_string(),
    }
}

fn vault_for(request: &EmotionRequest) -> AsterVault {
    AsterVault::new_durable(
        &request.vault,
        DEFAULT_VAULT_ID.parse().unwrap(),
        request.vault_salt.as_bytes().to_vec(),
        VaultOptions::default(),
    )
    .unwrap()
}

fn temp_root(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("{name}-{}", std::process::id()))
}

fn write_samples(path: &std::path::Path, content: String) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}
