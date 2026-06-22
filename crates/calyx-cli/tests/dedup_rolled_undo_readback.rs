use calyx_aster::cf::ColumnFamily;
use calyx_aster::dedup::{
    DedupAction, DedupPolicy, DedupResult, EpochSecs, IngestInput, TauStrategy, TctCosineConfig,
    ingest_at_with_retention,
};
use calyx_aster::recurrence::{
    RetentionPolicy, StoredRecurrenceRow, decode_recurrence_row, read_series,
};
use calyx_aster::sst::SstReader;
use calyx_aster::vault::encode::decode_write_batch;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_aster::wal::replay_dir;
use calyx_core::{CxId, Modality, SlotId, SlotVector, VaultId, VaultStore};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[path = "support/dedup_fsv_io.rs"]
mod dedup_fsv_io;

use dedup_fsv_io::{
    fsv_root, list_dir_files as list_files, reset_dir, write_blake3_sums, write_json, write_text,
};

const VAULT_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const SALT: &str = "dedup-rolled-undo-readback-salt";

#[test]
fn dedup_undo_clears_rolled_recurrence_summary_readback() {
    let (root, keep) = fsv_root(
        "CALYX_DEDUP_ROLLED_UNDO_FSV_ROOT",
        "calyx-dedup-rolled-undo-fsv",
    );
    reset_dir(&root);
    let vault_dir = root.join("vault");
    let vault = durable_vault(&vault_dir);
    let retention = RetentionPolicy::new(3, u64::MAX).expect("small retention");
    let mut results = Vec::new();
    let mut target = None;
    for index in 0..7 {
        let result = ingest_at_with_retention(
            &vault,
            &input(index),
            EpochSecs(1_000 + index as i64 * 100),
            None,
            retention,
        )
        .expect("ingest rolled recurrence");
        if let DedupResult::New(cx_id) = result {
            target = Some(cx_id);
        }
        results.push(json!(result));
    }
    vault.flush().expect("flush before undo");
    let target = target.expect("target id");

    let series_before = recurrence_series(&vault_dir, target);
    let audit_before = dedup_audit(&vault_dir, target);
    let token = serde_json::to_string(&audit_before["reversal_token"]).expect("token");
    let recurrence_rows_before = decoded_latest_recurrence_rows(&vault_dir);
    let raw_base_before = raw_cf(&vault_dir, "base");
    let raw_recurrence_before = raw_cf(&vault_dir, "recurrence");
    let raw_wal_before = raw_wal(&vault_dir);
    let undo = stdout_json(readback(&[
        "readback",
        "dedup-undo",
        "--vault",
        &display(&vault_dir),
        "--token",
        &token,
    ]));
    let series_after_undo = recurrence_series(&vault_dir, target);
    let audit_after_undo = dedup_audit(&vault_dir, target);
    let recurrence_rows_after_undo = decoded_latest_recurrence_rows(&vault_dir);
    let cx_list_after_undo = stdout_json(readback(&[
        "readback",
        "cx-list",
        "--vault",
        &display(&vault_dir),
    ]));
    let raw_base_after_undo = raw_cf(&vault_dir, "base");
    let raw_recurrence_after_undo = raw_cf(&vault_dir, "recurrence");
    let raw_wal_after_undo = raw_wal(&vault_dir);
    let reopened = AsterVault::open(
        &vault_dir,
        vault_id(),
        SALT.as_bytes().to_vec(),
        VaultOptions::default(),
    )
    .expect("reopen before compaction");
    let compact = format!(
        "{:?}",
        reopened
            .compact_cf_once(ColumnFamily::Recurrence)
            .expect("compact recurrence")
    );
    drop(reopened);
    let active_sst_rows_after_compact = decoded_recurrence_sst_rows(&vault_dir);
    let cold = AsterVault::open(
        &vault_dir,
        vault_id(),
        SALT.as_bytes().to_vec(),
        VaultOptions::default(),
    )
    .expect("cold reopen after compaction");
    let cold_series = read_series(&cold, target).expect("cold series after compaction");
    let series_after_compact = recurrence_series(&vault_dir, target);
    let recurrence_rows_after_compact = decoded_latest_recurrence_rows(&vault_dir);
    let raw_base_after_compact = raw_cf(&vault_dir, "base");
    let raw_recurrence_after_compact = raw_cf(&vault_dir, "recurrence");
    let raw_wal_after_compact = raw_wal(&vault_dir);

    let readback = json!({
        "vault": vault_dir,
        "target": target,
        "retention": {"max_occurrences": 3, "max_age_secs": u64::MAX},
        "results": results,
        "series_before": series_before,
        "audit_before": audit_before,
        "recurrence_rows_before": recurrence_rows_before,
        "raw_base_before": raw_base_before,
        "raw_recurrence_before": raw_recurrence_before,
        "raw_wal_before": raw_wal_before,
        "undo": undo,
        "series_after_undo": series_after_undo,
        "audit_after_undo": audit_after_undo,
        "recurrence_rows_after_undo": recurrence_rows_after_undo,
        "cx_list_after_undo": cx_list_after_undo,
        "raw_base_after_undo": raw_base_after_undo,
        "raw_recurrence_after_undo": raw_recurrence_after_undo,
        "raw_wal_after_undo": raw_wal_after_undo,
        "compact": compact,
        "active_sst_rows_after_compact": active_sst_rows_after_compact,
        "series_after_compact": series_after_compact,
        "cold_reopen_after_compact": {
            "snapshot": cold.snapshot(),
            "frequency": cold_series.frequency,
            "occurrence_count": cold_series.occurrences.len(),
            "rollup_summary": cold_series.rollup_summary,
        },
        "recurrence_rows_after_compact": recurrence_rows_after_compact,
        "raw_base_after_compact": raw_base_after_compact,
        "raw_recurrence_after_compact": raw_recurrence_after_compact,
        "raw_wal_after_compact": raw_wal_after_compact,
        "files_after": list_files(&root),
    });
    write_json(&root.join("dedup-rolled-undo-readback.json"), &readback);
    for (name, field) in [
        ("base-before.tsv", "raw_base_before"),
        ("base-after-undo.tsv", "raw_base_after_undo"),
        ("base-after-compact.tsv", "raw_base_after_compact"),
        ("recurrence-before.tsv", "raw_recurrence_before"),
        ("recurrence-after-undo.tsv", "raw_recurrence_after_undo"),
        (
            "recurrence-after-compact.tsv",
            "raw_recurrence_after_compact",
        ),
        ("wal-before.tsv", "raw_wal_before"),
        ("wal-after-undo.tsv", "raw_wal_after_undo"),
        ("wal-after-compact.tsv", "raw_wal_after_compact"),
    ] {
        write_text(&root.join(name), readback[field].as_str().unwrap());
    }
    write_blake3_sums(&root);

    assert_eq!(readback["series_before"]["frequency"], json!(7));
    assert_eq!(readback["series_before"]["occurrence_count"], json!(3));
    assert_eq!(
        readback["series_before"]["rollup_summary"]["count_rolled"],
        json!(4)
    );
    assert!(has_kind(
        &readback["recurrence_rows_before"],
        "rollup_summary"
    ));
    assert_eq!(readback["undo"]["restored"].as_array().unwrap().len(), 6);
    assert_eq!(readback["cx_list_after_undo"].as_array().unwrap().len(), 7);
    assert_eq!(readback["series_after_undo"]["frequency"], json!(0));
    assert_eq!(readback["series_after_undo"]["occurrence_count"], json!(0));
    assert_eq!(readback["series_after_undo"]["rollup_summary"], Value::Null);
    assert!(!has_kind(
        &readback["recurrence_rows_after_undo"],
        "rollup_summary"
    ));
    assert!(has_kind(
        &readback["recurrence_rows_after_undo"],
        "tombstone"
    ));
    assert!(
        readback["active_sst_rows_after_compact"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        readback["cold_reopen_after_compact"]["rollup_summary"],
        Value::Null
    );
    assert_eq!(readback["cold_reopen_after_compact"]["frequency"], json!(0));
    assert!(!has_kind(
        &readback["recurrence_rows_after_compact"],
        "rollup_summary"
    ));

    println!("{}", serde_json::to_string_pretty(&readback).unwrap());
    if !keep {
        fs::remove_dir_all(root).expect("cleanup root");
    }
}

fn durable_vault(vault_dir: &Path) -> AsterVault {
    AsterVault::new_durable(
        vault_dir,
        vault_id(),
        SALT.as_bytes().to_vec(),
        VaultOptions {
            dedup_policy: Some(policy()),
            ..VaultOptions::default()
        },
    )
    .expect("open durable vault")
}

fn policy() -> DedupPolicy {
    DedupPolicy::TctCosine(
        TctCosineConfig::new(
            vec![slot(0)],
            TauStrategy::PerSlot(vec![(slot(0), 0.90)]),
            DedupAction::RecurrenceSeries,
        )
        .expect("policy"),
    )
}

fn input(index: usize) -> IngestInput {
    let temporal = [
        [1.0, 0.0],
        [0.0, 1.0],
        [-1.0, 0.0],
        [0.0, -1.0],
        [0.707, 0.707],
        [-0.707, 0.707],
        [0.707, -0.707],
    ][index];
    IngestInput::new(
        format!("rolled-undo-{index}").into_bytes(),
        62,
        Modality::Text,
    )
    .with_slot(
        slot(0),
        SlotVector::Dense {
            dim: 2,
            data: vec![1.0, 0.0],
        },
    )
    .with_slot(
        slot(20),
        SlotVector::Dense {
            dim: 2,
            data: temporal.to_vec(),
        },
    )
    .with_temporal_slot(slot(20))
}

fn recurrence_series(vault_dir: &Path, cx_id: CxId) -> Value {
    stdout_json(readback(&[
        "readback",
        "recurrence-series",
        "--vault",
        &display(vault_dir),
        "--cx-id",
        &cx_id.to_string(),
    ]))
}

fn dedup_audit(vault_dir: &Path, cx_id: CxId) -> Value {
    stdout_json(readback(&[
        "readback",
        "dedup-audit",
        "--vault",
        &display(vault_dir),
        "--cx-id",
        &cx_id.to_string(),
    ]))
}

fn decoded_latest_recurrence_rows(vault_dir: &Path) -> Value {
    json!(
        latest_cf_rows(vault_dir, ColumnFamily::Recurrence)
            .into_iter()
            .map(|(key, value)| recurrence_row_json(&key, &value))
            .collect::<Vec<_>>()
    )
}

fn decoded_recurrence_sst_rows(vault_dir: &Path) -> Value {
    let mut rows = Vec::new();
    for file in list_sst_files(&vault_dir.join("cf").join("recurrence")) {
        for row in SstReader::open(&file)
            .expect("open recurrence sst")
            .iter()
            .expect("read recurrence sst")
        {
            rows.push(recurrence_row_json(&row.key, &row.value));
        }
    }
    json!(rows)
}

fn recurrence_row_json(key: &[u8], value: &[u8]) -> Value {
    match decode_recurrence_row(value).expect("decode recurrence row") {
        StoredRecurrenceRow::Occurrence(occurrence) => json!({
            "kind": "occurrence",
            "key_hex": hex_bytes(key),
            "id": occurrence.id.0,
            "t_k": occurrence.t_k.0,
        }),
        StoredRecurrenceRow::RollupSummary(summary) => json!({
            "kind": "rollup_summary",
            "key_hex": hex_bytes(key),
            "count_rolled": summary.count_rolled,
            "oldest_t": summary.oldest_t.0,
        }),
        StoredRecurrenceRow::RolledOccurrence { id, rolled_into } => json!({
            "kind": "rolled_occurrence",
            "key_hex": hex_bytes(key),
            "id": id.0,
            "rolled_into": rolled_into.0,
        }),
        StoredRecurrenceRow::Tombstone { id } => json!({
            "kind": "tombstone",
            "key_hex": hex_bytes(key),
            "id": id.0,
        }),
    }
}

fn latest_cf_rows(vault_dir: &Path, cf: ColumnFamily) -> BTreeMap<Vec<u8>, Vec<u8>> {
    let mut rows = BTreeMap::new();
    for file in list_sst_files(&vault_dir.join("cf").join(cf.name())) {
        for row in SstReader::open(&file)
            .expect("open sst")
            .iter()
            .expect("read sst")
        {
            rows.insert(row.key, row.value);
        }
    }
    for record in replay_dir(vault_dir.join("wal"))
        .expect("replay wal")
        .records
    {
        for row in decode_write_batch(&record.payload).expect("decode wal") {
            if row.cf == cf {
                rows.insert(row.key, row.value);
            }
        }
    }
    rows
}

fn list_sst_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files = entries
        .map(|entry| entry.expect("entry").path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("sst"))
        .collect::<Vec<_>>();
    files.sort();
    files
}

fn has_kind(value: &Value, kind: &str) -> bool {
    value
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["kind"].as_str() == Some(kind))
}

fn readback(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_calyx"))
        .args(args)
        .output()
        .expect("run calyx")
}

fn command_stdout(args: &[&str]) -> String {
    stdout(readback(args))
}

fn raw_cf(vault_dir: &Path, cf: &str) -> String {
    command_stdout(&["readback", "--cf", cf, "--vault", &display(vault_dir)])
}

fn raw_wal(vault_dir: &Path) -> String {
    command_stdout(&["readback", "--wal", "--vault", &display(vault_dir)])
}

fn stdout_json(output: Output) -> Value {
    serde_json::from_str(&stdout(output)).expect("json stdout")
}

fn stdout(output: Output) -> String {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn display(path: &Path) -> String {
    path.display().to_string()
}

fn slot(value: u16) -> SlotId {
    SlotId::new(value)
}

fn vault_id() -> VaultId {
    VAULT_ID.parse().expect("valid vault id")
}
