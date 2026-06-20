use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_core::{Modality, QuantPolicy};
use calyx_registry::{LensForgeFile, LensForgeManifest, LensRuntime, lens_spec_from_manifest_path};
use sha2::{Digest, Sha256};

#[test]
fn onnx_fastembed_manifest_preserves_core_file_order_and_sidecars() {
    let root = temp_root("onnx-fastembed-sidecar");
    let model = write(&root, "model.onnx", b"model");
    let tokenizer = write(&root, "tokenizer.json", b"tokenizer");
    let config = write(&root, "config.json", br#"{"hidden_size":3}"#);
    let sidecar = write(&root, "model.onnx_data", b"sidecar");
    let manifest = LensForgeManifest {
        name: "fastembed-fixture".to_string(),
        modality: Modality::Text,
        runtime: "onnx-fastembed".to_string(),
        dim: 3,
        shape: None,
        dtype: "f32".to_string(),
        weights_sha256: plain_sha256_hex(b"model"),
        artifact_set_sha256: None,
        files: vec![
            file("model", &model, b"model"),
            file("model_sidecar", &sidecar, b"sidecar"),
            file("tokenizer", &tokenizer, b"tokenizer"),
            file("config", &config, br#"{"hidden_size":3}"#),
        ],
        pooling: "mean".to_string(),
        norm: "unit".to_string(),
        source_hf_id: "BAAI/bge-m3".to_string(),
        endpoint: None,
        license: Some("mit".to_string()),
        non_commercial: false,
        quant_default: QuantPolicy::turboquant_default(),
        truncate_dim: None,
        recall_delta: calyx_registry::spec::default_recall_delta(),
        max_batch: None,
    };
    fs::write(
        root.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let spec = lens_spec_from_manifest_path(root.join("manifest.json")).unwrap();
    let LensRuntime::Onnx { model_id, files } = spec.runtime else {
        panic!("expected ONNX runtime");
    };

    assert_eq!(model_id, "BAAI/bge-m3");
    assert!(files[0].ends_with("model.onnx"));
    assert!(files[1].ends_with("tokenizer.json"));
    assert!(files[2].ends_with("config.json"));
    assert!(files.iter().any(|path| path.ends_with("model.onnx_data")));
}

fn temp_root(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "calyx-fastembed-manifest-{label}-{}-{nanos}",
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

fn plain_sha256_hex(bytes: &[u8]) -> String {
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
