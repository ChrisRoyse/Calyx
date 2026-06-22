mod support;

use calyx_aster::manifest::{ManifestStore, is_quarantined};
use calyx_aster::sst::{SstEntry, SstReader, write_sst};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Constellation, CxFlags, CxId, FixedClock, InputRef, LedgerRef, Modality, SlotId, SlotVector,
    VaultId, VaultStore,
};
use calyx_ledger::{ActorId, DirectoryLedgerStore, EntryKind, LedgerAppender, SubjectId};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use support::fsv_io::{fsv_root, list_files, named_temp_root, reset_dir, write_json};

#[test]
fn verify_chain_ledger_reports_intact_and_broken_seq() {
    let dir = test_dir("ledger");
    reset_dir(&dir);
    let ledger_dir = dir.join("ledger-cf");
    write_standalone_ledger(&ledger_dir, 3);

    let intact = run(
        ["verify-chain", "--ledger"],
        &ledger_dir,
        ["--range", "0..3"],
    );
    assert_success(&intact);
    assert_eq!(stdout(&intact), "CHAIN_INTACT count=3");

    flip_file_byte(&ledger_dir.join("0000000000000001.ledger"));
    let broken = run(
        ["verify-chain", "--ledger"],
        &ledger_dir,
        ["--range", "0..3"],
    );

    assert!(!broken.status.success());
    assert_stderr_code_and_message(&broken, "CALYX_LEDGER_CHAIN_BROKEN", "seq=1");
    cleanup(dir);
}

#[test]
fn verify_chain_vault_quarantines_broken_range() {
    let dir = test_dir("vault");
    reset_dir(&dir);
    let vault = write_vault(&dir, 3);
    vault.flush().expect("flush");
    remove_wal_segments(&dir);
    tamper_ledger_ssts(&dir, 1);

    let broken = run(["verify-chain", "--vault"], &dir, ["--range", "0..3"]);
    assert!(!broken.status.success());
    assert_stderr_code_and_message(&broken, "CALYX_LEDGER_CHAIN_BROKEN", "seq=1");

    let manifest = ManifestStore::open(&dir).load_current().unwrap();
    assert!(is_quarantined(&manifest, 2));
    let readback = run_readback_ledger_seq(&dir, 2);
    assert!(!readback.status.success());
    assert_stderr_code_and_message(&readback, "CALYX_LEDGER_CHAIN_BROKEN", "quarantined");
    cleanup(dir);
}

#[test]
#[ignore = "manual FSV for issue #250 verify-chain quarantine"]
fn ph36_verify_chain_quarantine_manual_fsv() {
    let root =
        fsv_root("CALYX_FSV_ROOT", "calyx-ph36-verify-chain-fsv").join("verify-chain-quarantine");
    reset_dir(&root);
    let vault_dir = root.join("vault");
    reset_dir(&vault_dir);
    let vault = write_vault(&vault_dir, 20);
    vault.flush().expect("flush");
    remove_wal_segments(&vault_dir);
    let before_manifest = ManifestStore::open(&vault_dir).load_current().unwrap();
    let tampered_files = tamper_ledger_ssts(&vault_dir, 7);

    let verify = run(
        ["verify-chain", "--vault"],
        &vault_dir,
        ["--range", "0..20"],
    );
    let readback = run_readback_ledger_seq(&vault_dir, 8);
    let after_manifest = ManifestStore::open(&vault_dir).load_current().unwrap();
    let intact_empty = run(["verify-chain", "--vault"], &vault_dir, ["--range", "0..0"]);

    let readback_json = serde_json::json!({
        "vault": vault_dir,
        "before_quarantine_count": before_manifest.quarantines.len(),
        "after_quarantine_count": after_manifest.quarantines.len(),
        "quarantine": after_manifest.quarantines.last(),
        "tampered_files": tampered_files,
        "ledger_sst_files": list_files(&vault_dir.join("cf").join("ledger")),
        "wal_segment_files_after_removal": list_files(&vault_dir.join("wal")),
        "verify_stderr": stderr(&verify),
        "readback_stderr": stderr(&readback),
        "empty_range_stdout": stdout(&intact_empty),
        "seq_8_quarantined": is_quarantined(&after_manifest, 8),
    });
    let readback_path = root.join("verify-chain-readback.json");
    write_json(&readback_path, &readback_json);

    println!("PH36_VERIFY_CHAIN_FSV_ROOT={}", root.display());
    println!("PH36_VERIFY_CHAIN_READBACK={}", readback_path.display());
    println!("{}", serde_json::to_string_pretty(&readback_json).unwrap());

    assert!(!verify.status.success());
    assert_stderr_code_and_message(&verify, "CALYX_LEDGER_CHAIN_BROKEN", "seq=7");
    assert!(!readback.status.success());
    assert_stderr_code_and_message(&readback, "CALYX_LEDGER_CHAIN_BROKEN", "quarantined");
    assert_eq!(before_manifest.quarantines.len(), 0);
    assert_eq!(after_manifest.quarantines.len(), 1);
    assert!(is_quarantined(&after_manifest, 8));
    assert_success(&intact_empty);
    assert_eq!(stdout(&intact_empty), "CHAIN_INTACT count=0");
}

