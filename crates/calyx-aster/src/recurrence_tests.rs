use super::*;
use crate::compaction::CompactionResult;
use crate::sst::SstReader;
use crate::vault::VaultOptions;
use crate::vault::encode::decode_write_batch;
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

#[test]
fn rollup_tombstones_are_reclaimed_from_recurrence_ssts() {
    let (root, keep_root) = reclaim_root();
    let vault_dir = root.join("vault");
    fs::create_dir_all(&root).expect("create test root");
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"recurrence-reclaim-salt".to_vec(),
        VaultOptions::default(),
    )
    .expect("open durable vault");
    let cx_id = vault.cx_id_for_input(b"recurrence-reclaim", 41);
    let retention = RetentionPolicy::new(3, u64::MAX).expect("retention");
    vault.put(base_cx(cx_id)).expect("put base");

    for index in 0..7 {
        append_occurrence(
            &vault,
            cx_id,
            EpochSecs(100 + index),
            OccurrenceContext::new(format!("ctx-{index}").into_bytes()).expect("context"),
            EpochSecs(200 + index),
            retention,
        )
        .expect("append occurrence");
    }
    vault.flush().expect("flush before compaction");

    let before_series = read_series(&vault, cx_id).expect("before series");
    let before_rows = scan(&vault, ColumnFamily::Recurrence);
    let before_ssts = recurrence_sst_files(&vault_dir);
    assert_eq!(before_series.frequency, 7);
    assert_eq!(occurrence_ids(&before_series.occurrences), vec![4, 5, 6]);
    assert!(before_ssts.len() > 1);
    assert!(row_kinds(&before_rows).contains(&"tombstone"));
    assert!(!row_kinds(&before_rows).contains(&"rolled_occurrence"));

    let compacted = vault
        .compact_cf_once(ColumnFamily::Recurrence)
        .expect("compact recurrence")
        .expect("recurrence compacted");
    let CompactionResult::Compacted(report) = compacted else {
        panic!("expected compaction report");
    };

    let after_series = read_series(&vault, cx_id).expect("after series");
    let after_rows = scan(&vault, ColumnFamily::Recurrence);
    let after_ssts = recurrence_sst_files(&vault_dir);
    let compacted_sst_rows = rows_from_ssts(&after_ssts);
    let reopened = AsterVault::open(
        &vault_dir,
        vault_id(),
        b"recurrence-reclaim-salt",
        VaultOptions::default(),
    )
    .expect("cold reopen");
    let reopened_series = read_series(&reopened, cx_id).expect("reopened series");
    let wal_rows = wal_recurrence_rows(&vault_dir);

    assert_eq!(after_series.frequency, 7);
    assert_eq!(after_series.frequency, reopened_series.frequency);
    assert_eq!(occurrence_ids(&after_series.occurrences), vec![4, 5, 6]);
    assert_eq!(after_series.occurrences, reopened_series.occurrences);
    assert_eq!(report.reclaimed_input_files, before_ssts.len());
    assert_eq!(after_ssts.len(), 1);
    let compacted_kinds = row_kinds(&compacted_sst_rows);
    assert_eq!(
        compacted_kinds
            .iter()
            .filter(|kind| **kind == "occurrence")
            .count(),
        3
    );
    assert!(compacted_kinds.contains(&"rollup_summary"));
    assert!(!compacted_kinds.contains(&"tombstone"));
    assert!(!compacted_kinds.contains(&"rolled_occurrence"));
    assert!(wal_rows.len() > after_rows.len());

    if keep_root {
        write_reclaim_readback(
            &root,
            ReclaimEvidence {
                cx_id,
                report: &report,
                before_series: &before_series,
                after_series: &after_series,
                reopened_series: &reopened_series,
                before_rows: &before_rows,
                after_rows: &after_rows,
                compacted_sst_rows: &compacted_sst_rows,
                wal_rows: &wal_rows,
                before_ssts: &before_ssts,
                after_ssts: &after_ssts,
            },
        );
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

fn recurrence_rows_json(rows: &[(Vec<u8>, Vec<u8>)]) -> Vec<serde_json::Value> {
    rows.iter()
        .map(|(key, value)| {
            let decoded = decode_recurrence_row(value).expect("decode recurrence row");
            json!({
                "key_hex": hex(key),
                "kind": row_kind(&decoded),
                "decoded": serde_json::to_value(decoded).expect("recurrence row json"),
                "value_hex": hex(value),
            })
        })
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
        metadata: BTreeMap::new(),
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

fn reclaim_root() -> (PathBuf, bool) {
    if let Ok(root) = std::env::var("CALYX_RECURRENCE_RECLAIM_FSV_ROOT") {
        return (PathBuf::from(root), true);
    }
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    (
        std::env::temp_dir().join(format!("recurrence-reclaim-{}-{nonce}", std::process::id())),
        false,
    )
}

fn occurrence_ids(occurrences: &[Occurrence]) -> Vec<u64> {
    occurrences
        .iter()
        .map(|occurrence| occurrence.id.0)
        .collect()
}

fn row_kind(row: &StoredRecurrenceRow) -> &'static str {
    match row {
        StoredRecurrenceRow::Occurrence(_) => "occurrence",
        StoredRecurrenceRow::RollupSummary(_) => "rollup_summary",
        StoredRecurrenceRow::RolledOccurrence { .. } => "rolled_occurrence",
        StoredRecurrenceRow::Tombstone { .. } => "tombstone",
    }
}

fn row_kinds(rows: &[(Vec<u8>, Vec<u8>)]) -> Vec<&'static str> {
    rows.iter()
        .map(|(_, value)| row_kind(&decode_recurrence_row(value).expect("decode row")))
        .collect()
}

