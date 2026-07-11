use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Asymmetry, Input, Modality, Panel, QuantPolicy, Slot, SlotId, SlotKey, SlotState, VaultId,
};

use super::*;
use crate::frozen::{LensDType, NormPolicy, sha256_digest};
use crate::{
    AlgorithmicLens, DeterminismProof, ExternalCmdLens, FrozenLensContract, LensRuntime, LensSpec,
    RuntimeGolden,
};

#[test]
fn snapshot_measurement_chunks_by_runtime_limit_and_reports_stats() {
    let snapshot = algorithmic_snapshot(Some(3));
    let inputs = text_inputs(["alpha", "beta", "gamma", "delta", "epsilon"]);

    let (vectors, stats) =
        measure_registry_snapshot_lens_batch_with_stats(&snapshot, &inputs, Some(2)).unwrap();

    assert_eq!(vectors.len(), 5);
    assert!(
        vectors
            .iter()
            .all(|vector| matches!(vector, SlotVector::Dense { dim: 16, .. }))
    );
    assert_eq!(stats.input_count, 5);
    assert_eq!(stats.runtime_batch_limit, Some(2));
    assert_eq!(stats.effective_chunk_size, 2);
    assert_eq!(stats.chunk_count, 3);
}

#[test]
fn snapshot_measurement_empty_input_reports_zero_chunks() {
    let snapshot = algorithmic_snapshot(Some(3));

    let (vectors, stats) =
        measure_registry_snapshot_lens_batch_with_stats(&snapshot, &[], Some(2)).unwrap();

    assert!(vectors.is_empty());
    assert_eq!(stats.input_count, 0);
    assert_eq!(stats.effective_chunk_size, 2);
    assert_eq!(stats.chunk_count, 0);
}

#[test]
fn snapshot_measurement_rejects_zero_runtime_limit() {
    let snapshot = algorithmic_snapshot(Some(3));
    let inputs = text_inputs(["alpha"]);

    let error =
        measure_registry_snapshot_lens_batch_with_stats(&snapshot, &inputs, Some(0)).unwrap_err();

    assert_eq!(error.code, "CALYX_LENS_UNREACHABLE");
    assert!(error.message.contains("runtime batch limit must be > 0"));
}

#[test]
fn loaded_snapshot_lens_reuses_runtime_and_reports_zero_per_request_load() {
    let snapshot = algorithmic_snapshot(Some(2));
    let loaded = LoadedRegistrySnapshotLens::load(snapshot).unwrap();
    let first_inputs = text_inputs(["alpha", "beta", "gamma"]);
    let second_inputs = text_inputs(["delta", "epsilon"]);

    let (first_vectors, first_stats) = loaded
        .measure_batch_with_stats(&first_inputs, Some(2))
        .unwrap();
    let (second_vectors, second_stats) = loaded
        .measure_batch_with_stats(&second_inputs, Some(2))
        .unwrap();

    println!(
        "loaded_snapshot_lens_state load_ms={} first_stats={first_stats:?} second_stats={second_stats:?}",
        loaded.runtime_load_ms()
    );
    assert_eq!(first_vectors.len(), 3);
    assert_eq!(second_vectors.len(), 2);
    assert_eq!(first_stats.runtime_load_ms, 0);
    assert_eq!(second_stats.runtime_load_ms, 0);
    assert_eq!(first_stats.effective_chunk_size, 2);
    assert_eq!(first_stats.chunk_count, 2);
    assert_eq!(second_stats.effective_chunk_size, 2);
    assert_eq!(second_stats.chunk_count, 1);
}

#[test]
fn process_runtime_snapshot_without_golden_fails_closed() {
    let contract = FrozenLensContract::tei_http(
        "missing-process-golden",
        "http://127.0.0.1:9/embed",
        Modality::Text,
        4,
    );
    let snapshot = RegistryLensSnapshot {
        lens_id: contract.lens_id(),
        contract: contract.clone(),
        spec: Some(runtime_spec(
            &contract,
            LensRuntime::TeiHttp {
                endpoint: "http://127.0.0.1:9/embed".to_string(),
            },
        )),
        determinism: DeterminismProof::ContractOnlyExemption,
        runtime_golden: None,
    };

    let error = match super::runtime::load_runtime_lens(&snapshot) {
        Ok(_) => panic!("process runtime without a golden unexpectedly loaded"),
        Err(error) => error,
    };

    assert_eq!(error.code, "CALYX_LENS_RUNTIME_DRIFT");
    assert!(error.message.contains("no registration golden"));
}

