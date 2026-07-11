use super::*;

#[test]
fn registry_contract_audit_uses_spec_hashes_without_rehashing_model_artifacts() {
    let vault = temp_vault_dir("audit-spec-hashes");
    let vault_id: VaultId = "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap();
    let shape = SlotShape::Dense(768);
    let norm = NormPolicy::unit();
    let model_id = "fixture/onnx-static-contract";
    let weights_sha256 = [0x9A; 32];
    let norm_text = format!("{:?}", norm);
    let corpus_hash = sha256_digest(&[
        b"onnx-custom-v1",
        model_id.as_bytes(),
        b"mean",
        norm_text.as_bytes(),
    ]);
    let contract = FrozenLensContract::new(
        "fixture-onnx-static-contract",
        weights_sha256,
        corpus_hash,
        shape,
        Modality::Text,
        LensDType::F32,
        norm,
    );
    let lens_id = contract.lens_id();
    let artifact_dir = vault.join("artifacts");
    fs::create_dir_all(&artifact_dir).unwrap();
    let model = artifact_dir.join("model.onnx");
    let tokenizer = artifact_dir.join("tokenizer.json");
    let config = artifact_dir.join("config.json");
    fs::write(
        &model,
        b"first bytes deliberately not equal to the persisted hash",
    )
    .unwrap();
    fs::write(&tokenizer, br#"{"tokenizer":"fixture"}"#).unwrap();
    fs::write(&config, br#"{"pooling":"mean"}"#).unwrap();
    let spec = LensSpec {
        name: contract.name().to_string(),
        runtime: LensRuntime::Onnx {
            model_id: model_id.to_string(),
            files: vec![model.clone(), tokenizer, config],
        },
        output: shape,
        modality: Modality::Text,
        weights_sha256,
        corpus_hash,
        norm_policy: norm,
        max_batch: Some(4),
        axis: None,
        asymmetry: Asymmetry::None,
        quant_default: QuantPolicy::turboquant_default(),
        truncate_dim: None,
        recall_delta: crate::spec::default_recall_delta(),
        retrieval_only: false,
        excluded_from_dedup: false,
    };
    let panel = panel_with_lens_shape(lens_id, shape);
    AsterVault::new_durable(
        &vault,
        vault_id,
        [0x7C; 32],
        VaultOptions {
            panel: Some(panel),
            ..VaultOptions::default()
        },
    )
    .unwrap();
    let manifest = ManifestStore::open(&vault).load_current().unwrap();
    install_registry_snapshot(
        &vault,
        &VaultRegistrySnapshot {
            version: 1,
            panel_ref: manifest.panel_ref,
            lenses: vec![RegistryLensSnapshot {
                lens_id,
                contract,
                spec: Some(spec),
                determinism: DeterminismProof::ContractOnlyExemption,
                runtime_golden: None,
            }],
        },
    );
    let before_manifest = ManifestStore::open(&vault).load_current().unwrap();
    println!(
        "before no-rehash audit: manifest_seq={} registry_ref={:?} model_bytes={}",
        before_manifest.manifest_seq,
        before_manifest
            .registry_ref
            .as_ref()
            .map(|reference| reference.logical_path.as_str()),
        fs::metadata(&model).unwrap().len()
    );

    fs::write(
        &model,
        b"changed model bytes that would alter a physical artifact hash if audit rehashed it",
    )
    .unwrap();
    let audit = audit_vault_registry_contracts(&vault).unwrap();
    let after_manifest = ManifestStore::open(&vault).load_current().unwrap();
    println!(
        "after no-rehash audit: valid={} checked_count={} diff_count={} manifest_seq={} model_bytes={}",
        audit.valid,
        audit.checked_count,
        audit.diffs.len(),
        after_manifest.manifest_seq,
        fs::metadata(&model).unwrap().len()
    );

    assert!(audit.valid);
    assert_eq!(audit.checked_count, 1);
    assert!(audit.diffs.is_empty());
    assert_eq!(before_manifest.registry_ref, after_manifest.registry_ref);
}
