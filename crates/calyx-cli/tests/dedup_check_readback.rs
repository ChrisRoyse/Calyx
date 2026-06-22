use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use calyx_aster::cf::{ColumnFamily, base_key};
use calyx_aster::dedup::{
    CALYX_DEDUP_DPI_EXCEEDED, CALYX_DEDUP_INVALID_TAU, CALYX_DEDUP_NO_REQUIRED_SLOTS, DedupAction,
    DedupPolicy, TauStrategy, TctCosineConfig, check_dedup, check_dedup_with_limit,
};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Constellation, CxFlags, CxId, InputRef, LedgerRef, Modality, SlotId, SlotVector, VaultId,
    VaultStore,
};
use serde_json::{Value, json};

#[path = "support/dedup_fsv_io.rs"]
mod dedup_fsv_io;

use dedup_fsv_io::{
    fsv_root, list_dir_files as list_files, reset_dir, write_blake3_sums, write_json, write_text,
};

const VAULT_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const SALT: &str = "dedup-check-readback-salt";

#[test]
fn dedup_check_readback_cli_matches_near_duplicate_and_distinct() {
    let (root, keep_root) = fsv_root("CALYX_DEDUP_ENGINE_FSV_ROOT", "calyx-dedup-engine-fsv");
    reset_dir(&root);
    let vault_dir = root.join("vault");
    let before = json!({
        "root_exists": root.exists(),
        "vault_current_exists": vault_dir.join("CURRENT").exists(),
        "vault_files": list_files(&vault_dir),
    });
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        SALT.as_bytes().to_vec(),
        VaultOptions::default(),
    )
    .expect("open durable vault");
    let existing = sample_constellation(&vault);
    let existing_id = existing.cx_id;
    vault.put(existing).expect("put existing");
    vault.flush().expect("flush vault");

    let output = run_dedup_check(&vault_dir, existing_id, "0", "0.90", "0.95", "0.85");
    assert_success(&output);
    let stdout_json: Value = serde_json::from_slice(&output.stdout).expect("parse cli json");
    let missing_slot = run_dedup_check(&vault_dir, existing_id, "9", "0.90", "0.95", "0.85");
    let invalid_tau = run_dedup_check(&vault_dir, existing_id, "0", "2.0", "0.95", "0.85");
    let dpi_candidate = candidate_constellation(CxId::from_bytes([0xee; 16]));
    let dpi_error = check_dedup_with_limit(&dpi_candidate, &vault, &dedup_policy(), None, 0)
        .expect_err("dpi exceeded");
    let invalid_calibrated_tau_edges = invalid_calibrated_tau_edges(&vault);
    let bypass_candidate = candidate_constellation(CxId::from_bytes([0xbe; 16]));
    let bypassed_empty_required = DedupPolicy::TctCosine(TctCosineConfig {
        required_slots: Vec::new(),
        tau: TauStrategy::Calibrated,
        action: DedupAction::Collapse,
    });
    let bypassed_error = check_dedup(&bypass_candidate, &vault, &bypassed_empty_required, None)
        .expect_err("bypassed empty required slots");
    let base_bytes = vault
        .read_cf_at(vault.snapshot(), ColumnFamily::Base, &base_key(existing_id))
        .expect("read base")
        .expect("base row");

    let readback = json!({
        "before": before,
        "existing": existing_id,
        "cli_stdout": stdout_json,
        "cli_stderr": stderr(&output),
        "base_cf_hex": hex_bytes(&base_bytes),
        "base_cf_blake3": blake3_hex(&base_bytes),
        "after": {
            "vault_current_exists": vault_dir.join("CURRENT").exists(),
            "vault_files": list_files(&vault_dir),
        },
        "edge_missing_slot": {
            "status_success": missing_slot.status.success(),
            "stderr": stderr(&missing_slot),
        },
        "edge_invalid_tau": {
            "status_success": invalid_tau.status.success(),
            "stderr": stderr(&invalid_tau),
        },
        "edge_dpi_exceeded": {
            "candidate": dpi_candidate.cx_id,
            "candidate_limit": 0,
            "after_error_code": dpi_error.code,
            "expected_error_code": CALYX_DEDUP_DPI_EXCEEDED,
        },
        "edge_invalid_calibrated_tau": invalid_calibrated_tau_edges.clone(),
        "edge_bypassed_empty_required_slots": {
            "candidate": bypass_candidate.cx_id,
            "required_slots": [],
            "after_error_code": bypassed_error.code,
            "expected_error_code": CALYX_DEDUP_NO_REQUIRED_SLOTS,
        }
    });
    write_json(&root.join("dedup-check-readback.json"), &readback);
    write_text(&root.join("dedup-check-cli-stdout.json"), stdout(&output));
    write_text(&root.join("dedup-check-cli-stderr.txt"), stderr(&output));
    write_text(&root.join("base-cf.hex"), hex_bytes(&base_bytes));
    write_blake3_sums(&root);

    assert_close(stdout_json["near"]["target_cos"].as_f64().unwrap(), 0.95);
    assert_eq!(
        stdout_json["near"]["decision"]["Match"]["existing"],
        json!(existing_id)
    );
    assert_close(
        stdout_json["distinct"]["target_cos"].as_f64().unwrap(),
        0.85,
    );
    assert_eq!(stdout_json["distinct"]["decision"], json!("NoMatch"));
    assert!(!missing_slot.status.success());
    assert!(stderr(&missing_slot).contains("no dense vector"));
    assert!(!invalid_tau.status.success());
    assert!(stderr(&invalid_tau).contains("--tau"));
    assert_eq!(dpi_error.code, CALYX_DEDUP_DPI_EXCEEDED);
    assert_eq!(bypassed_error.code, CALYX_DEDUP_NO_REQUIRED_SLOTS);
    for edge in invalid_calibrated_tau_edges {
        assert_eq!(edge["after_error_code"], json!(CALYX_DEDUP_INVALID_TAU));
    }

    println!("dedup_check_readback_fsv_root={}", root.display());
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    if !keep_root {
        fs::remove_dir_all(root).expect("cleanup temp root");
    }
}

