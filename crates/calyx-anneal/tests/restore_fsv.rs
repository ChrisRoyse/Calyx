use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_anneal::{
    AsterAnnealLedgerStore, BaseFaultEvent, BaseShard, RestoreCommand, RestoreConfig, ShardId,
    alert_operator, attempt_restore, base_shard_checksum, decode_anneal_ledger_payload,
    fail_reads_on_range, install_recorded_read_barriers, record_base_shard_checksum,
    verify_base_shards,
};
use calyx_aster::cf::{ColumnFamily, KeyRange, base_key};
use calyx_aster::mvcc::CALYX_ASTER_BASE_CORRUPT;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{CxId, FixedClock};
use calyx_ledger::{ActorId, EntryKind, LedgerAppender, decode as decode_ledger};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

mod fsv_support;
use fsv_support::{strict_physical_files as physical_files, vault_id};

const TEST_TS: u64 = 1_785_600_903;

#[ignore = "manual FSV for #403 base-shard fail-closed restore path"]
#[test]
fn ph44_base_shard_restore_manual_fsv() {
    let root = reset_dir(&fsv_root().join(format!("issue403-{}", std::process::id())));
    let vault_dir = reset_dir(&root.join("vault"));
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"issue403-restore-fsv-salt".to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let clock = FixedClock::new(TEST_TS);
    let blocked = cx(0x43);
    let outside = cx(0x44);
    write_base(&vault, blocked, b"issue403-blocked-base-row");
    write_base(&vault, outside, b"issue403-outside-base-row");
    vault.flush().unwrap();

    let empty_events = verify_base_shards(&vault, &clock).unwrap();
    assert!(empty_events.is_empty());
    let empty_edge = json!({
        "before_checksum_rows": raw_rows(&vault, ColumnFamily::AnnealChecksums),
        "outcome_count": empty_events.len(),
        "after_checksum_rows": raw_rows(&vault, ColumnFamily::AnnealChecksums),
    });
    let base_rows_before = raw_rows(&vault, ColumnFamily::Base);
    let range = cx_range(blocked);
    let actual = base_shard_checksum(&vault, &range).unwrap();
    let shard = BaseShard::new(ShardId::new("shard_43"), range, flip(actual));
    record_base_shard_checksum(&vault, &shard, &clock).unwrap();
    let events = verify_base_shards(&vault, &clock).unwrap();
    assert_eq!(events.len(), 1);
    let event = events[0].clone();
    let mut ledger = anneal_ledger(&vault, clock);
    fail_reads_on_range(&vault, &event).unwrap();
    let alerts_path = vault_dir.join("alerts.jsonl");
    alert_operator(&event, &mut ledger, &alerts_path).unwrap();

    let blocked_read = read_base_value(&vault, blocked);
    assert_eq!(
        blocked_read["error"]["code"],
        json!(CALYX_ASTER_BASE_CORRUPT)
    );
    let outside_read = read_base_value(&vault, outside);
    assert_eq!(
        outside_read["value_ascii"],
        json!("issue403-outside-base-row")
    );
    let reinstall_count = install_recorded_read_barriers(&vault).unwrap();
    let operator_required =
        attempt_restore(shard.shard_id.clone(), &RestoreConfig::operator_required()).unwrap();
    let restore_failure = attempt_restore(
        shard.shard_id.clone(),
        &RestoreConfig {
            auto_restore: true,
            command: Some(RestoreCommand {
                program: "calyx-missing-restore-fsv-command".to_string(),
                args: Vec::new(),
            }),
        },
    )
    .expect_err("missing restore command fails closed");
    let blocked_alert_path = root.join("blocked-alert-path");
    fs::create_dir_all(&blocked_alert_path).unwrap();
    let alert_failure = alert_operator(&event, &mut ledger, &blocked_alert_path)
        .expect_err("directory alert path fails after ledger write");
    vault.flush().unwrap();

    let readback = json!({
        "source_of_truth": "Aster base CF bytes, anneal_checksums CF, Ledger CF, WAL, alerts.jsonl, runtime read barrier result",
        "vault": vault_dir,
        "empty_metadata_edge": empty_edge,
        "base_rows_before": base_rows_before,
        "checksum_rows_after_barrier": raw_rows(&vault, ColumnFamily::AnnealChecksums),
        "fault_event": event_json(&event),
        "blocked_read_after_barrier": blocked_read,
        "outside_read_after_barrier": outside_read,
        "reinstalled_barriers_from_checksum_cf": reinstall_count,
        "operator_required_edge": operator_required,
        "restore_failure_edge": {"code": restore_failure.code, "message": restore_failure.message},
        "alert_failure_edge": {"code": alert_failure.code, "message": alert_failure.message},
        "alerts_jsonl": fs::read_to_string(&alerts_path).unwrap(),
        "ledger_rows": ledger_rows(&vault),
        "wal_files": wal_files(&vault_dir),
        "physical_files": physical_files(&root),
    });
    let path = root.join("ph44-base-restore-readback.json");
    fs::write(&path, serde_json::to_vec_pretty(&readback).unwrap()).unwrap();
    println!("PH44_RESTORE_FSV {}", path.display());
}

