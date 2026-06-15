use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_core::{Modality, SlotShape};
use sha2::{Digest, Sha256};

use super::{LensForgeFile, LensForgeManifest, lens_spec_from_manifest_path};
use crate::frozen::{NormPolicy, sha256_digest};
use crate::spec::LensRuntime;

#[test]
fn lensforge_manifest_round_trips_to_stable_lens_spec() {
    let root = temp_root("round-trip");
    let model = write(&root, "model_int8.onnx", b"tiny model bytes");
    let tokenizer = write(&root, "tokenizer.json", br#"{"tiny":true}"#);
    let config = write(&root, "config.json", br#"{"hidden_size":3}"#);
    let files = vec![
        file("model", &model, b"tiny model bytes"),
        file("tokenizer", &tokenizer, br#"{"tiny":true}"#),
        file("config", &config, br#"{"hidden_size":3}"#),
    ];
    let manifest = LensForgeManifest {
        name: "tiny-text".to_string(),
        modality: Modality::Text,
        runtime: "onnx-int8".to_string(),
        dim: 3,
        dtype: "int8".to_string(),
        weights_sha256: plain_sha256_hex(b"tiny model bytes"),
        artifact_set_sha256: Some(artifact_hash(&[
            b"tiny model bytes",
            br#"{"tiny":true}"#,
            br#"{"hidden_size":3}"#,
        ])),
        files,
        pooling: "mean".to_string(),
        norm: "l2".to_string(),
        source_hf_id: "fixture/tiny".to_string(),
        license: Some("apache-2.0".to_string()),
        non_commercial: false,
    };
    let manifest_path = root.join("manifest.json");
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let first = lens_spec_from_manifest_path(&manifest_path).unwrap();
    let second = lens_spec_from_manifest_path(&manifest_path).unwrap();

    assert_eq!(first.lens_id(), second.lens_id());
    assert_eq!(first.output, SlotShape::Dense(3));
    assert_eq!(first.modality, Modality::Text);
    assert_eq!(first.norm_policy, NormPolicy::unit());
    assert!(matches!(
        first.runtime,
        LensRuntime::Onnx { ref model_id, .. } if model_id == "fixture/tiny"
    ));
    assert_eq!(
        hex_from_bytes(&first.weights_sha256),
        manifest.artifact_set_sha256.unwrap()
    );
}

#[test]
fn lensforge_manifest_missing_required_field_is_config_invalid() {
    let root = temp_root("missing-field");
    write(&root, "model_int8.onnx", b"model");
    let manifest = root.join("manifest.json");
    fs::write(
        &manifest,
        br#"{
  "name": "bad",
  "modality": "text",
  "runtime": "onnx-int8",
  "dtype": "int8",
  "weights_sha256": "0000000000000000000000000000000000000000000000000000000000000000",
  "files": [],
  "pooling": "mean",
  "norm": "l2",
  "source_hf_id": "fixture/bad"
}"#,
    )
    .unwrap();

    let error = lens_spec_from_manifest_path(&manifest).unwrap_err();

    assert_eq!(error.code, "CALYX_LENS_CONFIG_INVALID");
    assert!(error.message.contains("dim"), "{}", error.message);
}

#[test]
fn model2vec_manifest_maps_to_static_lookup_runtime() {
    let root = temp_root("model2vec-static");
    let matrix = write(&root, "embeddings.cslm", &static_matrix_bytes());
    let tokenizer = write(&root, "tokenizer.json", br#"{ "tokenizer": true }"#);
    let files = vec![
        file("embeddings", &matrix, &static_matrix_bytes()),
        file("tokenizer", &tokenizer, br#"{ "tokenizer": true }"#),
    ];
    let manifest = LensForgeManifest {
        name: "tiny-model2vec".to_string(),
        modality: Modality::Text,
        runtime: "model2vec".to_string(),
        dim: 2,
        dtype: "int8".to_string(),
        weights_sha256: plain_sha256_hex(&static_matrix_bytes()),
        artifact_set_sha256: Some(artifact_hash(&[
            &static_matrix_bytes(),
            br#"{ "tokenizer": true }"#,
        ])),
        files,
        pooling: "mean".to_string(),
        norm: "l2".to_string(),
        source_hf_id: "minishlab/potion-base-8M".to_string(),
        license: Some("mit".to_string()),
        non_commercial: false,
    };
    let manifest_path = root.join("manifest.json");
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let spec = lens_spec_from_manifest_path(&manifest_path).unwrap();

    assert_eq!(spec.output, SlotShape::Dense(2));
    assert!(matches!(
        spec.runtime,
        LensRuntime::StaticLookup { ref embeddings_file, ref tokenizer, dim }
            if dim == 2
                && embeddings_file.ends_with("embeddings.cslm")
                && tokenizer.ends_with("tokenizer.json")
    ));
    assert_eq!(
        hex_from_bytes(&spec.weights_sha256),
        manifest.artifact_set_sha256.unwrap()
    );
}

#[test]
fn candle_fp16_manifest_preserves_runtime_dtype_and_pooling() {
    let root = temp_root("candle-fp16");
    let weights = write(&root, "model.safetensors", b"tiny candle weights");
    let tokenizer = write(&root, "tokenizer.json", br#"{"tokenizer":true}"#);
    let config = write(&root, "config.json", br#"{"hidden_size":3}"#);
    let files = vec![
        file("model", &weights, b"tiny candle weights"),
        file("tokenizer", &tokenizer, br#"{"tokenizer":true}"#),
        file("config", &config, br#"{"hidden_size":3}"#),
    ];
    let manifest = LensForgeManifest {
        name: "tiny-candle".to_string(),
        modality: Modality::Text,
        runtime: "candle-fp16".to_string(),
        dim: 3,
        dtype: "f16".to_string(),
        weights_sha256: plain_sha256_hex(b"tiny candle weights"),
        artifact_set_sha256: Some(artifact_hash(&[
            b"tiny candle weights",
            br#"{"tokenizer":true}"#,
            br#"{"hidden_size":3}"#,
        ])),
        files,
        pooling: "cls".to_string(),
        norm: "l2".to_string(),
        source_hf_id: "sentence-transformers/all-MiniLM-L6-v2".to_string(),
        license: Some("apache-2.0".to_string()),
        non_commercial: false,
    };
    let manifest_path = root.join("manifest.json");
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let spec = lens_spec_from_manifest_path(&manifest_path).unwrap();

    assert_eq!(spec.output, SlotShape::Dense(3));
    assert!(matches!(
        spec.runtime,
        LensRuntime::CandleLocal { ref model_id, ref files, ref dtype, ref pooling }
            if model_id == "sentence-transformers/all-MiniLM-L6-v2"
                && files[0].ends_with("model.safetensors")
                && files[1].ends_with("tokenizer.json")
                && files[2].ends_with("config.json")
                && dtype == "f16"
                && pooling == "cls"
    ));
    assert_eq!(
        hex_from_bytes(&spec.weights_sha256),
        manifest.artifact_set_sha256.unwrap()
    );
}

fn temp_root(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "calyx-lensforge-{label}-{}-{nanos}",
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

fn static_matrix_bytes() -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"CXLKUP1\0");
    bytes.extend_from_slice(&2_u32.to_le_bytes());
    bytes.extend_from_slice(&2_u32.to_le_bytes());
    bytes.push(1);
    bytes.extend_from_slice(&[0, 0, 0]);
    bytes.extend_from_slice(&1.0_f32.to_le_bytes());
    bytes.extend_from_slice(&[1_u8, 0, 0, 1]);
    bytes
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
    hex_from_bytes(&sha256_digest(parts))
}

fn plain_sha256_hex(bytes: &[u8]) -> String {
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    hex_from_bytes(&digest)
}

fn hex_from_bytes(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
