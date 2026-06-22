use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::ledger_head::{head_anchor_path, read_head_anchor};
use calyx_aster::ledger_view::{AsterLedgerCfStore, parse_aster_ledger_seq};
use calyx_aster::sst::SstReader;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{FixedClock, VaultId};
use calyx_ledger::{
    ActorId, EntryKind, LedgerAppender, LedgerCfStore, SubjectId, VerifyResult, verify_chain,
};
mod fsv_support;
use fsv_support::{fsv_root_env_subdir, reset_dir};
use serde_json::json;
use ulid::Ulid;

#[test]
fn issue816_aster_external_head_anchor_detects_newest_sst_truncation() {
    let (root, preserve) = fsv_root_env_subdir(
        "CALYX_FSV_ROOT",
        "issue816-ledger-head-anchor",
        "calyx-issue816",
    );
    reset_dir(&root);
    let vault_dir = root.join("vault");
    let vault_id = VaultId::from_ulid(Ulid::from_bytes([0x16; 16]));
    let salt = b"issue816-ledger-head-anchor";

    let vault = AsterVault::new_durable_with_clock(
        &vault_dir,
        vault_id,
        salt.to_vec(),
        VaultOptions::default(),
        FixedClock::new(816_000),
    )
    .unwrap();
    for seq in 0..3 {
        vault
            .append_ledger_entry(
                EntryKind::Admin,
                SubjectId::Query(format!("issue816-subject-{seq}").into_bytes()),
                format!("issue816-payload-{seq}").into_bytes(),
                ActorId::Agent("issue816-fsv".to_string()),
            )
            .unwrap();
    }
    vault.flush().unwrap();
    drop(vault);

    let wal_dir = vault_dir.join("wal");
    let archived_wal_dir = root.join("wal-archived-after-flush");
    let wal_bytes_before = dir_bytes(&wal_dir);
    if wal_dir.is_dir() {
        fs::rename(&wal_dir, &archived_wal_dir).unwrap();
    }

    let anchor_path = head_anchor_path(&vault_dir);
    let anchor_bytes = fs::read(&anchor_path).unwrap();
    let anchor = read_head_anchor(&vault_dir).unwrap().unwrap();
    assert_eq!(anchor.height, 3);

    let before = AsterLedgerCfStore::open(&vault_dir).unwrap();
    let before_rows = before.scan().unwrap();
    let before_head = before_rows.last().map_or(0, |row| row.seq + 1);
    assert_eq!(before_head, 3);
    assert_eq!(
        verify_chain(&before, 0..before_head).unwrap(),
        VerifyResult::Intact { count: 3 }
    );

    let deleted_ssts = delete_ssts_containing_seq(&vault_dir, 2);
    assert!(!deleted_ssts.is_empty());

    let after = AsterLedgerCfStore::open(&vault_dir).unwrap();
    let after_rows = after.scan().unwrap();
    let after_head = after_rows.last().map_or(0, |row| row.seq + 1);
    let after_verify = verify_chain(&after, 0..after_head).unwrap();
    let recovery_error = LedgerAppender::open(after.clone(), FixedClock::new(816_999)).unwrap_err();
    assert_eq!(recovery_error.code, "CALYX_LEDGER_CHAIN_BROKEN");
    assert!(matches!(
        after_verify,
        VerifyResult::Corrupt {
            ref reason,
            ..
        } if reason.contains("anchored head 3")
    ));

    let readback = json!({
        "anchor_path": anchor_path,
        "anchor_bytes": anchor_bytes.len(),
        "anchor_blake3": blake3::hash(&anchor_bytes).to_hex().to_string(),
        "anchor_height": anchor.height,
        "anchor_tip_hash": hex(&anchor.tip_hash),
        "wal_bytes_before_archive": wal_bytes_before,
        "wal_archived": archived_wal_dir.is_dir(),
        "before_row_count": before_rows.len(),
        "before_head": before_head,
        "before_chain_intact": true,
        "deleted_sst_count": deleted_ssts.len(),
        "deleted_ssts": deleted_ssts,
        "after_row_count": after_rows.len(),
        "after_head": after_head,
        "after_verify": verify_result_name(&after_verify),
        "recovery_error_code": recovery_error.code,
        "recovery_error_mentions_end_truncated": recovery_error.to_string().contains("end-truncated"),
    });
    fs::write(
        root.join("issue816_ledger_head_anchor_readback.json"),
        serde_json::to_vec_pretty(&readback).unwrap(),
    )
    .unwrap();

    if !preserve {
        fs::remove_dir_all(&root).unwrap();
    }
}

fn delete_ssts_containing_seq(vault_dir: &Path, seq: u64) -> Vec<String> {
    let mut deleted = Vec::new();
    for path in ledger_sst_paths(vault_dir) {
        let reader = SstReader::open(&path).unwrap();
        let contains_seq = reader
            .iter()
            .unwrap()
            .iter()
            .any(|entry| parse_aster_ledger_seq(&entry.key).unwrap() == seq);
        drop(reader);
        if contains_seq {
            fs::remove_file(&path).unwrap();
            deleted.push(path.to_string_lossy().to_string());
        }
    }
    deleted
}

fn ledger_sst_paths(vault_dir: &Path) -> Vec<PathBuf> {
    let dir = vault_dir.join("cf").join("ledger");
    let mut paths = fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("sst"))
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

fn dir_bytes(path: &Path) -> u64 {
    if !path.is_dir() {
        return 0;
    }
    fs::read_dir(path)
        .unwrap()
        .map(|entry| entry.unwrap().metadata().unwrap().len())
        .sum()
}

fn verify_result_name(result: &VerifyResult) -> &'static str {
    match result {
        VerifyResult::Intact { .. } => "intact",
        VerifyResult::Broken { .. } => "broken",
        VerifyResult::Corrupt { .. } => "corrupt",
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(hex_digit(byte >> 4));
        out.push(hex_digit(byte & 0x0f));
    }
    out
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => char::from(b'0' + value),
        10..=15 => char::from(b'a' + value - 10),
        _ => unreachable!("nibble out of range"),
    }
}