fn write_standalone_ledger(dir: &Path, count: usize) {
    let store = DirectoryLedgerStore::open(dir).expect("open ledger dir");
    let mut appender = LedgerAppender::open(store, FixedClock::new(10)).expect("open appender");
    for seq in 0..count {
        appender
            .append(
                EntryKind::Ingest,
                SubjectId::Cx(CxId::from_bytes([seq as u8; 16])),
                format!("payload-{seq}").into_bytes(),
                ActorId::Service("verify-cli-test".to_string()),
            )
            .expect("append");
    }
}

fn write_vault(dir: &Path, count: usize) -> AsterVault {
    let vault = AsterVault::new_durable(dir, vault_id(), b"salt".to_vec(), VaultOptions::default())
        .expect("open vault");
    for seed in 0..count {
        vault
            .put(sample_constellation(&vault, seed as u16))
            .expect("put");
    }
    vault
}

fn sample_constellation(vault: &AsterVault, seed: u16) -> Constellation {
    let input = format!("ph36-verify-chain-{seed}");
    let cx_id = vault.cx_id_for_input(input.as_bytes(), 7);
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
        created_at: 1_785_200_000 + u64::from(seed),
        input_ref: InputRef {
            hash: input_hash,
            pointer: Some(format!("synthetic://ph36/verify-chain/{seed}")),
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

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn tamper_ledger_ssts(vault: &Path, seq: u64) -> Vec<String> {
    let key = seq.to_be_bytes();
    let mut touched = Vec::new();
    for file in sst_files(&vault.join("cf").join("ledger")) {
        let reader = SstReader::open(&file).expect("open ledger sst");
        let mut rows = reader.iter().expect("read ledger sst");
        let mut changed = false;
        for row in &mut rows {
            if row.key == key {
                let last = row.value.len() - 1;
                row.value[last] ^= 1;
                changed = true;
            }
        }
        if changed {
            rewrite_sst(&file, &rows);
            touched.push(file.file_name().unwrap().to_string_lossy().to_string());
        }
    }
    touched.sort();
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

fn flip_file_byte(path: &Path) {
    let mut bytes = fs::read(path).expect("read row");
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    fs::write(path, bytes).expect("write row");
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
        .expect("run calyx readback ledger seq")
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

fn assert_stderr_code_and_message(output: &Output, code: &str, message_part: &str) {
    let stderr = stderr(output);
    let parsed: serde_json::Value = serde_json::from_str(&stderr)
        .unwrap_or_else(|error| panic!("stderr JSON: {error}: {stderr}"));

    assert_eq!(parsed["code"], code, "stderr: {stderr}");
    assert!(
        parsed["message"]
            .as_str()
            .is_some_and(|message| message.contains(message_part)),
        "stderr: {stderr}"
    );
}

fn sst_files(dir: &Path) -> Vec<PathBuf> {
    list_files(dir)
        .into_iter()
        .filter(|file| file.ends_with(".sst"))
        .map(|file| dir.join(file))
        .collect()
}

fn test_dir(name: &str) -> PathBuf {
    named_temp_root("calyx-cli-verify-chain", name)
}

fn cleanup(dir: PathBuf) {
    fs::remove_dir_all(dir).unwrap();
}
