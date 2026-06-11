use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use calyx_aster::manifest::{ManifestStore, is_quarantined};
use calyx_aster::sst::{SstEntry, SstReader, write_sst};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Constellation, CxFlags, CxId, InputRef, LedgerRef, Modality, SlotId, SlotVector, VaultStore,
};
use serde_json::{Value, json};

use super::common::{hex, reset_dir, vault_id};

pub fn run_tamper_fsv(root: &Path) -> Value {
    let vault_dir = root.join("tamper-vault");
    reset_dir(&vault_dir);
    let (vault, cx_ids) = write_vault(&vault_dir, 20);
    vault.flush().expect("flush vault");
    remove_wal_segments(&vault_dir);

    let intact = run(
        ["verify-chain", "--vault"],
        &vault_dir,
        ["--range", "0..20"],
    );
    assert_success(&intact);
    let before_manifest = ManifestStore::open(&vault_dir).load_current().unwrap();
    let tampered = tamper_ledger_ssts(&vault_dir, 11, 8);
    let broken = run(
        ["verify-chain", "--vault"],
        &vault_dir,
        ["--range", "0..20"],
    );
    let provenance = run(
        ["get-provenance", "--vault"],
        &vault_dir,
        ["--cx", &cx_ids[11].to_string()],
    );
    let after_manifest = ManifestStore::open(&vault_dir).load_current().unwrap();
    let readback_seq = run_readback_ledger_seq(&vault_dir, 11);

    assert!(!broken.status.success());
    assert!(stderr(&broken).contains("CALYX_LEDGER_CHAIN_BROKEN at seq=11"));
    assert!(!provenance.status.success());
    assert!(stderr(&provenance).contains("CALYX_LEDGER_CHAIN_BROKEN"));
    assert!(is_quarantined(&after_manifest, 11));

    json!({
        "vault": vault_dir,
        "intact_stdout": stdout(&intact),
        "before_quarantine_count": before_manifest.quarantines.len(),
        "after_quarantine_count": after_manifest.quarantines.len(),
        "quarantine": after_manifest.quarantines.last(),
        "tampered": tampered,
        "verify_stderr": stderr(&broken),
        "broken_seq": 11,
        "seq_11_quarantined": is_quarantined(&after_manifest, 11),
        "provenance_stderr": stderr(&provenance),
        "readback_stderr": stderr(&readback_seq),
    })
}

fn write_vault(dir: &Path, count: usize) -> (AsterVault, Vec<CxId>) {
    let vault = AsterVault::new_durable(dir, vault_id(), b"salt".to_vec(), VaultOptions::default())
        .expect("open vault");
    let mut ids = Vec::new();
    for seed in 0..count {
        let id = vault.cx_id_for_input(format!("ph36-fsv-integration-{seed}").as_bytes(), 7);
        ids.push(id);
        vault
            .put(sample_constellation(id, seed as u16))
            .expect("put");
    }
    (vault, ids)
}

fn sample_constellation(cx_id: CxId, seed: u16) -> Constellation {
    let input = format!("ph36-fsv-integration-{seed}");
    let mut input_hash = [0_u8; 32];
    input_hash[..input.len()].copy_from_slice(input.as_bytes());
    let mut slots = BTreeMap::new();
    slots.insert(
        SlotId::new(0),
        SlotVector::Dense {
            dim: 2,
            data: vec![f32::from(seed), 0.5],
        },
    );
    Constellation {
        cx_id,
        vault_id: vault_id(),
        panel_version: 7,
        created_at: 1_785_700_000 + u64::from(seed),
        input_ref: InputRef {
            hash: input_hash,
            pointer: Some(format!("synthetic://ph36/fsv/{seed}")),
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

fn tamper_ledger_ssts(vault: &Path, seq: u64, offset: usize) -> Vec<Value> {
    let key = seq.to_be_bytes();
    let mut touched = Vec::new();
    for file in sst_files(&vault.join("cf").join("ledger")) {
        let reader = SstReader::open(&file).expect("open ledger sst");
        let mut rows = reader.iter().expect("read ledger sst");
        let mut tamper = None;
        for row in &mut rows {
            if row.key == key {
                let before = row.value.clone();
                row.value[offset] ^= 1;
                tamper = Some(json!({
                    "file": file.file_name().unwrap().to_string_lossy(),
                    "seq": seq,
                    "offset": offset,
                    "before_byte": before[offset],
                    "after_byte": row.value[offset],
                    "before_prefix_hex": hex(&before[..before.len().min(32)]),
                    "after_prefix_hex": hex(&row.value[..row.value.len().min(32)]),
                }));
            }
        }
        if let Some(tamper) = tamper {
            rewrite_sst(&file, &rows);
            touched.push(tamper);
        }
    }
    touched
}

fn rewrite_sst(path: &Path, rows: &[SstEntry]) {
    let refs = rows
        .iter()
        .map(|row| (row.key.as_slice(), row.value.as_slice()));
    write_sst(path, refs).expect("rewrite tampered sst");
}

fn remove_wal_segments(vault: &Path) {
    for file in list_files(&vault.join("wal")) {
        if file.ends_with(".wal") {
            fs::remove_file(vault.join("wal").join(file)).unwrap();
        }
    }
}

fn run<const A: usize, const B: usize>(
    prefix: [&str; A],
    path: &Path,
    suffix: [&str; B],
) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_calyx"));
    for arg in prefix {
        command.arg(arg);
    }
    command.arg(path);
    for arg in suffix {
        command.arg(arg);
    }
    command.output().expect("run calyx")
}

fn run_readback_ledger_seq(vault: &Path, seq: u64) -> Output {
    Command::new(env!("CARGO_BIN_EXE_calyx"))
        .arg("readback")
        .arg("--cf")
        .arg("ledger")
        .arg("--vault")
        .arg(vault)
        .arg("--seq")
        .arg(seq.to_string())
        .output()
        .expect("run readback")
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

fn sst_files(dir: &Path) -> Vec<PathBuf> {
    list_files(dir)
        .into_iter()
        .filter(|file| file.ends_with(".sst"))
        .map(|file| dir.join(file))
        .collect()
}

fn list_files(dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files = entries
        .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
        .collect::<Vec<_>>();
    files.sort();
    files
}