fn recurrence_sst_files(vault_dir: &Path) -> Vec<PathBuf> {
    let mut files = fs::read_dir(vault_dir.join("cf/recurrence"))
        .expect("read recurrence cf dir")
        .filter_map(|entry| {
            let path = entry.expect("dir entry").path();
            (path.extension().and_then(|value| value.to_str()) == Some("sst")).then_some(path)
        })
        .collect::<Vec<_>>();
    files.sort();
    files
}

fn rows_from_ssts(files: &[PathBuf]) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut rows = Vec::new();
    for file in files {
        for entry in SstReader::open(file)
            .expect("open sst")
            .iter()
            .expect("iter sst")
        {
            rows.push((entry.key, entry.value));
        }
    }
    rows.sort_by(|left, right| left.0.cmp(&right.0));
    rows
}

fn wal_recurrence_rows(vault_dir: &Path) -> Vec<(Vec<u8>, Vec<u8>)> {
    let replay = crate::wal::replay_dir(vault_dir.join("wal")).expect("replay wal");
    let mut rows = Vec::new();
    for record in replay.records {
        let batch = decode_write_batch(&record.payload).expect("decode wal batch");
        for row in batch {
            if row.cf == ColumnFamily::Recurrence {
                rows.push((row.key, row.value));
            }
        }
    }
    rows
}

struct ReclaimEvidence<'a> {
    cx_id: CxId,
    report: &'a crate::compaction::CompactionReport,
    before_series: &'a RecurrenceSeries,
    after_series: &'a RecurrenceSeries,
    reopened_series: &'a RecurrenceSeries,
    before_rows: &'a [(Vec<u8>, Vec<u8>)],
    after_rows: &'a [(Vec<u8>, Vec<u8>)],
    compacted_sst_rows: &'a [(Vec<u8>, Vec<u8>)],
    wal_rows: &'a [(Vec<u8>, Vec<u8>)],
    before_ssts: &'a [PathBuf],
    after_ssts: &'a [PathBuf],
}

fn write_reclaim_readback(root: &Path, evidence: ReclaimEvidence<'_>) {
    let readback = json!({
        "cx_id": evidence.cx_id.to_string(),
        "retention": {
            "max_occurrences": 3,
            "appended_occurrences": 7,
        },
        "compaction": {
            "input_files": evidence.report.input_files,
            "reclaimed_input_files": evidence.report.reclaimed_input_files,
            "output_path": evidence.report.output_path,
            "output_bytes": evidence.report.output_bytes,
            "before_ssts": evidence.before_ssts,
            "after_ssts": evidence.after_ssts,
        },
        "before": {
            "frequency": evidence.before_series.frequency,
            "active_occurrence_ids": occurrence_ids(&evidence.before_series.occurrences),
            "rows": recurrence_rows_json(evidence.before_rows),
        },
        "after": {
            "frequency": evidence.after_series.frequency,
            "active_occurrence_ids": occurrence_ids(&evidence.after_series.occurrences),
            "rows": recurrence_rows_json(evidence.after_rows),
            "compacted_sst_rows": recurrence_rows_json(evidence.compacted_sst_rows),
        },
        "cold_reopen": {
            "frequency": evidence.reopened_series.frequency,
            "active_occurrence_ids": occurrence_ids(&evidence.reopened_series.occurrences),
        },
        "wal": {
            "recurrence_rows": recurrence_rows_json(evidence.wal_rows),
            "history_retained_until_wal_recycler": true,
        },
        "verdict": {
            "active_rows_bounded": evidence.after_series.occurrences.len() <= 3,
            "sst_inputs_reclaimed": evidence.report.reclaimed_input_files
                == evidence.before_ssts.len(),
            "single_compacted_recurrence_sst": evidence.after_ssts.len() == 1,
            "compacted_sst_tombstones_pruned": !row_kinds(evidence.compacted_sst_rows)
                .contains(&"tombstone"),
            "cold_reopen_matches_after": evidence.reopened_series.occurrences
                == evidence.after_series.occurrences
                && evidence.reopened_series.frequency == evidence.after_series.frequency,
            "wal_history_retained_by_contract": true,
        }
    });
    fs::write(
        root.join("recurrence-reclaim-readback.json"),
        serde_json::to_vec_pretty(&readback).expect("json"),
    )
    .expect("write reclaim readback");
    write_blake3_sums(root);
    println!("recurrence_reclaim_fsv_root={}", root.display());
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());
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