#[test]
fn process_runtime_snapshot_rejects_weakened_golden_tolerance() {
    let contract = FrozenLensContract::tei_http(
        "weak-process-golden",
        "http://127.0.0.1:9/embed",
        Modality::Text,
        4,
    );
    let probe = Input::new(Modality::Text, b"runtime identity".to_vec());
    let snapshot = RegistryLensSnapshot {
        lens_id: contract.lens_id(),
        contract: contract.clone(),
        spec: Some(runtime_spec(
            &contract,
            LensRuntime::TeiHttp {
                endpoint: "http://127.0.0.1:9/embed".to_string(),
            },
        )),
        determinism: DeterminismProof::ProbeVerified,
        runtime_golden: Some(RuntimeGolden {
            lens_id: contract.lens_id(),
            runtime_version: "tei-http-golden-v1".to_string(),
            probe,
            golden_output: vec![0.5; 4],
            tolerance: 1.0,
        }),
    };

    let error = match super::runtime::load_runtime_lens(&snapshot) {
        Ok(_) => panic!("process runtime with weak tolerance unexpectedly loaded"),
        Err(error) => error,
    };

    assert_eq!(error.code, "CALYX_LENS_RUNTIME_DRIFT");
    assert!(error.message.contains("tolerance"));
}

#[cfg(unix)]
#[test]
fn vault_open_rejects_external_runtime_behavior_drift() {
    let vault = temp_vault_dir("external-runtime-drift");
    fs::create_dir_all(&vault).unwrap();
    let model_state = vault.join("served-model.txt");
    let script = vault.join("external-runtime.py");
    fs::write(&model_state, "1").unwrap();
    fs::write(
        &script,
        format!(
            r#"import json, pathlib, struct, sys
value = float(pathlib.Path({}).read_text())
while True:
    header = sys.stdin.buffer.read(4)
    if not header:
        break
    size = struct.unpack(">I", header)[0]
    payload = json.loads(sys.stdin.buffer.read(size))
    vectors = [[value, value + 1, value + 2, value + 3] for _ in payload["inputs"]]
    body = json.dumps({{"vectors": vectors}}).encode()
    sys.stdout.buffer.write(struct.pack(">I", len(body)))
    sys.stdout.buffer.write(body)
    sys.stdout.buffer.flush()
"#,
            serde_json::to_string(model_state.to_str().unwrap()).unwrap()
        ),
    )
    .unwrap();

    let name = "external-runtime-drift";
    let command = "python3";
    let args = vec![script.display().to_string()];
    let contract = external_contract(name, command, &args, 4);
    let lens = ExternalCmdLens::new(name, command, args.clone(), Modality::Text, 4);
    let spec = runtime_spec(
        &contract,
        LensRuntime::ExternalCmd {
            cmd: command.to_string(),
            args,
        },
    );
    let lens_id = contract.lens_id();
    let mut registry = Registry::new();
    registry
        .register_frozen_with_spec(lens, contract, spec)
        .unwrap();
    assert!(registry.lens_snapshots()[0].runtime_golden.is_some());

    let panel = panel_with_runtime_lens(lens_id, 4, name);
    let vault_id: VaultId = "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap();
    AsterVault::new_durable(
        &vault,
        vault_id,
        [0x6B; 32],
        VaultOptions {
            panel: Some(panel.clone()),
            ..VaultOptions::default()
        },
    )
    .unwrap();
    persist_vault_panel_state(&vault, &panel, &registry).unwrap();
    drop(registry);
    fs::write(&model_state, "2").unwrap();

    let error = match load_vault_panel_state(&vault) {
        Ok(_) => panic!("drifted external runtime unexpectedly opened"),
        Err(error) => error,
    };

    assert_eq!(error.code, "CALYX_LENS_RUNTIME_DRIFT");
    assert!(error.message.contains("drifted"));
    fs::remove_dir_all(vault).unwrap();
}