fn read_base_value(vault: &AsterVault, id: CxId) -> Value {
    match vault.read_cf_at(vault.latest_seq(), ColumnFamily::Base, &base_key(id)) {
        Ok(Some(value)) => json!({
            "key_hex": hex(&base_key(id)),
            "value_ascii": ascii(&value),
            "value_sha256": sha256_hex(&value),
        }),
        Ok(None) => json!({"key_hex": hex(&base_key(id)), "missing": true}),
        Err(error) => json!({
            "key_hex": hex(&base_key(id)),
            "error": {"code": error.code, "message": error.message, "remediation": error.remediation},
        }),
    }
}

fn raw_rows(vault: &AsterVault, cf: ColumnFamily) -> Vec<Value> {
    vault
        .scan_cf_at(vault.latest_seq(), cf)
        .unwrap()
        .into_iter()
        .map(|(key, value)| {
            json!({
                "cf": cf.name(),
                "key_hex": hex(&key),
                "value_len": value.len(),
                "value_sha256": sha256_hex(&value),
                "value_ascii": ascii(&value),
            })
        })
        .collect()
}

fn ledger_rows(vault: &AsterVault) -> Vec<Value> {
    vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Ledger)
        .unwrap()
        .into_iter()
        .map(|(key, value)| {
            let entry = decode_ledger(&value).unwrap();
            assert_eq!(entry.kind, EntryKind::Anneal);
            let payload = decode_anneal_ledger_payload(&entry.payload).unwrap();
            json!({"key_hex": hex(&key), "entry_hash": hex(&entry.entry_hash), "payload": payload})
        })
        .collect()
}

fn event_json(event: &BaseFaultEvent) -> Value {
    json!({
        "shard_id": event.shard().shard_id,
        "expected_sha256": hex(&event.expected()),
        "actual_sha256": hex(&event.actual()),
        "detected_at": event.detected_at(),
    })
}

fn anneal_ledger(
    vault: &AsterVault,
    clock: FixedClock,
) -> calyx_anneal::AnnealLedger<AsterAnnealLedgerStore<'_, calyx_core::SystemClock>, FixedClock> {
    let appender = LedgerAppender::open(AsterAnnealLedgerStore::new(vault), clock).unwrap();
    calyx_anneal::AnnealLedger::new(
        appender,
        ActorId::Service("calyx-anneal-restore-fsv".to_string()),
    )
    .unwrap()
}

fn write_base(vault: &AsterVault, id: CxId, value: &[u8]) {
    vault
        .write_cf(ColumnFamily::Base, base_key(id), value.to_vec())
        .unwrap();
}

fn cx(byte: u8) -> CxId {
    CxId::from_bytes([byte; 16])
}

fn cx_range(id: CxId) -> KeyRange {
    let mut end = id.as_bytes().to_vec();
    end[15] = end[15].saturating_add(1);
    KeyRange {
        start: base_key(id),
        end: Some(end),
    }
}

fn flip(mut value: [u8; 32]) -> [u8; 32] {
    value[0] ^= 0xff;
    value
}

fn wal_files(vault: &Path) -> Vec<String> {
    let mut files = fs::read_dir(vault.join("wal"))
        .unwrap()
        .map(|entry| entry.unwrap().path().display().to_string())
        .collect::<Vec<_>>();
    files.sort();
    files
}

fn reset_dir(path: &Path) -> PathBuf {
    let _ = fs::remove_dir_all(path);
    fs::create_dir_all(path).unwrap();
    path.to_path_buf()
}

fn fsv_root() -> PathBuf {
    env::var_os("CALYX_ISSUE403_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| env::temp_dir().join("calyx-issue403-restore-fsv"))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex(&hasher.finalize())
}

fn ascii(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| {
            if byte.is_ascii_graphic() || *byte == b' ' {
                *byte as char
            } else {
                '.'
            }
        })
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
