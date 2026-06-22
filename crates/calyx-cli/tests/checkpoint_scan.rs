mod support;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::encode::decode_write_batch;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Constellation, CxFlags, InputRef, LedgerRef, Modality, SlotId, SlotVector, VaultId, VaultStore,
};
use calyx_ledger::CheckpointConfig;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use support::fsv_io::{fsv_root, list_files, named_temp_root, reset_dir, write_json};

#[test]
fn scan_ledger_vault_prints_checkpoint_admin_json() {
    let dir = test_dir("scan");
    reset_dir(&dir);
    let vault = write_vault(&dir, 5, CheckpointConfig::new(2));
    vault.flush().expect("flush");

    let scan = run_scan(&dir);
    assert_success(&scan);
    let admins = admin_rows(&scan);

    assert_eq!(admins.len(), 2);
    assert_eq!(admins[0]["payload"]["tag"], "checkpoint_v1");
    assert_eq!(admins[0]["payload"]["range_start"], 0);
    assert_eq!(admins[0]["payload"]["range_end"], 2);
    assert_eq!(
        admins[0]["payload"]["root"].as_str().unwrap(),
        stdout(&run_merkle_root(&dir, "0..2"))
    );
    cleanup(dir);
}

#[test]
#[ignore = "manual FSV for issue #251 checkpoint scheduler"]
fn ph36_checkpoint_scheduler_manual_fsv() {
    let root = fsv_root("CALYX_FSV_ROOT", "calyx-ph36-checkpoint-fsv").join("checkpoint-scheduler");
    reset_dir(&root);
    let vault_dir = root.join("vault");
    reset_dir(&vault_dir);
    let before_scan = run_scan(&vault_dir);
    let vault = write_vault(
        &vault_dir,
        9,
        CheckpointConfig::new(3).with_sign_key([42; 32]),
    );
    vault.flush().expect("flush");

    let scan = run_scan(&vault_dir);
    assert_success(&scan);
    let admins = admin_rows(&scan);
    let root_matches = admins
        .iter()
        .map(|row| {
            let start = row["payload"]["range_start"].as_u64().unwrap();
            let end = row["payload"]["range_end"].as_u64().unwrap();
            let direct = stdout(&run_merkle_root(&vault_dir, &format!("{start}..{end}")));
            row["payload"]["root"].as_str().unwrap() == direct
        })
        .collect::<Vec<_>>();
    let wal = inspect_wal(&vault_dir);
    let readback = serde_json::json!({
        "vault": vault_dir,
        "before_scan_status": before_scan.status.code(),
        "before_scan_stderr": stderr(&before_scan),
        "scan_stdout": stdout(&scan),
        "admin_checkpoint_count": admins.len(),
        "admin_checkpoints": admins,
        "roots_match_direct_merkle": root_matches,
        "wal": wal,
        "ledger_sst_files": list_files(&vault_dir.join("cf").join("ledger")),
        "wal_segment_files": list_files(&vault_dir.join("wal")),
    });
    let readback_path = root.join("checkpoint-readback.json");
    write_json(&readback_path, &readback);

    println!("PH36_CHECKPOINT_FSV_ROOT={}", root.display());
    println!("PH36_CHECKPOINT_READBACK={}", readback_path.display());
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    assert_eq!(admins.len(), 3);
    assert!(root_matches.iter().all(|matched| *matched));
    assert_eq!(wal["records_with_two_ledger_rows"], 3);
    assert_eq!(wal["ledger_rows_before_base"], 9);
    assert!(
        admins
            .iter()
            .all(|row| row["payload"]["signature"].is_string())
    );
    assert!(
        admins
            .iter()
            .all(|row| row["payload"]["signer_pubkey"].is_string())
    );
}

fn write_vault(dir: &Path, count: usize, checkpoint: CheckpointConfig) -> AsterVault {
    let options = VaultOptions {
        ledger_checkpoint: Some(checkpoint),
        ..VaultOptions::default()
    };
    let vault =
        AsterVault::new_durable(dir, vault_id(), b"salt".to_vec(), options).expect("open vault");
    for seed in 0..count {
        vault
            .put(sample_constellation(&vault, seed as u16))
            .expect("put");
    }
    vault
}

fn sample_constellation(vault: &AsterVault, seed: u16) -> Constellation {
    let input = format!("ph36-checkpoint-scan-{seed}");
    let cx_id = vault.cx_id_for_input(input.as_bytes(), 7);
    let mut input_hash = [0_u8; 32];
    input_hash[..input.len()].copy_from_slice(input.as_bytes());
    let mut slots = BTreeMap::new();
    slots.insert(
        SlotId::new(0),
        SlotVector::Dense {
            dim: 2,
            data: vec![f32::from(seed), 0.75],
        },
    );
    Constellation {
        cx_id,
        vault_id: vault_id(),
        panel_version: 7,
        created_at: 1_785_400_000 + u64::from(seed),
        input_ref: InputRef {
            hash: input_hash,
            pointer: Some(format!("synthetic://ph36/checkpoint-scan/{seed}")),
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 99,
            hash: [9; 32],
        },
        flags: CxFlags::default(),
    }
}

fn admin_rows(output: &Output) -> Vec<Value> {
    stdout(output)
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("json line"))
        .filter(|row| row["kind"] == "Admin")
        .collect()
}

fn run_scan(vault: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_calyx"))
        .arg("scan")
        .arg("--cf")
        .arg("ledger")
        .arg("--vault")
        .arg(vault)
        .output()
        .expect("run calyx scan")
}

fn run_merkle_root(vault: &Path, range: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_calyx"))
        .arg("merkle-root")
        .arg("--vault")
        .arg(vault)
        .arg("--range")
        .arg(range)
        .output()
        .expect("run calyx merkle-root")
}

fn inspect_wal(vault: &Path) -> Value {
    let replay = calyx_aster::wal::replay_dir(vault.join("wal")).expect("replay wal");
    let mut records_with_two_ledger_rows = 0;
    let mut ledger_rows_before_base = 0;
    for record in replay.records {
        let rows = decode_write_batch(&record.payload).expect("decode wal batch");
        let ledger_indexes = rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.cf == ColumnFamily::Ledger)
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        let base_index = rows.iter().position(|row| row.cf == ColumnFamily::Base);
        if ledger_indexes.len() == 2 {
            records_with_two_ledger_rows += 1;
        }
        if let (Some(first), Some(base)) = (ledger_indexes.first(), base_index)
            && *first < base
        {
            ledger_rows_before_base += 1;
        }
    }
    serde_json::json!({
        "records_with_two_ledger_rows": records_with_two_ledger_rows,
        "ledger_rows_before_base": ledger_rows_before_base,
    })
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        stdout(output),
        stderr(output)
    );
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).trim().to_string()
}

fn test_dir(name: &str) -> PathBuf {
    named_temp_root("calyx-cli-checkpoint", name)
}

fn cleanup(dir: PathBuf) {
    fs::remove_dir_all(dir).unwrap();
}
