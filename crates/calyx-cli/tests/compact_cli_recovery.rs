use calyx_aster::cf::{ColumnFamily, base_key};
use calyx_aster::manifest::ManifestStore;
use calyx_aster::sst::write_sst;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Constellation, CxFlags, InputRef, LedgerRef, Modality, SlotId, SlotVector, VaultId, VaultStore,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

mod support;
use support::fsv_io::{
    case_fsv_root, reset_dir, write_blake3_sums_by_path as write_blake3_sums, write_json,
    write_text,
};

const VAULT_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const SALT: &str = "cli-compact-recovery-salt";

#[test]
fn compact_cli_uses_durable_recovery_safe_sst_and_cold_open_survives() {
    let (root, keep_root) = test_root("happy");
    reset_dir(&root);
    let vault_dir = root.join("vault");
    let base_dir = vault_dir.join("cf").join("base");
    let before = json!({
        "root_exists": root.exists(),
        "vault_current_exists": vault_dir.join("CURRENT").exists(),
        "base_files": list_names(&base_dir),
    });
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        SALT.as_bytes().to_vec(),
        VaultOptions::default(),
    )
    .expect("open durable vault");
    let first = sample_constellation(&vault, "first", 0x31);
    let second = sample_constellation(&vault, "second", 0x32);
    let first_id = first.cx_id;
    let second_id = second.cx_id;

    vault.put(first.clone()).expect("put first");
    vault.put(second.clone()).expect("put second");
    vault.flush().expect("flush before CLI compact");
    let manifest_before = manifest_readback(&vault_dir);
    let base_before_files = list_names(&base_dir);
    let raw_base_before = run_readback_base(&vault_dir);
    assert_success(&raw_base_before);
    assert!(base_before_files.len() >= 2);
    drop(vault);

    let compact = run_compact(&vault_dir, "base");
    assert_success(&compact);
    let base_after_files = list_names(&base_dir);
    let compacted_name = base_after_files
        .first()
        .expect("one compacted base SST")
        .clone();
    let compacted_path = base_dir.join(&compacted_name);
    let raw_base_after = run_readback_base(&vault_dir);
    assert_success(&raw_base_after);
    let reopened = AsterVault::open(
        &vault_dir,
        vault_id(),
        SALT.as_bytes().to_vec(),
        VaultOptions::default(),
    )
    .expect("cold open compacted vault");
    let got_first = reopened
        .get(first_id, reopened.snapshot())
        .expect("cold first");
    let got_second = reopened
        .get(second_id, reopened.snapshot())
        .expect("cold second");
    let first_base_bytes = reopened
        .read_cf_at(reopened.snapshot(), ColumnFamily::Base, &base_key(first_id))
        .expect("read first base")
        .expect("first base row");
    let second_base_bytes = reopened
        .read_cf_at(
            reopened.snapshot(),
            ColumnFamily::Base,
            &base_key(second_id),
        )
        .expect("read second base")
        .expect("second base row");
    let compacted_bytes = fs::read(&compacted_path).expect("read compacted SST");
    let manifest_after = manifest_readback(&vault_dir);
    let readback = json!({
        "before": before,
        "manifest_before": manifest_before,
        "base_before_files": base_before_files,
        "compact_stdout": stdout(&compact),
        "compact_stderr": stderr(&compact),
        "manifest_after": manifest_after,
        "base_after_files": base_after_files,
        "compacted_name_recovery_safe": durable_cli_name(
            &compacted_name,
            manifest_before["durable_seq"].as_u64().unwrap(),
        ),
        "raw_base_before": stdout(&raw_base_before),
        "raw_base_after": stdout(&raw_base_after),
        "cold_open_snapshot": reopened.snapshot(),
        "cold_first_id": got_first.cx_id,
        "cold_second_id": got_second.cx_id,
        "first_base_blake3": blake3_hex(&first_base_bytes),
        "second_base_blake3": blake3_hex(&second_base_bytes),
        "compacted_sst_blake3": blake3_hex(&compacted_bytes),
        "compacted_sst_len": compacted_bytes.len(),
    });
    write_json(&root.join("cli-compact-readback.json"), &readback);
    write_text(&root.join("compact-stdout.txt"), stdout(&compact));
    write_text(&root.join("compact-stderr.txt"), stderr(&compact));
    write_text(
        &root.join("base-readback-before.tsv"),
        stdout(&raw_base_before),
    );
    write_text(
        &root.join("base-readback-after.tsv"),
        stdout(&raw_base_after),
    );
    write_text(&root.join("compacted-sst.hex"), hex_bytes(&compacted_bytes));
    write_blake3_sums(&root);

    assert_eq!(base_after_files.len(), 1);
    assert!(stdout(&compact).contains("COMPACTED"));
    assert!(durable_cli_name(
        &compacted_name,
        manifest_before["durable_seq"].as_u64().unwrap()
    ));
    assert_recovered_matches(first, got_first);
    assert_recovered_matches(second, got_second);
    assert_eq!(
        manifest_before["durable_seq"],
        manifest_after["durable_seq"]
    );
    assert!(stdout(&raw_base_after).contains(&hex_bytes(&base_key(first_id))));
    assert!(stdout(&raw_base_after).contains(&hex_bytes(&base_key(second_id))));

    println!("cli_compact_recovery_fsv_root={}", root.display());
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());
    cleanup(root, keep_root);
}

