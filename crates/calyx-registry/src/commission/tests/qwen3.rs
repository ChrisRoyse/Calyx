use super::*;

#[test]
fn qwen3_manifest_preserves_explicit_max_tokens_and_defaults_legacy() {
    let root = temp_root("qwen3-max-tokens");
    let weights = write(&root, "model.safetensors", b"qwen3 weights");
    let tokenizer = write(&root, "tokenizer.json", br#"{"tokenizer":true}"#);
    let config = write(&root, "config.json", br#"{"hidden_size":1024}"#);
    let files = vec![
        file("model", &weights, b"qwen3 weights"),
        file("tokenizer", &tokenizer, br#"{"tokenizer":true}"#),
        file("config", &config, br#"{"hidden_size":1024}"#),
    ];
    let mut manifest = LensForgeManifest {
        name: "qwen3-local".to_string(),
        modality: Modality::Text,
        runtime: "fastembed-qwen3".to_string(),
        dim: 1024,
        shape: Some(super::super::LensForgeShape::Dense { dim: 1024 }),
        dtype: "f16".to_string(),
        weights_sha256: plain_sha256_hex(b"qwen3 weights"),
        artifact_set_sha256: Some(artifact_hash(&[
            b"qwen3 weights",
            br#"{"tokenizer":true}"#,
            br#"{"hidden_size":1024}"#,
        ])),
        files,
        pooling: "mean".to_string(),
        norm: "unit".to_string(),
        source_hf_id: "Qwen/Qwen3-Embedding-0.6B".to_string(),
        endpoint: None,
        license: Some("apache-2.0".to_string()),
        non_commercial: false,
        quant_default: QuantPolicy::turboquant_default(),
        truncate_dim: None,
        recall_delta: crate::spec::default_recall_delta(),
        max_batch: None,
        max_tokens: Some(8_192),
        representation: None,
        special_tokens: None,
        batch_policy: None,
    };

    let explicit = lens_spec_from_manifest_with_license_override(&manifest, &root, false).unwrap();
    assert!(matches!(
        explicit.runtime,
        LensRuntime::FastembedQwen3 {
            max_tokens: 8_192,
            ..
        }
    ));

    manifest.max_tokens = None;
    let legacy = lens_spec_from_manifest_with_license_override(&manifest, &root, false).unwrap();
    assert!(matches!(
        legacy.runtime,
        LensRuntime::FastembedQwen3 {
            max_tokens: crate::DEFAULT_QWEN3_MAX_TOKENS,
            ..
        }
    ));
    assert_ne!(explicit.corpus_hash, legacy.corpus_hash);
    assert_ne!(explicit.lens_id(), legacy.lens_id());
}
