use std::collections::BTreeMap;
use std::fs;

use calyx_aster::cf::{ColumnFamily, base_key};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{CxFlags, InputRef, LedgerRef, Modality, SlotId, SlotVector, VaultId, VaultStore};
use serde_json::Value;
use ulid::Ulid;

use super::*;

#[test]
fn parse_erase_requires_cx_id_and_accepts_fsv_out() {
    let parsed = parse_erase(&tokens([
        "mydb",
        "--cx-id",
        "00000000000000000000000000000000",
        "--fsv-out",
        "target/fsv/erase.json",
    ]))
    .unwrap();

    assert_eq!(
        parsed,
        Subcommand::Erase(EraseArgs {
            vault: "mydb".to_string(),
            cx_id: "00000000000000000000000000000000".to_string(),
            fsv_out: Some("target/fsv/erase.json".into()),
        })
    );
    assert_eq!(
        parse_erase(&tokens(["mydb"])).unwrap_err().code(),
        "CALYX_CLI_USAGE_ERROR"
    );
}

#[test]
fn erase_visible_cx_persists_tombstone_and_readback_artifact() {
    let (root, resolved, vault) = test_vault("happy");
    let cx = sample_cx(&vault, resolved.vault_id, b"erase-cli-happy");
    let cx_id = cx.cx_id;
    vault.put(cx).unwrap();
    vault.flush().unwrap();
    let fsv = root.join("fsv").join("erase.json");

    let report = erase_report(&resolved, cx_id, Some(&fsv)).unwrap();
    let reopened = open_vault(&resolved).unwrap();
    let base_after = reopened
        .read_cf_at(reopened.snapshot(), ColumnFamily::Base, &base_key(cx_id))
        .unwrap();
    let tombstone = read_tombstone(&resolved, cx_id).unwrap().unwrap();
    let fsv_value: Value = serde_json::from_slice(&fs::read(&fsv).unwrap()).unwrap();

    assert_eq!(report.status, "erased");
    assert!(report.base_visible_before);
    assert!(!report.base_visible_after);
    assert_eq!(report.slot_rows_checked_before, 2);
    assert_eq!(report.slot_rows_visible_after, 0);
    assert_eq!(report.records_deleted, 1);
    assert_eq!(report.tombstone_seq, tombstone.seq);
    assert_eq!(report.verify_chain_status, "ok");
    assert!(report.context_key_shredded);
    assert!(base_after.is_none());
    assert_eq!(fsv_value["status"], "erased");
    assert_eq!(fsv_value["cx_id"], cx_id.to_string());
    fs::remove_dir_all(root).ok();
}

#[test]
fn erase_rerun_is_idempotent_and_does_not_append_tombstone() {
    let (root, resolved, vault) = test_vault("idempotent");
    let cx = sample_cx(&vault, resolved.vault_id, b"erase-cli-idempotent");
    let cx_id = cx.cx_id;
    vault.put(cx).unwrap();
    vault.flush().unwrap();

    let first = erase_report(&resolved, cx_id, None).unwrap();
    let second = erase_report(&resolved, cx_id, None).unwrap();

    assert_eq!(first.status, "erased");
    assert_eq!(second.status, "already_tombstoned");
    assert_eq!(second.tombstone_seq, first.tombstone_seq);
    assert_eq!(second.records_deleted, 1);
    assert_eq!(erase_tombstone_count(&resolved).unwrap(), 1);
    fs::remove_dir_all(root).ok();
}

#[test]
fn erase_missing_cx_without_tombstone_fails_closed() {
    let (root, resolved, _vault) = test_vault("missing");
    let cx_id = CxId::from_bytes([0x41; 16]);

    let err = erase_report(&resolved, cx_id, None).unwrap_err();

    assert_eq!(err.code(), "CALYX_VAULT_ACCESS_DENIED");
    assert!(err.to_string().contains("has no Cx erasure tombstone"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn parse_rejects_invalid_cx_id_format() {
    let err = parse_cx_id("not-hex").unwrap_err();

    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
}

fn test_vault(name: &str) -> (std::path::PathBuf, ResolvedVault, AsterVault) {
    let root = std::env::temp_dir().join(format!(
        "calyx-cli-erase-{name}-{}-{}",
        std::process::id(),
        crate::cmd::vault::now_ms()
    ));
    let vault_id = VaultId::from_ulid(Ulid::new());
    let path = root.join("vaults").join(vault_id.to_string());
    let vault = AsterVault::new_durable(
        &path,
        vault_id,
        vault_salt(vault_id, name),
        VaultOptions::default(),
    )
    .unwrap();
    let resolved = ResolvedVault {
        path,
        name: name.to_string(),
        vault_id,
    };
    (root, resolved, vault)
}

fn sample_cx(vault: &AsterVault, vault_id: VaultId, input: &[u8]) -> calyx_core::Constellation {
    let cx_id = vault.cx_id_for_input(input, 1);
    calyx_core::Constellation {
        cx_id,
        vault_id,
        panel_version: 1,
        created_at: 11,
        input_ref: InputRef {
            hash: *blake3::hash(input).as_bytes(),
            pointer: Some("synthetic://erase-cli".to_string()),
            redacted: true,
        },
        modality: Modality::Text,
        slots: BTreeMap::from([(
            SlotId::new(0),
            SlotVector::Dense {
                dim: 2,
                data: vec![0.25, 0.75],
            },
        )]),
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 0,
            hash: [0; 32],
        },
        flags: CxFlags::default(),
    }
}

fn erase_tombstone_count(resolved: &ResolvedVault) -> CliResult<usize> {
    let store = AsterLedgerCfStore::open(&resolved.path)?;
    let mut count = 0;
    for row in store.scan()? {
        let entry = calyx_ledger::decode(&row.bytes)?;
        if calyx_ledger::tombstone_from_entry(&entry)?.is_some() {
            count += 1;
        }
    }
    Ok(count)
}

fn tokens<const N: usize>(items: [&str; N]) -> Vec<String> {
    items.into_iter().map(str::to_string).collect()
}
