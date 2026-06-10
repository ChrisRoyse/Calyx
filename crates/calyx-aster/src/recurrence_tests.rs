use super::*;
use crate::vault::VaultOptions;
use calyx_core::{CxFlags, InputRef, LedgerRef, Modality, VaultId};
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn wal_append_failure_leaves_recurrence_uncommitted() {
    let (root, keep_root) = test_root("recurrence-wal-fail");
    let vault_dir = root.join("vault");
    fs::create_dir_all(&root).expect("create test root");
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"recurrence-wal-fail-salt".to_vec(),
        VaultOptions::default(),
    )
    .expect("open durable vault");
    let cx_id = vault.cx_id_for_input(b"recurrence-wal-fail", 41);
    vault.put(base_cx(cx_id)).expect("put base");
    vault.flush().expect("flush base");

    let before = snapshot_state(&vault, cx_id);
    vault.fail_next_wal_append_for_test();
    let error = append_occurrence(
        &vault,
        cx_id,
        EpochSecs(100),
        OccurrenceContext::new(b"ctx".to_vec()).expect("context"),
        EpochSecs(100),
        RetentionPolicy::default(),
    )
    .expect_err("injected WAL failure");
    let after = snapshot_state(&vault, cx_id);

    assert_eq!(error.code, "CALYX_DISK_PRESSURE");
    assert_eq!(after.snapshot, before.snapshot);
    assert_eq!(after.occurrence_count, 0);
    assert!(after.series.occurrences.is_empty());
    assert!(!after.base.scalars.contains_key(FREQUENCY_SCALAR));
    assert_eq!(after.base_rows, before.base_rows);
    assert_eq!(after.recurrence_rows, before.recurrence_rows);
    assert_eq!(after.online_rows, before.online_rows);
    assert_eq!(after.ledger_rows, before.ledger_rows);

    if keep_root {
        write_fsv_readback(&root, cx_id, &error, &before, &after);
    } else {
        let _ = fs::remove_dir_all(root);
    }
}

#[derive(Debug)]
struct SnapshotState {
    snapshot: u64,
    base: Constellation,
    series: RecurrenceSeries,
    occurrence_count: u64,
    base_rows: Vec<(Vec<u8>, Vec<u8>)>,
    recurrence_rows: Vec<(Vec<u8>, Vec<u8>)>,
    online_rows: Vec<(Vec<u8>, Vec<u8>)>,
    ledger_rows: Vec<(Vec<u8>, Vec<u8>)>,
}

fn snapshot_state(vault: &AsterVault, cx_id: CxId) -> SnapshotState {
    let snapshot = vault.snapshot();
    SnapshotState {
        snapshot,
        base: vault.get(cx_id, snapshot).expect("base"),
        series: read_series(vault, cx_id).expect("series"),
        occurrence_count: occurrence_count(vault, cx_id).expect("count"),
        base_rows: scan(vault, ColumnFamily::Base),
        recurrence_rows: scan(vault, ColumnFamily::Recurrence),
        online_rows: scan(vault, ColumnFamily::Online),
        ledger_rows: scan(vault, ColumnFamily::Ledger),
    }
}

fn scan(vault: &AsterVault, cf: ColumnFamily) -> Vec<(Vec<u8>, Vec<u8>)> {
    vault.scan_cf_at(vault.snapshot(), cf).expect("scan cf")
}

fn write_fsv_readback(
    root: &Path,
    cx_id: CxId,
    error: &CalyxError,
    before: &SnapshotState,
    after: &SnapshotState,
) {
    let readback = json!({
        "chosen_error_code": "CALYX_DISK_PRESSURE",
        "wal_write_error_added": false,
        "cx_id": cx_id.to_string(),
        "error": {
            "code": error.code,
            "message": error.message,
            "remediation": error.remediation,
        },
        "before": state_json(before),
        "after": state_json(after),
        "unchanged": {
            "snapshot": after.snapshot == before.snapshot,
            "base_rows": after.base_rows == before.base_rows,
            "recurrence_rows": after.recurrence_rows == before.recurrence_rows,
            "online_rows": after.online_rows == before.online_rows,
            "ledger_rows": after.ledger_rows == before.ledger_rows,
        }
    });
    fs::write(
        root.join("recurrence-wal-failure-readback.json"),
        serde_json::to_vec_pretty(&readback).expect("json"),
    )
    .expect("write readback");
    write_blake3_sums(root);
    println!("recurrence_wal_failure_fsv_root={}", root.display());
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());
}

fn state_json(state: &SnapshotState) -> serde_json::Value {
    json!({
        "snapshot": state.snapshot,
        "frequency_scalar": state.base.scalars.get(FREQUENCY_SCALAR),
        "occurrence_count": state.occurrence_count,
        "series_frequency": state.series.frequency,
        "series_occurrences": state.series.occurrences,
        "base_rows": rows_json(&state.base_rows),
        "recurrence_rows": rows_json(&state.recurrence_rows),
        "online_rows": rows_json(&state.online_rows),
        "ledger_rows": rows_json(&state.ledger_rows),
    })
}

fn rows_json(rows: &[(Vec<u8>, Vec<u8>)]) -> Vec<serde_json::Value> {
    rows.iter()
        .map(|(key, value)| json!({ "key_hex": hex(key), "value_hex": hex(value) }))
        .collect()
}

fn base_cx(cx_id: CxId) -> Constellation {
    Constellation {
        cx_id,
        vault_id: vault_id(),
        panel_version: 41,
        created_at: 100,
        input_ref: InputRef {
            hash: *blake3::hash(b"recurrence-wal-fail").as_bytes(),
            pointer: None,
            redacted: true,
        },
        modality: Modality::Text,
        slots: BTreeMap::new(),
        scalars: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 0,
            hash: [0; 32],
        },
        flags: CxFlags {
            ungrounded: true,
            redacted_input: true,
            ..CxFlags::default()
        },
    }
}

fn test_root(name: &str) -> (PathBuf, bool) {
    if let Ok(root) = std::env::var("CALYX_RECURRENCE_WAL_FAILURE_FSV_ROOT") {
        return (PathBuf::from(root), true);
    }
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    (
        std::env::temp_dir().join(format!("{name}-{}-{nonce}", std::process::id())),
        false,
    )
}

fn write_blake3_sums(root: &Path) {
    let mut files = Vec::new();
    collect_files(root, root, &mut files);
    files.sort();
    let mut lines = String::new();
    for relative in files {
        if relative == Path::new("BLAKE3SUMS.txt") {
            continue;
        }
        let bytes = fs::read(root.join(&relative)).expect("read checksum file");
        lines.push_str(&format!(
            "{}  {}\n",
            blake3::hash(&bytes).to_hex(),
            relative.to_string_lossy().replace('\\', "/")
        ));
    }
    fs::write(root.join("BLAKE3SUMS.txt"), lines).expect("write checksum manifest");
}

fn collect_files(root: &Path, dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            collect_files(root, &path, files);
        } else {
            files.push(path.strip_prefix(root).expect("relative").to_path_buf());
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("vault id")
}