#[test]
fn vault_batch_limit_update_persists_manifest_backed_registry() {
    let (vault, lens_id) = test_vault_with_batch_lens("happy", Some(1));
    let before_manifest = ManifestStore::open(&vault).load_current().unwrap();
    let before_max = manifest_registry_max_batch(&vault, lens_id);
    println!(
        "before batch-limit happy path: manifest_seq={} registry_ref={:?} max_batch={before_max:?}",
        before_manifest.manifest_seq,
        before_manifest
            .registry_ref
            .as_ref()
            .map(|reference| reference.logical_path.as_str())
    );

    let write = set_vault_registry_batch_limits(
        &vault,
        &[RegistryBatchLimitUpdate {
            lens_id,
            max_batch: 8,
        }],
    )
    .unwrap();
    let after_manifest = ManifestStore::open(&vault).load_current().unwrap();
    let after_state = load_vault_panel_state(&vault).unwrap();
    let after_max = manifest_registry_max_batch(&vault, lens_id);
    println!(
        "after batch-limit happy path: manifest_seq={} registry_ref={:?} max_batch={after_max:?} wrote_manifest={} registry_file_exists={}",
        after_manifest.manifest_seq,
        after_manifest
            .registry_ref
            .as_ref()
            .map(|reference| reference.logical_path.as_str()),
        write.wrote_manifest,
        vault.join(&write.registry_ref.logical_path).is_file()
    );

    assert!(write.wrote_manifest);
    assert_eq!(write.changes.len(), 1);
    assert_eq!(write.changes[0].before, Some(1));
    assert_eq!(write.changes[0].after, 8);
    assert!(write.changes[0].changed);
    assert_ne!(before_manifest.registry_ref, after_manifest.registry_ref);
    assert_eq!(before_max, Some(1));
    assert_eq!(after_max, Some(8));
    assert_eq!(
        after_state
            .registry
            .lens_spec(lens_id)
            .and_then(|spec| spec.max_batch),
        Some(8)
    );
}

#[test]
fn vault_batch_limit_update_rejects_empty_without_manifest_write() {
    let (vault, lens_id) = test_vault_with_batch_lens("empty", Some(1));
    let before_manifest = ManifestStore::open(&vault).load_current().unwrap();
    let before_max = manifest_registry_max_batch(&vault, lens_id);
    println!(
        "before empty edge: manifest_seq={} registry_ref={:?} max_batch={before_max:?}",
        before_manifest.manifest_seq,
        before_manifest
            .registry_ref
            .as_ref()
            .map(|reference| reference.logical_path.as_str())
    );

    let error = set_vault_registry_batch_limits(&vault, &[]).unwrap_err();
    let after_manifest = ManifestStore::open(&vault).load_current().unwrap();
    let after_max = manifest_registry_max_batch(&vault, lens_id);
    println!(
        "after empty edge: error_code={} manifest_seq={} registry_ref={:?} max_batch={after_max:?}",
        error.code,
        after_manifest.manifest_seq,
        after_manifest
            .registry_ref
            .as_ref()
            .map(|reference| reference.logical_path.as_str())
    );

    assert_eq!(error.code, "CALYX_REGISTRY_BATCH_LIMIT_INVALID");
    assert_eq!(before_manifest.manifest_seq, after_manifest.manifest_seq);
    assert_eq!(before_manifest.registry_ref, after_manifest.registry_ref);
    assert_eq!(before_max, after_max);
}

