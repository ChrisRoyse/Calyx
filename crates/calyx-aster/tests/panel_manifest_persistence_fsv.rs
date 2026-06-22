#![cfg(unix)]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use calyx_aster::manifest::ManifestStore;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Asymmetry, Constellation, CxFlags, CxId, InputRef, LedgerRef, LensId, Modality, Panel,
    QuantPolicy, Slot, SlotId, SlotKey, SlotShape, SlotState, VaultId, VaultStore,
};
use serde_json::json;

mod fsv_support;
use fsv_support::named_fsv_root_os;

#[test]
fn explicit_panel_manifest_is_json_and_survives_default_reopen_flush() {
    let (root, keep_root) = named_fsv_root_os(
        "CALYX_ISSUE752_PANEL_MANIFEST_FSV_ROOT",
        "issue752-panel-manifest",
    );
    let _ = fs::remove_dir_all(&root);
    let panel = panel();
    let options = VaultOptions {
        panel: Some(panel.clone()),
        ..VaultOptions::default()
    };

    let vault = AsterVault::new_durable(
        &root,
        vault_id(),
        b"issue752-panel-manifest".to_vec(),
        options,
    )
    .expect("open explicit-panel durable vault");
    let initial = ManifestStore::open(&root)
        .load_current()
        .expect("load initial manifest");
    assert_eq!(initial.manifest_seq, 1);
    assert_eq!(initial.durable_seq, 0);
    assert!(initial.registry_ref.is_none());
    assert!(initial.panel_ref.logical_path.starts_with("panel/panel-v"));
    assert!(initial.panel_ref.logical_path.ends_with(".json"));
    assert_eq!(decode_panel(&root, &initial.panel_ref.logical_path), panel);

    vault.put(row(1)).expect("put first row");
    vault.flush().expect("flush first row");
    let flushed = ManifestStore::open(&root)
        .load_current()
        .expect("load flushed manifest");
    assert_eq!(flushed.durable_seq, 1);
    assert_eq!(flushed.panel_ref, initial.panel_ref);
    drop(vault);

    let reopened = AsterVault::open(
        &root,
        vault_id(),
        b"issue752-panel-manifest".to_vec(),
        VaultOptions::default(),
    )
    .expect("reopen without panel option");
    reopened.put(row(2)).expect("put after reopen");
    reopened.flush().expect("flush after reopen");
    let after_reopen = ManifestStore::open(&root)
        .load_current()
        .expect("load reopened manifest");
    assert_eq!(after_reopen.durable_seq, 2);
    assert_eq!(after_reopen.panel_ref, initial.panel_ref);
    assert_eq!(
        decode_panel(&root, &after_reopen.panel_ref.logical_path),
        panel
    );

    write_readback(
        &root,
        &initial.panel_ref.logical_path,
        &after_reopen.panel_ref.logical_path,
    );
    if !keep_root {
        fs::remove_dir_all(root).expect("cleanup issue752 panel manifest test");
    }
}

fn decode_panel(root: &Path, logical_path: &str) -> Panel {
    let bytes = fs::read(root.join(logical_path)).expect("read panel asset");
    serde_json::from_slice(&bytes).expect("decode panel asset")
}

fn panel() -> Panel {
    let slot_id = SlotId::new(0);
    Panel {
        version: 7,
        slots: vec![Slot {
            slot_id,
            slot_key: SlotKey::new(slot_id, "issue752-existing"),
            lens_id: LensId::from_bytes([7; 16]),
            shape: SlotShape::Dense(16),
            modality: Modality::Text,
            asymmetry: Asymmetry::None,
            quant: QuantPolicy::None,
            resource: Default::default(),
            axis: Some("issue752".to_string()),
            retrieval_only: false,
            excluded_from_dedup: false,
            bits_about: BTreeMap::new(),
            state: SlotState::Active,
            added_at_panel_version: 7,
        }],
        created_at: 70,
        kernel_ref: None,
        guard_ref: None,
    }
}

fn row(seed: u8) -> Constellation {
    Constellation {
        cx_id: CxId::from_bytes([seed; 16]),
        vault_id: vault_id(),
        panel_version: 7,
        created_at: seed as u64,
        input_ref: InputRef {
            hash: [seed; 32],
            pointer: Some(format!("synthetic://issue752/{seed}")),
            redacted: false,
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

fn write_readback(root: &Path, initial_panel_ref: &str, final_panel_ref: &str) {
    let current = fs::read_to_string(root.join("CURRENT")).expect("read CURRENT");
    let manifest = fs::read_to_string(root.join(current.trim())).expect("read manifest");
    let panel_bytes = fs::read(root.join(final_panel_ref)).expect("read final panel bytes");
    let readback = json!({
        "source_of_truth": "CURRENT pointer, immutable manifest JSON, immutable panel JSON bytes",
        "current_pointer": current.trim(),
        "initial_panel_ref": initial_panel_ref,
        "final_panel_ref": final_panel_ref,
        "panel_ref_preserved": initial_panel_ref == final_panel_ref,
        "panel_hash": blake3::hash(&panel_bytes).to_hex().to_string(),
        "manifest_mentions_json_panel": manifest.contains(final_panel_ref),
        "panel_asset_len": panel_bytes.len(),
    });
    fs::write(
        root.join("issue752-panel-manifest-readback.json"),
        serde_json::to_vec_pretty(&readback).unwrap(),
    )
    .expect("write readback");
}
