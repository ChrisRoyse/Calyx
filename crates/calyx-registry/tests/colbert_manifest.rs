use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_core::{Modality, QuantPolicy, SlotShape};
use calyx_registry::{
    LensForgeFile, LensForgeManifest, LensForgeShape, LensRuntime, NormPolicy,
    lens_spec_from_manifest_path,
};
use sha2::{Digest, Sha256};

#[test]
fn explicit_colbert_shape_maps_to_multi_runtime() {
    let root = temp_root("colbert-shape");
    let model = write(&root, "model_int8.onnx", b"colbert model");
    let tokenizer = write(&root, "tokenizer.json", b"colbert tokenizer");
    let config = write(&root, "config.json", br#"{"hidden_size":384}"#);
    let manifest = LensForgeManifest {
        name: "answerai-colbert".to_string(),
        modality: Modality::Text,
        runtime: "onnx-colbert".to_string(),
        dim: 384,
        shape: Some(LensForgeShape::Multi { token_dim: 384 }),
        dtype: "int8".to_string(),
        weights_sha256: plain_sha256_hex(b"colbert model"),
        artifact_set_sha256: Some(artifact_hash(&[
            b"colbert model",
            b"colbert tokenizer",
            br#"{"hidden_size":384}"#,
        ])),
        files: vec![
            file("model", &model, b"colbert model"),
            file("tokenizer", &tokenizer, b"colbert tokenizer"),
            file("config", &config, br#"{"hidden_size":384}"#),
        ],
        pooling: "late-interaction".to_string(),
        norm: "finite".to_string(),
        source_hf_id: "answerdotai/answerai-colbert-small-v1".to_string(),
        endpoint: None,
        license: Some("apache-2.0".to_string()),
        non_commercial: false,
        quant_default: QuantPolicy::turboquant_default(),
        truncate_dim: None,
        recall_delta: calyx_registry::spec::default_recall_delta(),
        max_batch: Some(1),
    };
    let manifest_path = root.join("manifest.json");
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let spec = lens_spec_from_manifest_path(&manifest_path).unwrap();

    assert_eq!(spec.output, SlotShape::Multi { token_dim: 384 });
    assert_eq!(spec.norm_policy, NormPolicy::Finite);
    assert!(matches!(
        spec.runtime,
        LensRuntime::OnnxColbert { ref model_id, .. }
            if model_id == "answerdotai/answerai-colbert-small-v1"
    ));
}

fn temp_root(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "calyx-colbert-manifest-{label}-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn write(root: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = root.join(name);
    fs::write(&path, bytes).unwrap();
    path
}

fn file(role: &str, path: &Path, bytes: &[u8]) -> LensForgeFile {
    LensForgeFile {
        role: role.to_string(),
        path: path.file_name().unwrap().into(),
        sha256: plain_sha256_hex(bytes),
        bytes: bytes.len() as u64,
    }
}

fn artifact_hash(parts: &[&[u8]]) -> String {
    let slices = parts.to_vec();
    let digest = calyx_registry::frozen::sha256_digest(&slices);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn plain_sha256_hex(bytes: &[u8]) -> String {
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