#[test]
fn vault_batch_limit_update_rejects_zero_without_manifest_write() {
    let (vault, lens_id) = test_vault_with_batch_lens("zero", Some(1));
    let before_manifest = ManifestStore::open(&vault).load_current().unwrap();
    let before_max = manifest_registry_max_batch(&vault, lens_id);
    println!(
        "before zero edge: manifest_seq={} registry_ref={:?} max_batch={before_max:?}",
        before_manifest.manifest_seq,
        before_manifest
            .registry_ref
            .as_ref()
            .map(|reference| reference.logical_path.as_str())
    );

    let error = set_vault_registry_batch_limits(
        &vault,
        &[RegistryBatchLimitUpdate {
            lens_id,
            max_batch: 0,
        }],
    )
    .unwrap_err();
    let after_manifest = ManifestStore::open(&vault).load_current().unwrap();
    let after_max = manifest_registry_max_batch(&vault, lens_id);
    println!(
        "after zero edge: error_code={} manifest_seq={} registry_ref={:?} max_batch={after_max:?}",
        error.code,
        after_manifest.manifest_seq,
        after_manifest
            .registry_ref
            .as_ref()
            .map(|reference| reference.logical_path.as_str())
    );

    assert_eq!(error.code, "CALYX_REGISTRY_BATCH_LIMIT_INVALID");
    assert_eq!(before_manifest.manifest_seq, after_manifest.manifest_seq);
    assert_eq!(before_manifest.registry_ref, after_manifest.registry_ref);
    assert_eq!(before_max, after_max);
}

#[test]
fn vault_batch_limit_update_rejects_missing_lens_without_manifest_write() {
    let (vault, lens_id) = test_vault_with_batch_lens("missing", Some(1));
    let missing = calyx_core::LensId::from_bytes([0xA5; 16]);
    let before_manifest = ManifestStore::open(&vault).load_current().unwrap();
    let before_max = manifest_registry_max_batch(&vault, lens_id);
    println!(
        "before missing edge: manifest_seq={} registry_ref={:?} existing_max_batch={before_max:?}",
        before_manifest.manifest_seq,
        before_manifest
            .registry_ref
            .as_ref()
            .map(|reference| reference.logical_path.as_str())
    );

    let error = set_vault_registry_batch_limits(
        &vault,
        &[RegistryBatchLimitUpdate {
            lens_id: missing,
            max_batch: 8,
        }],
    )
    .unwrap_err();
    let after_manifest = ManifestStore::open(&vault).load_current().unwrap();
    let after_max = manifest_registry_max_batch(&vault, lens_id);
    println!(
        "after missing edge: error_code={} manifest_seq={} registry_ref={:?} existing_max_batch={after_max:?}",
        error.code,
        after_manifest.manifest_seq,
        after_manifest
            .registry_ref
            .as_ref()
            .map(|reference| reference.logical_path.as_str())
    );

    assert_eq!(error.code, "CALYX_LENS_UNREACHABLE");
    assert_eq!(before_manifest.manifest_seq, after_manifest.manifest_seq);
    assert_eq!(before_manifest.registry_ref, after_manifest.registry_ref);
    assert_eq!(before_max, after_max);
}

fn runtime_spec(contract: &FrozenLensContract, runtime: LensRuntime) -> LensSpec {
    LensSpec {
        name: contract.name().to_string(),
        runtime,
        output: contract.shape(),
        modality: contract.modality(),
        weights_sha256: contract.weights_sha256(),
        corpus_hash: contract.corpus_hash(),
        norm_policy: contract.norm_policy(),
        max_batch: None,
        axis: None,
        asymmetry: Asymmetry::None,
        quant_default: QuantPolicy::turboquant_default(),
        truncate_dim: None,
        recall_delta: crate::spec::default_recall_delta(),
        retrieval_only: false,
        excluded_from_dedup: false,
    }
}

fn external_contract(name: &str, command: &str, args: &[String], dim: u32) -> FrozenLensContract {
    let args_text = args.join("\0");
    FrozenLensContract::new(
        name,
        sha256_digest(&[command.as_bytes(), args_text.as_bytes()]),
        sha256_digest(&[b"external-cmd-runtime-v1"]),
        SlotShape::Dense(dim),
        Modality::Text,
        LensDType::F32,
        NormPolicy::None,
    )
}

