#[cfg(unix)]
use std::fs;

use super::super::runtime::load_runtime_lens;
use super::*;
#[cfg(unix)]
use crate::ExternalCmdLens;
#[cfg(unix)]
use crate::frozen::{LensDType, NormPolicy, sha256_digest};
use crate::{FrozenLensContract, RuntimeGolden};

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

    let error = match load_runtime_lens(&snapshot) {
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

    let error = match load_runtime_lens(&snapshot) {
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

#[cfg(unix)]
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