#[test]
fn compact_cli_rejects_unbounded_sst_names_in_durable_vault() {
    let (root, keep_root) = test_root("reject-hidden");
    reset_dir(&root);
    let vault_dir = root.join("vault");
    let base_dir = vault_dir.join("cf").join("base");
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        SALT.as_bytes().to_vec(),
        VaultOptions::default(),
    )
    .expect("open durable vault");
    vault
        .put(sample_constellation(&vault, "hidden-a", 0x41))
        .expect("put hidden a");
    vault
        .put(sample_constellation(&vault, "hidden-b", 0x42))
        .expect("put hidden b");
    vault.flush().expect("flush hidden setup");
    drop(vault);
    // Layer 1: a non-canonical SST name fails closed at listing time.
    let hidden = base_dir.join("compact-legacy.sst");
    write_sst(&hidden, [(b"hidden".as_slice(), b"row".as_slice())]).expect("write hidden SST");
    let before_files = list_names(&base_dir);
    let output = run_compact(&vault_dir, "base");
    let after_files = list_names(&base_dir);
    fs::remove_file(&hidden).expect("remove non-canonical SST");

    // Layer 2: a canonical durable name beyond CURRENT durable_seq is refused
    // by the manifest bound check.
    let durable_seq = manifest_readback(&vault_dir)["durable_seq"]
        .as_u64()
        .expect("durable seq");
    let unbounded = base_dir.join(format!("{:020}-0000.sst", durable_seq + 1));
    write_sst(&unbounded, [(b"future".as_slice(), b"row".as_slice())])
        .expect("write unbounded SST");
    let unbounded_before_files = list_names(&base_dir);
    let unbounded_output = run_compact(&vault_dir, "base");
    let unbounded_after_files = list_names(&base_dir);
    let readback = json!({
        "before_files": before_files,
        "after_files": after_files,
        "status_success": output.status.success(),
        "stderr": stderr(&output),
        "stdout": stdout(&output),
        "unbounded_before_files": unbounded_before_files,
        "unbounded_after_files": unbounded_after_files,
        "unbounded_status_success": unbounded_output.status.success(),
        "unbounded_stderr": stderr(&unbounded_output),
        "unbounded_sst_still_present": unbounded.exists(),
    });
    write_json(
        &root.join("cli-compact-hidden-reject-readback.json"),
        &readback,
    );
    write_blake3_sums(&root);

    assert!(!output.status.success());
    assert!(stderr(&output).contains("CALYX_ASTER_CORRUPT_SHARD"));
    assert!(stderr(&output).contains("compact-legacy.sst"));
    assert_eq!(before_files, after_files);
    assert!(!unbounded_output.status.success());
    assert!(stderr(&unbounded_output).contains("not bounded by CURRENT durable_seq"));
    assert!(unbounded.exists());
    assert_eq!(unbounded_before_files, unbounded_after_files);

    println!("cli_compact_hidden_reject_fsv_root={}", root.display());
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());
    cleanup(root, keep_root);
}

fn sample_constellation(vault: &AsterVault, label: &str, seed: u8) -> Constellation {
    let input = format!("cli-compact-{label}");
    let cx_id = vault.cx_id_for_input(input.as_bytes(), 62);
    let mut input_hash = [0_u8; 32];
    input_hash[..input.len()].copy_from_slice(input.as_bytes());
    let mut slots = BTreeMap::new();
    slots.insert(
        SlotId::new(0),
        SlotVector::Dense {
            dim: 2,
            data: vec![f32::from(seed), 1.0],
        },
    );
    Constellation {
        cx_id,
        vault_id: vault_id(),
        panel_version: 62,
        created_at: 1_786_437_000 + u64::from(seed),
        input_ref: InputRef {
            hash: input_hash,
            pointer: Some(format!("synthetic://issue627/{label}")),
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 0,
            hash: [seed; 32],
        },
        flags: CxFlags::default(),
    }
}

fn manifest_readback(vault_dir: &Path) -> Value {
    let store = ManifestStore::open(vault_dir);
    let manifest = store.load_current().expect("load manifest");
    json!({
        "current_pointer": store.current_pointer().expect("current pointer"),
        "manifest_seq": manifest.manifest_seq,
        "durable_seq": manifest.durable_seq,
        "degraded_rebuildable": manifest.degraded_rebuildable,
    })
}

fn durable_cli_name(name: &str, durable_seq: u64) -> bool {
    let Some(stem) = name.strip_suffix(".sst") else {
        return false;
    };
    let Some((seq, index)) = stem.split_once('-') else {
        return false;
    };
    seq.parse::<u64>().ok() == Some(durable_seq)
        && index
            .parse::<u16>()
            .is_ok_and(|value| (9_000..=9_999).contains(&value))
}

fn run_compact(vault: &Path, cf: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_calyx"))
        .arg("compact")
        .arg("--vault")
        .arg(vault)
        .arg("--cf")
        .arg(cf)
        .output()
        .expect("run calyx compact")
}

fn run_readback_base(vault: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_calyx"))
        .arg("readback")
        .arg("--cf")
        .arg("base")
        .arg("--vault")
        .arg(vault)
        .output()
        .expect("run calyx readback base")
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        stdout(output),
        stderr(output)
    );
}

fn assert_recovered_matches(mut expected: Constellation, got: Constellation) {
    expected.provenance = got.provenance.clone();
    assert_ne!(got.provenance.hash, [0; 32]);
    assert_eq!(got, expected);
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).trim().to_string()
}

fn list_names(dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names = entries
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .to_string()
        })
        .collect::<Vec<_>>();
    names.sort();
    names
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

fn test_root(name: &str) -> (PathBuf, bool) {
    case_fsv_root("CALYX_CLI_COMPACT_FSV_ROOT", "calyx-cli-compact", name)
}

fn cleanup(root: PathBuf, keep_root: bool) {
    if !keep_root {
        fs::remove_dir_all(root).expect("cleanup test root");
    }
}
