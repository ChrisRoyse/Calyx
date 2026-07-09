#![cfg(unix)]

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use calyx_aster::{
    manifest::VaultManifest,
    vault::{AsterVault, VaultOptions},
};
use calyx_core::{
    Constellation, CxFlags, CxId, InputRef, LedgerRef, Modality, VaultId, VaultStore,
};
use serde_json::{Value, json};

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

    let before = read_manifest_assets(&root);
    assert!(
        before
            .panel_path
            .starts_with("panel/generated-no-active-panel-")
    );
    assert!(before.panel_path.ends_with(".json"));
    assert_eq!(
        before.panel_payload["kind"],
        "calyx_manifest_generated_asset_v1"
    );
    assert_eq!(before.panel_payload["asset_kind"], "panel");
    assert_eq!(before.panel_payload["status"], "no-active-panel");
    assert!(!root.join("panel/current.bin").exists());
    assert!(!root.join("codebooks/default.bin").exists());

    let panel_asset_path = root.join(&before.panel_path);
    let original_mode = fs::metadata(&panel_asset_path)
        .expect("panel asset metadata")
        .permissions()
        .mode();
    fs::set_permissions(&panel_asset_path, fs::Permissions::from_mode(0o444))
        .expect("make generated panel read-only");

    let reopened = durable_vault(&root);
    reopened.put(row(2)).expect("put reopened row");
    reopened
        .flush()
        .expect("flush without rewriting generated assets");

    let after = read_manifest_assets(&root);
    assert_eq!(before.panel_path, after.panel_path);
    assert_eq!(before.panel_hash, after.panel_hash);
    assert_eq!(before.panel_payload, after.panel_payload);
    assert_eq!(before.codebook_paths, after.codebook_paths);
    assert_eq!(before.codebook_hashes, after.codebook_hashes);
    assert_eq!(before.codebook_payloads, after.codebook_payloads);
    write_readback(&root, original_mode, &before, &after);

    let _ = fs::set_permissions(&panel_asset_path, fs::Permissions::from_mode(original_mode));
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

#[derive(Debug)]
struct ManifestAssetReadback {
    current_pointer: String,
    manifest_seq: u64,
    durable_seq: u64,
    panel_path: String,
    panel_hash: String,
    panel_payload: Value,
    codebook_paths: Vec<String>,
    codebook_hashes: Vec<String>,
    codebook_payloads: Vec<Value>,
}

fn read_manifest_assets(root: &Path) -> ManifestAssetReadback {
    let current_pointer = fs::read_to_string(root.join("CURRENT"))
        .expect("read CURRENT")
        .trim()
        .to_owned();
    let manifest_bytes =
        fs::read_to_string(root.join(&current_pointer)).expect("read current manifest");
    let manifest: VaultManifest =
        serde_json::from_str(&manifest_bytes).expect("parse current manifest");
    let panel_bytes =
        fs::read(root.join(&manifest.panel_ref.logical_path)).expect("read manifest panel asset");
    let panel_hash = blake3::hash(&panel_bytes).to_hex().to_string();
    assert_eq!(panel_hash, manifest.panel_ref.blake3_hex);
    let panel_payload: Value =
        serde_json::from_slice(&panel_bytes).expect("parse manifest panel asset");

    let mut codebook_paths = Vec::new();
    let mut codebook_hashes = Vec::new();
    let mut codebook_payloads = Vec::new();
    for codebook_ref in &manifest.codebook_refs {
        let bytes =
            fs::read(root.join(&codebook_ref.logical_path)).expect("read manifest codebook asset");
        let hash = blake3::hash(&bytes).to_hex().to_string();
        assert_eq!(hash, codebook_ref.blake3_hex);
        let payload: Value = serde_json::from_slice(&bytes).expect("parse codebook asset");
        assert_eq!(payload["kind"], "calyx_manifest_generated_asset_v1");
        assert_eq!(payload["asset_kind"], "codebook");
        assert_eq!(payload["status"], "no-active-codebook");
        codebook_paths.push(codebook_ref.logical_path.clone());
        codebook_hashes.push(hash);
        codebook_payloads.push(payload);
    }

    ManifestAssetReadback {
        current_pointer,
        manifest_seq: manifest.manifest_seq,
        durable_seq: manifest.durable_seq,
        panel_path: manifest.panel_ref.logical_path,
        panel_hash,
        panel_payload,
        codebook_paths,
        codebook_hashes,
        codebook_payloads,
    }
}

fn write_readback(
    root: &Path,
    original_mode: u32,
    before: &ManifestAssetReadback,
    after: &ManifestAssetReadback,
) {
    let readback = json!({
        "source_of_truth": "CURRENT manifest pointer plus immutable generated asset bytes",
        "before": asset_json(before),
        "after": asset_json(after),
        "panel_path_unchanged": before.panel_path == after.panel_path,
        "panel_hash_unchanged": before.panel_hash == after.panel_hash,
        "codebook_paths_unchanged": before.codebook_paths == after.codebook_paths,
        "codebook_hashes_unchanged": before.codebook_hashes == after.codebook_hashes,
        "legacy_panel_current_bin_exists": root.join("panel/current.bin").exists(),
        "legacy_codebook_default_bin_exists": root.join("codebooks/default.bin").exists(),
        "original_mode": format!("{original_mode:o}"),
        "readonly_mode_for_reopen": "444",
        "files": list_files(root),
    });
    fs::write(
        root.join("durable-manifest-assets-readback.json"),
        serde_json::to_string_pretty(&readback).unwrap(),
    )
    .expect("write durable manifest asset readback");
}

fn asset_json(readback: &ManifestAssetReadback) -> Value {
    json!({
        "current_pointer": readback.current_pointer,
        "manifest_seq": readback.manifest_seq,
        "durable_seq": readback.durable_seq,
        "panel_path": readback.panel_path,
        "panel_hash": readback.panel_hash,
        "panel_payload": readback.panel_payload,
        "codebook_paths": readback.codebook_paths,
        "codebook_hashes": readback.codebook_hashes,
        "codebook_payloads": readback.codebook_payloads,
    })
}

fn list_files(root: &Path) -> Vec<String> {
    let mut files = fs::read_dir(root)
        .expect("read root")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    files.sort();
    files
}