fn algorithmic_snapshot(max_batch: Option<usize>) -> RegistryLensSnapshot {
    let lens = AlgorithmicLens::byte_features("issue999-byte", Modality::Text);
    let contract = lens.contract().clone();
    let spec = LensSpec {
        name: contract.name().to_string(),
        runtime: LensRuntime::Algorithmic {
            kind: "byte-features".to_string(),
        },
        output: contract.shape(),
        modality: contract.modality(),
        weights_sha256: contract.weights_sha256(),
        corpus_hash: contract.corpus_hash(),
        norm_policy: contract.norm_policy(),
        max_batch,
        axis: None,
        asymmetry: Asymmetry::None,
        quant_default: QuantPolicy::turboquant_default(),
        truncate_dim: None,
        recall_delta: crate::spec::default_recall_delta(),
        retrieval_only: false,
        excluded_from_dedup: false,
    };
    RegistryLensSnapshot {
        lens_id: contract.lens_id(),
        contract,
        spec: Some(spec),
        determinism: DeterminismProof::ContractOnlyExemption,
        runtime_golden: None,
    }
}

fn text_inputs<const N: usize>(values: [&str; N]) -> Vec<Input> {
    values
        .into_iter()
        .map(|value| Input::new(Modality::Text, value.as_bytes().to_vec()))
        .collect()
}

fn test_vault_with_batch_lens(
    name: &str,
    max_batch: Option<usize>,
) -> (PathBuf, calyx_core::LensId) {
    let vault = temp_vault_dir(name);
    let vault_id: VaultId = "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap();
    let mut registry = Registry::new();
    let lens = AlgorithmicLens::byte_features("batch-limit-lens", Modality::Text);
    let contract = lens.contract().clone();
    let lens_id = contract.lens_id();
    let spec = LensSpec {
        name: contract.name().to_string(),
        runtime: LensRuntime::Algorithmic {
            kind: "byte-features".to_string(),
        },
        output: contract.shape(),
        modality: contract.modality(),
        weights_sha256: contract.weights_sha256(),
        corpus_hash: contract.corpus_hash(),
        norm_policy: contract.norm_policy(),
        max_batch,
        axis: None,
        asymmetry: Asymmetry::None,
        quant_default: QuantPolicy::turboquant_default(),
        truncate_dim: None,
        recall_delta: crate::spec::default_recall_delta(),
        retrieval_only: false,
        excluded_from_dedup: false,
    };
    registry
        .register_frozen_with_spec(lens, contract, spec)
        .unwrap();
    let panel = panel_with_lens(lens_id);
    AsterVault::new_durable(
        &vault,
        vault_id,
        [0x5A; 32],
        VaultOptions {
            panel: Some(panel.clone()),
            ..VaultOptions::default()
        },
    )
    .unwrap();
    persist_vault_panel_state(&vault, &panel, &registry).unwrap();
    (vault, lens_id)
}

fn panel_with_lens(lens_id: calyx_core::LensId) -> Panel {
    panel_with_runtime_lens(lens_id, 16, "batch-limit-lens")
}

fn panel_with_runtime_lens(lens_id: calyx_core::LensId, dim: u32, name: &str) -> Panel {
    let slot = SlotId::new(0);
    Panel {
        version: 1,
        slots: vec![Slot {
            slot_id: slot,
            slot_key: SlotKey::new(slot, name),
            lens_id,
            shape: SlotShape::Dense(dim),
            modality: Modality::Text,
            asymmetry: Asymmetry::None,
            quant: QuantPolicy::None,
            resource: Default::default(),
            axis: Some(name.to_string()),
            retrieval_only: false,
            excluded_from_dedup: false,
            bits_about: BTreeMap::new(),
            state: SlotState::Active,
            added_at_panel_version: 1,
        }],
        created_at: 1,
        kernel_ref: None,
        guard_ref: None,
    }
}

fn manifest_registry_max_batch(vault: &Path, lens_id: calyx_core::LensId) -> Option<usize> {
    let manifest = ManifestStore::open(vault).load_current().unwrap();
    let registry_ref = manifest.registry_ref.as_ref().unwrap();
    let bytes = fs::read(vault.join(&registry_ref.logical_path)).unwrap();
    let snapshot: VaultRegistrySnapshot = serde_json::from_slice(&bytes).unwrap();
    snapshot
        .lenses
        .iter()
        .find(|lens| lens.lens_id == lens_id)
        .and_then(|lens| lens.spec.as_ref())
        .and_then(|spec| spec.max_batch)
}

fn temp_vault_dir(name: &str) -> PathBuf {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "calyx-registry-batch-limit-{name}-{}-{now}",
        std::process::id()
    ))
}
