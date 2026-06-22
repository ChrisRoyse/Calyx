mod support;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use calyx_aster::manifest::{ManifestStore, is_quarantined};
use calyx_aster::sst::{SstEntry, SstReader, write_sst};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Constellation, CxFlags, InputRef, LedgerRef, Modality, SlotId, SlotVector, VaultId, VaultStore,
};
use serde_json::{Value, json};
use support::fsv_io::{list_files, named_temp_root, reset_dir, write_blake3_sums, write_json};

#[test]
fn verify_chain_vault_quarantines_missing_physical_row() {
    let root = test_dir("missing-row");
    let case = run_case(&root, "missing", Tamper::RemoveRow(1));

    assert_ne!(case["verify_status"], 0);
    assert_stderr_code_and_message(&case, "verify_stderr", "CALYX_LEDGER_CORRUPT", "seq=1");
    assert_eq!(case["after_quarantine_count"], 1);
    assert_eq!(case["seq_1_quarantined"], true);
    assert_stderr_code_and_message(
        &case,
        "readback_stderr",
        "CALYX_LEDGER_CHAIN_BROKEN",
        "quarantined",
    );
    cleanup(root);
}

#[test]
fn verify_chain_vault_quarantines_corrupt_physical_payload() {
    let root = test_dir("corrupt-payload");
    let case = run_case(&root, "corrupt-payload", Tamper::TruncateValue(1));

    assert_ne!(case["verify_status"], 0);
    assert_stderr_code_and_message(&case, "verify_stderr", "CALYX_LEDGER_CORRUPT", "seq=1");
    assert_eq!(case["after_quarantine_count"], 1);
    assert_eq!(case["seq_1_quarantined"], true);
    cleanup(root);
}

#[test]
fn verify_chain_vault_quarantines_key_encoded_seq_mismatch() {
    let root = test_dir("seq-mismatch");
    let case = run_case(
        &root,
        "seq-mismatch",
        Tamper::EncodedSeq { seq: 1, encoded: 9 },
    );

    assert_ne!(case["verify_status"], 0);
    assert!(
        case["verify_stderr"]
            .as_str()
            .unwrap()
            .contains("encoded seq 9")
    );
    assert_eq!(case["after_quarantine_count"], 1);
    assert_eq!(case["seq_1_quarantined"], true);
    cleanup(root);
}

