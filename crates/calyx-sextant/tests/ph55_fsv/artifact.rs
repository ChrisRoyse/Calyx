use std::fs;
use std::path::Path;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::AsterVault;
use calyx_aster::vault::encode::decode_write_batch;
use calyx_aster::wal::replay_dir;
use calyx_core::{FixedClock, SlotId};
use serde_json::{Value, json};

pub(super) fn source_of_truth(root: &Path, vault_dir: &Path) -> Value {
    json!({
        "evidence_root": root.display().to_string(),
        "vault": vault_dir.display().to_string(),
        "relational_cf": vault_dir.join("cf/relational").display().to_string(),
        "kv_cf": vault_dir.join("cf/kv").display().to_string(),
        "base_cf": vault_dir.join("cf/base").display().to_string(),
        "slot_00_cf": vault_dir.join("cf/slot_00").display().to_string(),
        "ledger_cf": vault_dir.join("cf/ledger").display().to_string(),
        "wal": vault_dir.join("wal").display().to_string(),
        "readback_json": root.join("issue467-ph55-fsv-readback.json").display().to_string()
    })
}

pub(super) fn cf_counts(vault: &AsterVault<FixedClock>) -> Value {
    let seq = vault.latest_seq();
    json!({
        "snapshot": seq,
        "relational": vault.scan_cf_at(seq, ColumnFamily::Relational).unwrap().len(),
        "kv": vault.scan_cf_at(seq, ColumnFamily::Kv).unwrap().len(),
        "base": vault.scan_cf_at(seq, ColumnFamily::Base).unwrap().len(),
        "slot_00": vault.scan_cf_at(seq, ColumnFamily::slot(SlotId::new(0))).unwrap().len(),
        "ledger": vault.scan_cf_at(seq, ColumnFamily::Ledger).unwrap().len()
    })
}

pub(super) fn wal_batches(wal_dir: &Path) -> Vec<Value> {
    replay_dir(wal_dir)
        .unwrap()
        .records
        .into_iter()
        .map(|record| {
            let rows = decode_write_batch(&record.payload).unwrap();
            json!({
                "seq": record.seq,
                "cfs": rows.iter().map(|row| row.cf.name()).collect::<Vec<_>>()
            })
        })
        .collect()
}

pub(super) fn physical_files(root: &Path) -> Vec<Value> {
    let mut files = Vec::new();
    collect_files(root, &mut files);
    files.sort_by_key(|file| file["path"].as_str().unwrap_or_default().to_string());
    files
}

fn collect_files(root: &Path, files: &mut Vec<Value>) {
    if !root.exists() {
        return;
    }
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_files(&path, files);
        } else {
            files.push(json!({
                "path": path.display().to_string(),
                "bytes": fs::metadata(&path).unwrap().len()
            }));
        }
    }
}
