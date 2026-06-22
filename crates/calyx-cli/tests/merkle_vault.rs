mod support;

use calyx_aster::cf::{CfRouter, ColumnFamily};
use calyx_aster::sst::SstEntry;
use calyx_aster::vault::encode::decode_write_batch;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_aster::wal::replay_dir;
use calyx_core::{
    CalyxError, Constellation, CxFlags, InputRef, LedgerRef, Modality, Result as CalyxResult,
    SlotId, SlotVector, VaultId, VaultStore,
};
use calyx_ledger::{LedgerCfStore, LedgerRow, merkle_root};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use support::fsv_io::{fsv_root, list_files, named_temp_root, reset_dir, write_json};

#[test]
fn vault_merkle_reads_flushed_aster_ledger_cf_without_side_dirs() {
    let dir = test_dir("flushed-cf");
    reset_dir(&dir);
    let vault = open_vault(&dir);

    vault.put(sample_constellation(&vault, 1)).expect("put");
    vault.flush().expect("flush");

    let direct = SnapshotLedgerStore::from_cf(&dir).expect("read direct CF");
    let expected = hex(&merkle_root(&direct, 0..1).expect("direct Merkle root"));
    let output = run_merkle_root(&dir, "0..1");

    assert_success(&output);
    assert_eq!(stdout(&output), expected);
    assert_eq!(direct.row_count(), 1);
    assert_no_side_ledger_dirs(&dir);
    cleanup(dir);
}

#[test]
fn vault_merkle_reads_wal_when_ledger_checkpoint_sst_is_missing() {
    let dir = test_dir("wal-recovery");
    reset_dir(&dir);
    let vault = open_vault(&dir);

    vault.put(sample_constellation(&vault, 2)).expect("put");
    remove_sst_files(&dir.join("cf").join("ledger"));

    let direct = SnapshotLedgerStore::from_wal(&dir).expect("read direct WAL");
    let expected = hex(&merkle_root(&direct, 0..1).expect("direct Merkle root"));
    let output = run_merkle_root(&dir, "0..1");

    assert_success(&output);
    assert_eq!(stdout(&output), expected);
    assert_eq!(direct.row_count(), 1);
    assert_no_side_ledger_dirs(&dir);
    cleanup(dir);
}

#[test]
fn vault_merkle_fails_closed_without_aster_layout() {
    let dir = test_dir("empty-dir");
    reset_dir(&dir);
    let output = run_merkle_root(&dir, "0..1");
    let stderr = stderr(&output);

    assert!(!output.status.success());
    assert!(stderr.contains("CALYX_LEDGER_CORRUPT"), "{stderr}");
    assert!(stderr.contains("cf/ledger"), "{stderr}");
    assert!(stderr.contains("wal"), "{stderr}");
    assert!(!dir.join("cf").exists());
    assert_no_side_ledger_dirs(&dir);
    cleanup(dir);
}