fn invalid_calibrated_tau_edges(vault: &AsterVault) -> Vec<Value> {
    [
        ("nan", f32::NAN),
        ("pos_inf", f32::INFINITY),
        ("neg_inf", f32::NEG_INFINITY),
        ("above_one", 1.01),
        ("below_neg_one", -1.01),
    ]
    .into_iter()
    .map(|(label, tau)| {
        let mut profile = BTreeMap::new();
        profile.insert(SlotId::new(0), tau);
        let policy = DedupPolicy::TctCosine(
            TctCosineConfig::new(
                vec![SlotId::new(0)],
                TauStrategy::Calibrated,
                DedupAction::Collapse,
            )
            .expect("policy"),
        );
        let candidate = candidate_constellation(CxId::from_bytes([0xca; 16]));
        let error =
            check_dedup(&candidate, vault, &policy, Some(&profile)).expect_err("invalid tau");
        json!({
            "label": label,
            "profile_tau": label,
            "after_error_code": error.code,
            "expected_error_code": CALYX_DEDUP_INVALID_TAU,
        })
    })
    .collect()
}

fn dedup_policy() -> DedupPolicy {
    DedupPolicy::TctCosine(
        TctCosineConfig::new(
            vec![SlotId::new(0)],
            TauStrategy::PerSlot(vec![(SlotId::new(0), 0.90)]),
            DedupAction::Collapse,
        )
        .expect("policy"),
    )
}

fn run_dedup_check(
    vault_dir: &Path,
    cx_id: calyx_core::CxId,
    slot: &str,
    tau: &str,
    near_cos: &str,
    distinct_cos: &str,
) -> Output {
    Command::new(env!("CARGO_BIN_EXE_calyx"))
        .arg("readback")
        .arg("dedup-check")
        .arg("--vault")
        .arg(vault_dir)
        .arg("--cx-id")
        .arg(cx_id.to_string())
        .arg("--slot")
        .arg(slot)
        .arg("--tau")
        .arg(tau)
        .arg("--near-cos")
        .arg(near_cos)
        .arg("--distinct-cos")
        .arg(distinct_cos)
        .arg("--vault-id")
        .arg(VAULT_ID)
        .arg("--salt")
        .arg(SALT)
        .output()
        .expect("run calyx readback dedup-check")
}

fn sample_constellation(vault: &AsterVault) -> Constellation {
    let input = b"dedup check readback source";
    let cx_id = vault.cx_id_for_input(input, 41);
    let mut input_hash = [0_u8; 32];
    input_hash[..input.len()].copy_from_slice(input);
    let mut slots = BTreeMap::new();
    slots.insert(
        SlotId::new(0),
        SlotVector::Dense {
            dim: 2,
            data: vec![1.0, 0.0],
        },
    );
    Constellation {
        cx_id,
        vault_id: vault_id(),
        panel_version: 41,
        created_at: 1_786_406_500,
        input_ref: InputRef {
            hash: input_hash,
            pointer: Some("synthetic://ph41/dedup-check-source".to_string()),
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 0,
            hash: [0; 32],
        },
        flags: CxFlags::default(),
    }
}

fn candidate_constellation(cx_id: CxId) -> Constellation {
    let mut slots = BTreeMap::new();
    slots.insert(
        SlotId::new(0),
        SlotVector::Dense {
            dim: 2,
            data: vec![1.0, 0.0],
        },
    );
    Constellation {
        cx_id,
        vault_id: vault_id(),
        panel_version: 41,
        created_at: 1_786_406_501,
        input_ref: InputRef {
            hash: [0xee; 32],
            pointer: Some("synthetic://ph41/dedup-dpi-candidate".to_string()),
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 0,
            hash: [0; 32],
        },
        flags: CxFlags::default(),
    }
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        stdout(output),
        stderr(output)
    );
}

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 1.0e-6,
        "actual={actual} expected={expected}"
    );
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).trim().to_string()
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn blake3_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn vault_id() -> VaultId {
    VAULT_ID.parse().expect("valid vault id")
}
