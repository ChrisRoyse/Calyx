//! PH56 T04 FSV for bounded memtable backpressure.

use calyx_aster::cf::ColumnFamily;
use calyx_aster::memtable::Memtable;
use calyx_aster::resource::{ResourceStatus, VramBudgetStatus};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::VaultId;
mod fsv_support;
use fsv_support::{fsv_root_os, reset_dir};
use serde_json::json;
use std::fs;
use std::path::Path;

const CAP_BYTES: usize = 128;
const VALUE_BYTES: usize = 52;

#[test]
#[ignore = "manual FSV writes PH56 memtable source-of-truth artifacts"]
fn issue471_bounded_memtable_backpressure_fsv() {
    let root = fsv_root_os("CALYX_FSV_ROOT", "calyx-issue471-fsv");
    reset_dir(&root);
    let vault_dir = root.join("vault");
    fs::create_dir_all(&vault_dir).expect("create vault dir");
    let options = VaultOptions {
        memtable_byte_cap: CAP_BYTES,
        ..VaultOptions::default()
    };
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"issue471-memtable".to_vec(),
        options,
    )
    .expect("open durable vault");
    let row_value = vec![0x4D; VALUE_BYTES];
    let first_key = 1u64.to_be_bytes().to_vec();
    let second_key = 2u64.to_be_bytes().to_vec();
    let rejected_key = 3u64.to_be_bytes().to_vec();
    let expected_row_bytes = Memtable::entry_size(&first_key, &row_value);
    assert_eq!(expected_row_bytes, 64);

    let before = status(&vault, &vault_dir);
    assert_eq!(cf_used(&before, "base"), 0);
    let seq1 = vault
        .write_cf(ColumnFamily::Base, first_key.clone(), row_value.clone())
        .expect("first write");
    let after_one = status(&vault, &vault_dir);
    assert_eq!(seq1, 1);
    assert_eq!(cf_used(&after_one, "base"), expected_row_bytes as u64);

    let seq2 = vault
        .write_cf(ColumnFamily::Base, second_key.clone(), row_value.clone())
        .expect("second write");
    let after_flush = status(&vault, &vault_dir);
    assert_eq!(seq2, 2);
    assert_eq!(cf_used(&after_flush, "base"), 0);
    assert!(after_flush.backpressure.memtable_absorbed_total >= 1);

    let wal_before_reject = wal_bytes(&vault_dir);
    let seq_before_reject = vault.latest_seq();
    let error = vault
        .write_cf(
            ColumnFamily::Base,
            rejected_key.clone(),
            vec![0xEE; CAP_BYTES],
        )
        .expect_err("oversize write rejects");
    let after_reject = status(&vault, &vault_dir);
    assert_eq!(error.code, "CALYX_BACKPRESSURE");
    assert_eq!(vault.latest_seq(), seq_before_reject);
    assert_eq!(wal_bytes(&vault_dir), wal_before_reject);
    assert_eq!(after_reject.backpressure.memtable_rejected_total, 1);

    let metrics_after_reject = after_reject.to_metrics_text("issue471");
    fs::write(
        root.join("resource-after-reject.prom"),
        &metrics_after_reject,
    )
    .expect("write metrics");
    fs::write(
        root.join("resource-after-reject.json"),
        serde_json::to_vec_pretty(&after_reject).expect("encode resource status"),
    )
    .expect("write resource json");
    vault.flush().expect("flush accepted rows");
    drop(vault);

    let reopened = AsterVault::open(
        &vault_dir,
        vault_id(),
        b"issue471-memtable".to_vec(),
        VaultOptions::default(),
    )
    .expect("reopen durable vault");
    let reopened_seq = reopened.latest_seq();
    let first_visible = reopened
        .read_cf_at(reopened_seq, ColumnFamily::Base, &first_key)
        .expect("read first")
        .is_some();
    let second_visible = reopened
        .read_cf_at(reopened_seq, ColumnFamily::Base, &second_key)
        .expect("read second")
        .is_some();
    let rejected_visible = reopened
        .read_cf_at(reopened_seq, ColumnFamily::Base, &rejected_key)
        .expect("read rejected")
        .is_some();
    assert!(first_visible);
    assert!(second_visible);
    assert!(!rejected_visible);

    let readback = json!({
        "cap_bytes": CAP_BYTES,
        "high_water_bytes": CAP_BYTES * 4 / 5,
        "expected_base_row_bytes": expected_row_bytes,
        "before_base_used_bytes": cf_used(&before, "base"),
        "after_one_base_used_bytes": cf_used(&after_one, "base"),
        "after_flush_base_used_bytes": cf_used(&after_flush, "base"),
        "absorbed_total_after_flush": after_flush.backpressure.memtable_absorbed_total,
        "rejected_error_code": error.code,
        "seq_before_reject": seq_before_reject,
        "seq_after_reject": reopened_seq,
        "wal_bytes_before_reject": wal_before_reject,
        "wal_bytes_after_reject": wal_bytes(&vault_dir),
        "rejected_total_after_reject": after_reject.backpressure.memtable_rejected_total,
        "first_visible_after_reopen": first_visible,
        "second_visible_after_reopen": second_visible,
        "rejected_visible_after_reopen": rejected_visible,
        "sst_files": sst_files(&vault_dir),
        "metrics_file": "resource-after-reject.prom",
    });
    fs::write(
        root.join("issue471-memtable-fsv-readback.json"),
        serde_json::to_vec_pretty(&readback).expect("encode readback"),
    )
    .expect("write readback");
    println!(
        "issue471 FSV: row_bytes={expected_row_bytes} absorbed={} rejected={} reopened_seq={reopened_seq}",
        after_flush.backpressure.memtable_absorbed_total,
        after_reject.backpressure.memtable_rejected_total
    );
}

fn status(vault: &AsterVault, vault_dir: &Path) -> ResourceStatus {
    vault
        .resource_status(
            vault_dir,
            VramBudgetStatus {
                budget_bytes: 0,
                used_bytes: 0,
                probe_warning: None,
            },
        )
        .expect("resource status")
}

fn cf_used(status: &ResourceStatus, cf: &str) -> u64 {
    status
        .memtable
        .per_cf
        .iter()
        .find(|entry| entry.cf == cf)
        .map_or(0, |entry| entry.used_bytes)
}

fn wal_bytes(vault_dir: &Path) -> u64 {
    let wal = vault_dir.join("wal");
    if !wal.is_dir() {
        return 0;
    }
    fs::read_dir(wal)
        .expect("read wal dir")
        .map(|entry| {
            fs::metadata(entry.expect("wal entry").path())
                .expect("wal metadata")
                .len()
        })
        .sum()
}

fn sst_files(vault_dir: &Path) -> Vec<String> {
    let mut files = Vec::new();
    collect_ssts(&vault_dir.join("cf"), &mut files);
    files.sort();
    files
}

fn collect_ssts(dir: &Path, files: &mut Vec<String>) {
    if !dir.is_dir() {
        return;
    }
    for entry in fs::read_dir(dir).expect("read cf dir") {
        let path = entry.expect("cf entry").path();
        if path.is_dir() {
            collect_ssts(&path, files);
        } else if path.extension().and_then(|value| value.to_str()) == Some("sst") {
            files.push(path.display().to_string());
        }
    }
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("vault id")
}
