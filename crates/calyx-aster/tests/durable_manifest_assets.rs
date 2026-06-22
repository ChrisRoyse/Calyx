#![cfg(unix)]

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Constellation, CxFlags, CxId, InputRef, LedgerRef, Modality, VaultId, VaultStore,
};
use serde_json::json;

mod fsv_support;
use fsv_support::named_fsv_root_os;

#[test]
fn durable_manifest_assets_are_not_rewritten_on_reopen() {
    let (root, keep_root) = named_fsv_root_os(
        "CALYX_DURABLE_MANIFEST_ASSETS_FSV_ROOT",
        "durable-manifest-assets",
    );
    let _ = fs::remove_dir_all(&root);

    let vault = durable_vault(&root);
    vault.put(row(1)).expect("put initial row");
    vault.flush().expect("flush initial manifest");

    let panel_path = root.join("panel/current.bin");
    let before = fs::read(&panel_path).expect("read panel before");
    let before_hash = blake3::hash(&before).to_hex().to_string();
    let original_mode = fs::metadata(&panel_path)
        .expect("panel metadata")
        .permissions()
        .mode();
    fs::set_permissions(&panel_path, fs::Permissions::from_mode(0o444))
        .expect("make panel read-only");

    let reopened = durable_vault(&root);
    reopened.put(row(2)).expect("put reopened row");
    reopened.flush().expect("flush without rewriting panel");

    let after = fs::read(&panel_path).expect("read panel after");
    let after_hash = blake3::hash(&after).to_hex().to_string();
    assert_eq!(before, after);
    assert_eq!(before_hash, after_hash);
    write_readback(&root, original_mode, before_hash, after_hash);

    let _ = fs::set_permissions(&panel_path, fs::Permissions::from_mode(original_mode));
    if !keep_root {
        fs::remove_dir_all(root).expect("cleanup durable manifest asset test");
    }
}

fn durable_vault(dir: &PathBuf) -> AsterVault {
    AsterVault::new_durable(
        dir,
        vault_id(),
        b"durable-manifest-assets".to_vec(),
        VaultOptions::default(),
    )
    .expect("open durable vault")
}

fn row(seed: u8) -> Constellation {
    Constellation {
        cx_id: CxId::from_bytes([seed; 16]),
        vault_id: vault_id(),
        panel_version: 1,
        created_at: seed as u64,
        input_ref: InputRef {
            hash: [seed; 32],
            pointer: None,
            redacted: true,
        },
        modality: Modality::Text,
        slots: BTreeMap::new(),
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: seed as u64,
            hash: [seed; 32],
        },
        flags: CxFlags::default(),
    }
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("vault id")
}

fn write_readback(root: &PathBuf, original_mode: u32, before_hash: String, after_hash: String) {
    let current = fs::read_to_string(root.join("CURRENT")).expect("read CURRENT");
    let manifest = fs::read_to_string(root.join(current.trim())).expect("read manifest");
    let readback = json!({
        "source_of_truth": "panel/current.bin bytes plus CURRENT manifest pointer",
        "panel_path": root.join("panel/current.bin"),
        "before_hash": before_hash,
        "after_hash": after_hash,
        "hash_unchanged": before_hash == after_hash,
        "original_mode": format!("{original_mode:o}"),
        "readonly_mode_for_reopen": "444",
        "current_pointer": current.trim(),
        "manifest_contains_panel_ref": manifest.contains("panel/current.bin"),
        "files": list_files(root),
    });
    fs::write(
        root.join("durable-manifest-assets-readback.json"),
        serde_json::to_string_pretty(&readback).unwrap(),
    )
    .expect("write durable manifest asset readback");
}

fn list_files(root: &PathBuf) -> Vec<String> {
    let mut files = fs::read_dir(root)
        .expect("read root")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    files.sort();
    files
}
