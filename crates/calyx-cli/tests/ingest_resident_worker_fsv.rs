use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Asymmetry, Modality, Panel, Placement, QuantPolicy, Slot, SlotId, SlotKey, SlotResource,
    SlotShape, SlotState, VaultId, VaultStore,
};
use calyx_registry::{AlgorithmicLens, LensRuntime, LensSpec, Registry, persist_vault_panel_state};
use serde_json::Value;
use ulid::Ulid;

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

#[test]
fn gpu_placed_persisted_lens_uses_resident_binary_worker_and_persists_cfs() {
    let root = temp_root("resident-worker-fsv");
    let vault_id = VaultId::from_ulid(Ulid::new());
    let vault_path = root.join("vaults").join(vault_id.to_string());
    println!("resident_worker_fsv_root={}", root.display());
    println!("resident_worker_fsv_vault={}", vault_path.display());
    let salt = b"resident-worker-fsv-salt".to_vec();
    let (panel, registry) = gpu_algorithmic_panel();
    AsterVault::new_durable(
        &vault_path,
        vault_id,
        salt.clone(),
        VaultOptions {
            panel: Some(panel.clone()),
            ..VaultOptions::default()
        },
    )
    .expect("create durable resident-worker FSV vault");
    persist_vault_panel_state(&vault_path, &panel, &registry)
        .expect("persist registry snapshot for resident-worker FSV");

    let before = cf_state(&vault_path, vault_id, salt.clone());
    println!(
        "resident_worker_fsv_before source_of_truth=Aster CF readback state={}",
        before
    );
    let batch = vault_path.join("resident-worker.jsonl");
    fs::write(
        &batch,
        "{\"text\":\"resident worker process FSV alpha\"}\n{\"text\":\"resident worker process FSV beta\"}\n",
    )
    .expect("write resident-worker batch");

    let output = Command::new(calyx_exe())
        .env("CALYX_HOME", &root)
        .arg("ingest")
        .arg(&vault_path)
        .arg("--batch")
        .arg(&batch)
        .arg("--idempotent")
        .output()
        .expect("run calyx ingest resident-worker FSV");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    println!("resident_worker_fsv_stdout={stdout}");
    println!("resident_worker_fsv_stderr={stderr}");
    let summary_result = serde_json::from_str::<Value>(stdout.trim());
    let after = cf_state(&vault_path, vault_id, salt);
    let readback = serde_json::json!({
        "issue": 1046,
        "source_of_truth": {
            "vault_path": vault_path.clone(),
            "batch_path": batch.clone(),
            "cf_readback": "Aster Base/Ledger/slot_00 column families opened after ingest flush",
            "resident_protocol": "binary ready/response frames observed by the parent worker loop",
        },
        "command": {
            "status_code": output.status.code(),
            "success": output.status.success(),
            "stdout": stdout.to_string(),
            "stderr": stderr.to_string(),
            "summary_parse_ok": summary_result.is_ok(),
            "summary_parse_error": summary_result.as_ref().err().map(ToString::to_string),
            "protocol_markers": {
                "spawned": stderr.contains("phase=measure_lens_worker_resident_spawned"),
                "ready_parent_observed": stderr.contains("ready_frame_bytes="),
                "response_parent_observed": stderr.contains("observed_by=parent"),
                "resident_ok": stderr.contains("phase=measure_lens_worker_resident_ok"),
                "runtime_load_zero": stderr.contains("runtime_load_ms=0"),
                "old_one_shot_absent": !stderr.contains("phase=measure_lens_worker_spawned lens_id="),
            },
        },
        "before": before.clone(),
        "after": after.clone(),
    });
    let readback_path = write_fsv_readback(vault_id, &readback);
    println!("resident_worker_fsv_readback={}", readback_path.display());
    assert!(
        output.status.success(),
        "resident worker ingest failed: status={:?} stdout={stdout} stderr={stderr}",
        output.status
    );
    let summary = summary_result.expect("parse ingest summary");
    println!(
        "resident_worker_fsv_after source_of_truth=Aster CF readback state={}",
        after
    );

    assert_eq!(summary["status"], "ingested");
    assert_eq!(summary["row_count"], 2);
    assert_eq!(summary["new_count"], 2);
    assert_eq!(summary["already_count"], 0);
    assert_eq!(summary["verified_base_rows"], 2);
    assert_eq!(before["base_rows"], 0);
    assert_eq!(before["slot_00_rows"], 0);
    assert_eq!(before["ledger_rows"], 0);
    assert_eq!(after["base_rows"], 2);
    assert_eq!(after["slot_00_rows"], 2);
    assert_eq!(after["ledger_rows"], 1);
    assert!(stderr.contains("phase=measure_lens_worker_resident_spawned"));
    assert!(stderr.contains("phase=measure_lens_worker_resident_child_ready"));
    assert!(stderr.contains("ready_frame_bytes="));
    assert!(stderr.contains("phase=measure_lens_worker_resident_child_response"));
    assert!(stderr.contains("observed_by=parent"));
    assert!(stderr.contains("phase=measure_lens_worker_resident_ok"));
    assert!(stderr.contains("runtime_load_ms=0"));
    assert!(
        !stderr.contains("phase=measure_lens_worker_spawned lens_id="),
        "old one-shot process churn log must not appear: {stderr}"
    );

    if std::env::var("CALYX_KEEP_RESIDENT_WORKER_FSV_ROOT").as_deref() == Ok("1") {
        println!("resident_worker_fsv_preserved_root={}", root.display());
    } else {
        fs::remove_dir_all(root).ok();
    }
}