#[test]
#[ignore = "manual FSV for issue #651 physical verify-chain quarantine"]
fn issue651_verify_chain_physical_quarantine_fsv_writes_readbacks() {
    let root = std::env::var("CALYX_CLI_ISSUE651_FSV_DIR")
        .map(PathBuf::from)
        .expect("CALYX_CLI_ISSUE651_FSV_DIR is required");
    reset_dir(&root);

    let happy = run_case(&root, "happy", Tamper::None);
    let missing = run_case(&root, "missing-row", Tamper::RemoveRow(1));
    let corrupt = run_case(&root, "corrupt-payload", Tamper::TruncateValue(1));
    let mismatch = run_case(
        &root,
        "seq-mismatch",
        Tamper::EncodedSeq { seq: 1, encoded: 9 },
    );
    let readback = json!({
        "happy": happy,
        "missing_row": missing,
        "corrupt_payload": corrupt,
        "seq_mismatch": mismatch,
    });
    write_json(&root.join("issue651-readback.json"), &readback);
    write_blake3_sums(&root);

    println!(
        "ISSUE651_VERIFY_CHAIN_FSV happy={} missing={} corrupt={} mismatch={}",
        readback["happy"]["verify_stdout"],
        readback["missing_row"]["verify_stderr"],
        readback["corrupt_payload"]["verify_stderr"],
        readback["seq_mismatch"]["verify_stderr"]
    );

    assert_eq!(readback["happy"]["verify_status"], 0);
    assert_eq!(readback["missing_row"]["seq_1_quarantined"], true);
    assert_eq!(readback["corrupt_payload"]["seq_1_quarantined"], true);
    assert_eq!(readback["seq_mismatch"]["seq_1_quarantined"], true);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tamper {
    None,
    RemoveRow(u64),
    TruncateValue(u64),
    EncodedSeq { seq: u64, encoded: u64 },
}

fn run_case(root: &Path, name: &str, tamper: Tamper) -> Value {
    let vault_dir = root.join(name);
    reset_dir(&vault_dir);
    let vault = write_vault(&vault_dir, 4);
    vault.flush().expect("flush vault");
    remove_wal_segments(&vault_dir);
    let before_manifest = ManifestStore::open(&vault_dir).load_current().unwrap();
    let tampered_files = apply_tamper(&vault_dir, tamper);
    let verify = run_verify(&vault_dir, "0..4");
    let readback = run_readback_ledger_seq(&vault_dir, 1);
    let after_manifest = ManifestStore::open(&vault_dir).load_current().unwrap();

    json!({
        "vault": vault_dir,
        "tamper": format!("{tamper:?}"),
        "tampered_files": tampered_files,
        "before_quarantine_count": before_manifest.quarantines.len(),
        "after_quarantine_count": after_manifest.quarantines.len(),
        "quarantine": after_manifest.quarantines.last(),
        "seq_1_quarantined": is_quarantined(&after_manifest, 1),
        "verify_status": verify.status.code(),
        "verify_stdout": stdout(&verify),
        "verify_stderr": stderr(&verify),
        "readback_status": readback.status.code(),
        "readback_stdout": stdout(&readback),
        "readback_stderr": stderr(&readback),
        "current_pointer": fs::read_to_string(vault_dir.join("CURRENT")).unwrap(),
        "manifest_bytes_hex_prefix": hex_prefix(&fs::read(vault_dir.join("MANIFEST")).unwrap(), 96),
        "ledger_sst_files": list_files(&vault_dir.join("cf").join("ledger")),
        "wal_segment_files": list_files(&vault_dir.join("wal")),
    })
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
    let input = format!("issue651-verify-chain-{seed}");
    let cx_id = vault.cx_id_for_input(input.as_bytes(), 651);
    let mut input_hash = [0_u8; 32];
    input_hash[..input.len()].copy_from_slice(input.as_bytes());
    Constellation {
        cx_id,
        vault_id: vault_id(),
        panel_version: 651,
        created_at: 1_786_000_000 + u64::from(seed),
        input_ref: InputRef {
            hash: input_hash,
            pointer: Some(format!("synthetic://ph36/issue651/{seed}")),
            redacted: false,
        },
        modality: Modality::Text,
        slots: BTreeMap::from([(
            SlotId::new(0),
            SlotVector::Dense {
                dim: 2,
                data: vec![f32::from(seed), 0.651],
            },
        )]),
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

fn apply_tamper(vault: &Path, tamper: Tamper) -> Vec<String> {
    match tamper {
        Tamper::None => Vec::new(),
        Tamper::RemoveRow(seq) => rewrite_ledger_rows(vault, seq, |rows, key| {
            let before = rows.len();
            rows.retain(|row| row.key != key);
            rows.len() != before
        }),
        Tamper::TruncateValue(seq) => rewrite_ledger_rows(vault, seq, |rows, key| {
            mutate_row(rows, key, |value| value.truncate(12))
        }),
        Tamper::EncodedSeq { seq, encoded } => rewrite_ledger_rows(vault, seq, |rows, key| {
            mutate_row(rows, key, |value| {
                value[..8].copy_from_slice(&encoded.to_be_bytes())
            })
        }),
    }
}

fn rewrite_ledger_rows(
    vault: &Path,
    seq: u64,
    mutate: impl Fn(&mut Vec<SstEntry>, Vec<u8>) -> bool,
) -> Vec<String> {
    let key = seq.to_be_bytes().to_vec();
    let mut touched = Vec::new();
    for file in sst_files(&vault.join("cf").join("ledger")) {
        let reader = SstReader::open(&file).expect("open ledger sst");
        let mut rows = reader.iter().expect("read ledger sst");
        if mutate(&mut rows, key.clone()) {
            rewrite_sst(&file, &rows);
            touched.push(file.file_name().unwrap().to_string_lossy().to_string());
        }
    }
    touched.sort();
    touched
}

fn mutate_row(rows: &mut [SstEntry], key: Vec<u8>, mutate: impl Fn(&mut Vec<u8>)) -> bool {
    for row in rows {
        if row.key == key {
            mutate(&mut row.value);
            return true;
        }
    }
    false
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

fn run_verify(vault: &Path, range: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_calyx"))
        .arg("verify-chain")
        .arg("--vault")
        .arg(vault)
        .arg("--range")
        .arg(range)
        .output()
        .expect("run calyx verify-chain")
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

fn sst_files(dir: &Path) -> Vec<PathBuf> {
    list_files(dir)
        .into_iter()
        .filter(|file| file.ends_with(".sst"))
        .map(|file| dir.join(file))
        .collect()
}

fn assert_stderr_code_and_message(case: &Value, field: &str, code: &str, message_part: &str) {
    let stderr = case[field].as_str().unwrap();
    let parsed: Value = serde_json::from_str(stderr)
        .unwrap_or_else(|error| panic!("{field} JSON: {error}: {stderr}"));

    assert_eq!(parsed["code"], code, "{field}: {stderr}");
    assert!(
        parsed["message"]
            .as_str()
            .is_some_and(|message| message.contains(message_part)),
        "{field}: {stderr}"
    );
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).trim().to_string()
}

fn hex_prefix(bytes: &[u8], len: usize) -> String {
    bytes
        .iter()
        .take(len)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn test_dir(name: &str) -> PathBuf {
    named_temp_root("calyx-cli-verify-chain-physical", name)
}

fn cleanup(dir: PathBuf) {
    fs::remove_dir_all(dir).unwrap();
}