#[test]
#[ignore = "manual FSV for issue #348"]
fn ph36_merkle_vault_real_aster_cf_manual_fsv() {
    let root = fsv_root("CALYX_FSV_ROOT", "calyx-ph36-merkle-vault-fsv")
        .join("merkle-vault-real-aster-cf");
    reset_dir(&root);
    let vault_dir = root.join("vault");
    reset_dir(&vault_dir);
    let before_side_dirs = side_ledger_dirs(&vault_dir);
    let vault = open_vault(&vault_dir);

    let before_cf_rows = SnapshotLedgerStore::from_cf(&vault_dir)
        .expect("read before CF")
        .row_count();
    vault
        .put(sample_constellation(&vault, 42))
        .expect("put synthetic constellation");
    vault.flush().expect("flush durable vault");

    let cf_store = SnapshotLedgerStore::from_cf(&vault_dir).expect("read CF after");
    let wal_store = SnapshotLedgerStore::from_wal(&vault_dir).expect("read WAL after");
    let cli_output = run_merkle_root(&vault_dir, "0..1");
    assert_success(&cli_output);
    let cf_root = hex(&merkle_root(&cf_store, 0..1).expect("CF Merkle root"));
    let wal_root = hex(&merkle_root(&wal_store, 0..1).expect("WAL Merkle root"));
    let empty_output = run_merkle_root(&vault_dir, "0..0");
    assert_success(&empty_output);
    let missing_output = run_merkle_root(&vault_dir, "0..2");
    let empty_dir = root.join("empty-not-vault");
    reset_dir(&empty_dir);
    let fail_closed_output = run_merkle_root(&empty_dir, "0..1");

    let readback = serde_json::json!({
        "vault": vault_dir,
        "before": {
            "cf_rows": before_cf_rows,
            "side_ledger_dirs": before_side_dirs,
        },
        "after": {
            "cf_rows": cf_store.row_count(),
            "wal_rows": wal_store.row_count(),
            "ledger_sst_files": list_files(&vault_dir.join("cf").join("ledger")),
            "wal_segment_files": list_files(&vault_dir.join("wal")),
            "cli_root_0_1": stdout(&cli_output),
            "direct_cf_root_0_1": cf_root,
            "direct_wal_root_0_1": wal_root,
            "cli_matches_direct_cf": stdout(&cli_output) == cf_root,
            "cli_matches_direct_wal": stdout(&cli_output) == wal_root,
            "empty_range_root": stdout(&empty_output),
            "missing_range_error": stderr(&missing_output),
            "fail_closed_error": stderr(&fail_closed_output),
            "side_ledger_dirs": side_ledger_dirs(&vault_dir),
        }
    });
    let readback_path = root.join("merkle-vault-readback.json");
    write_json(&readback_path, &readback);

    println!("PH36_MERKLE_VAULT_FSV_ROOT={}", root.display());
    println!("PH36_MERKLE_VAULT_READBACK={}", readback_path.display());
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    assert_eq!(before_cf_rows, 0);
    assert_eq!(cf_store.row_count(), 1);
    assert_eq!(wal_store.row_count(), 1);
    assert_eq!(stdout(&cli_output), cf_root);
    assert_eq!(stdout(&cli_output), wal_root);
    assert_eq!(
        stdout(&empty_output),
        "0000000000000000000000000000000000000000000000000000000000000000"
    );
    assert!(!missing_output.status.success());
    assert!(stderr(&missing_output).contains("CALYX_LEDGER_CORRUPT"));
    assert!(!fail_closed_output.status.success());
    assert!(stderr(&fail_closed_output).contains("cf/ledger"));
    assert_no_side_ledger_dirs(&vault_dir);
    assert_no_side_ledger_dirs(&empty_dir);
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct SnapshotLedgerStore {
    rows: BTreeMap<u64, Vec<u8>>,
}

impl SnapshotLedgerStore {
    fn from_cf(vault: &Path) -> CalyxResult<Self> {
        let router = CfRouter::open(vault, 0)?;
        let mut store = Self::default();
        for entry in router.iter_cf(ColumnFamily::Ledger)? {
            store.insert_sst(entry)?;
        }
        Ok(store)
    }

    fn from_wal(vault: &Path) -> CalyxResult<Self> {
        let replay = replay_dir(vault.join("wal"))?;
        if let Some(torn) = replay.torn_tail {
            return Err(torn.error());
        }
        let mut store = Self::default();
        for record in replay.records {
            for row in decode_write_batch(&record.payload)? {
                if row.cf == ColumnFamily::Ledger {
                    store.insert(parse_seq(&row.key)?, row.value)?;
                }
            }
        }
        Ok(store)
    }

    fn row_count(&self) -> usize {
        self.rows.len()
    }

    fn insert_sst(&mut self, entry: SstEntry) -> CalyxResult<()> {
        self.insert(parse_seq(&entry.key)?, entry.value)
    }

    fn insert(&mut self, seq: u64, bytes: Vec<u8>) -> CalyxResult<()> {
        if let Some(existing) = self.rows.get(&seq) {
            if existing == &bytes {
                return Ok(());
            }
            return Err(CalyxError::ledger_corrupt(format!(
                "divergent ledger bytes for seq {seq}"
            )));
        }
        self.rows.insert(seq, bytes);
        Ok(())
    }
}

impl LedgerCfStore for SnapshotLedgerStore {
    fn scan(&self) -> CalyxResult<Vec<LedgerRow>> {
        Ok(self
            .rows
            .iter()
            .map(|(seq, bytes)| LedgerRow {
                seq: *seq,
                bytes: bytes.clone(),
            })
            .collect())
    }

    fn put_new(&mut self, seq: u64, _bytes: &[u8]) -> CalyxResult<()> {
        Err(CalyxError::ledger_append_only_violation(format!(
            "test snapshot store is read-only for seq {seq}"
        )))
    }
}

fn parse_seq(key: &[u8]) -> CalyxResult<u64> {
    let key: [u8; 8] = key
        .try_into()
        .map_err(|_| CalyxError::ledger_corrupt("ledger key width must be 8 bytes"))?;
    Ok(u64::from_be_bytes(key))
}

fn open_vault(dir: &Path) -> AsterVault {
    AsterVault::new_durable(dir, vault_id(), b"salt".to_vec(), VaultOptions::default())
        .expect("open durable vault")
}

fn sample_constellation(vault: &AsterVault, seed: u16) -> Constellation {
    let input = format!("ph36-cli-merkle-vault-{seed}");
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
        created_at: 1_785_100_000 + u64::from(seed),
        input_ref: InputRef {
            hash: input_hash,
            pointer: Some(format!("synthetic://ph36/merkle-vault/{seed}")),
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

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn side_ledger_dirs(vault: &Path) -> Vec<String> {
    ["ledger", "ledger-cf"]
        .into_iter()
        .filter(|name| vault.join(name).exists())
        .map(str::to_string)
        .collect()
}

fn assert_no_side_ledger_dirs(vault: &Path) {
    assert_eq!(side_ledger_dirs(vault), Vec::<String>::new());
}

fn remove_sst_files(dir: &Path) {
    for file in list_files(dir) {
        if file.ends_with(".sst") {
            fs::remove_file(dir.join(file)).unwrap();
        }
    }
}

fn test_dir(name: &str) -> PathBuf {
    named_temp_root("calyx-cli-merkle-vault", name)
}

fn cleanup(dir: PathBuf) {
    fs::remove_dir_all(dir).unwrap();
}