fn gpu_algorithmic_panel() -> (Panel, Registry) {
    let lens = AlgorithmicLens::byte_features("resident-worker-fsv", Modality::Text);
    let contract = lens.contract().clone();
    let lens_id = contract.lens_id();
    let mut registry = Registry::new();
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
        max_batch: Some(4),
        axis: Some("resident_worker".to_string()),
        asymmetry: Asymmetry::None,
        quant_default: QuantPolicy::None,
        truncate_dim: None,
        recall_delta: calyx_registry::spec::default_recall_delta(),
        retrieval_only: false,
        excluded_from_dedup: false,
    };
    registry
        .register_frozen_with_spec(lens, contract, spec)
        .expect("register algorithmic lens with reconstructable LensSpec");
    let slot = SlotId::new(0);
    let panel = Panel {
        version: 1,
        slots: vec![Slot {
            slot_id: slot,
            slot_key: SlotKey::new(slot, "resident_worker"),
            lens_id,
            shape: SlotShape::Dense(16),
            modality: Modality::Text,
            asymmetry: Asymmetry::None,
            quant: QuantPolicy::None,
            resource: SlotResource {
                placement: Placement::Gpu,
                ..SlotResource::default()
            },
            axis: Some("resident_worker".to_string()),
            retrieval_only: false,
            excluded_from_dedup: false,
            bits_about: BTreeMap::new(),
            state: SlotState::Active,
            added_at_panel_version: 1,
        }],
        created_at: 1,
        kernel_ref: None,
        guard_ref: None,
    };
    (panel, registry)
}

fn cf_state(vault_path: &Path, vault_id: VaultId, salt: Vec<u8>) -> Value {
    let vault = AsterVault::new_durable(vault_path, vault_id, salt, VaultOptions::default())
        .expect("open durable resident-worker FSV vault");
    let snapshot = vault.snapshot();
    serde_json::json!({
        "snapshot": snapshot,
        "base_rows": vault.scan_cf_at(snapshot, ColumnFamily::Base).expect("scan Base CF").len(),
        "ledger_rows": vault.scan_cf_at(snapshot, ColumnFamily::Ledger).expect("scan Ledger CF").len(),
        "slot_00_rows": vault.scan_cf_at(snapshot, ColumnFamily::slot(SlotId::new(0))).expect("scan slot_00 CF").len(),
    })
}

fn calyx_exe() -> PathBuf {
    std::env::var_os("CARGO_BIN_EXE_calyx")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/calyx.exe")
        })
}

fn write_fsv_readback(vault_id: VaultId, readback: &Value) -> PathBuf {
    let root = calyx_fsv::fsv_root_or_else("CALYX_FSV_ROOT", || {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/fsv/resident-worker-fsv")
    });
    fs::create_dir_all(&root).expect("create resident-worker FSV evidence root");
    let path = root.join(format!("resident-worker-readback-{vault_id}.json"));
    fs::write(
        &path,
        serde_json::to_vec_pretty(readback).expect("serialize resident-worker FSV readback"),
    )
    .expect("write resident-worker FSV readback");
    path
}

fn temp_root(name: &str) -> PathBuf {
    let id = NEXT_DIR.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!("calyx-cli-{name}-{}-{id}", std::process::id()))
}
