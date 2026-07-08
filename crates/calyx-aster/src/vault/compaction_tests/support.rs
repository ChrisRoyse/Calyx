use super::super::VaultOptions;
use crate::compaction::TieringPolicy;
use calyx_core::{CxFlags, CxId, InputRef, LedgerRef, Modality, SlotId, SlotVector, VaultId};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

pub(super) fn sample_constellation(seed: u8) -> calyx_core::Constellation {
    let mut slots = BTreeMap::new();
    slots.insert(
        SlotId::new(0),
        SlotVector::Dense {
            dim: 2,
            data: vec![0.25, 0.75],
        },
    );
    calyx_core::Constellation {
        cx_id: CxId::from_bytes([seed; 16]),
        vault_id: vault_id(),
        panel_version: 7,
        created_at: 1780831800 + u64::from(seed),
        input_ref: InputRef {
            hash: [seed; 32],
            pointer: Some(format!("synthetic://issue69/{seed:02x}")),
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: u64::from(seed),
            hash: [seed.wrapping_add(1); 32],
        },
        flags: CxFlags {
            ungrounded: true,
            ..CxFlags::default()
        },
    }
}

pub(super) fn add_inactive_slot(cx: &mut calyx_core::Constellation, seed: u8) {
    cx.slots.insert(
        SlotId::new(1),
        SlotVector::Dense {
            dim: 2,
            data: vec![f32::from(seed) / 255.0, 1.0],
        },
    );
}

pub(super) fn assert_recovered_matches(
    mut expected: calyx_core::Constellation,
    got: calyx_core::Constellation,
) {
    expected.provenance = got.provenance.clone();
    assert_ne!(got.provenance.hash, [0; 32]);
    assert_eq!(got, expected);
}

pub(super) fn tiered_options(hot: &Path, archive: &Path) -> VaultOptions {
    VaultOptions {
        tiering_policy: Some(TieringPolicy::new(hot, archive, [SlotId::new(0)], 7)),
        ..VaultOptions::default()
    }
}

pub(super) fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

pub(super) fn sst_names(dir: &Path) -> Vec<String> {
    let mut names = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
        .filter(|name| name.ends_with(".sst"))
        .collect::<Vec<_>>();
    names.sort();
    names
}

pub(super) fn maybe_sst_names(dir: &Path) -> Vec<String> {
    if !dir.exists() {
        return Vec::new();
    }
    sst_names(dir)
}

pub(super) fn remove_non_compacted_ssts(dir: &Path) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if name.ends_with(".sst") && !name.starts_with("compacted-") {
            fs::remove_file(path).unwrap();
        }
    }
}

pub(super) fn write_tiered_readback(
    root: &Path,
    vault_root: &Path,
    hot: &Path,
    archive: &Path,
    compacted_path: &Path,
    manifest_bytes: &[u8],
) {
    fs::create_dir_all(root).unwrap();
    let readback = serde_json::json!({
        "vault_root": vault_root,
        "hot_root": hot,
        "archive_root": archive,
        "current_manifest": String::from_utf8_lossy(manifest_bytes),
        "hot_base_ssts": sst_names(&hot.join("cf/base")),
        "hot_active_slot_ssts": sst_names(&hot.join("cf/slot_00")),
        "archive_inactive_slot_ssts": sst_names(&archive.join("cf/slot_01")),
        "vault_inactive_slot_ssts": maybe_sst_names(&vault_root.join("cf/slot_01")),
        "compacted_inactive_slot": compacted_path,
    });
    fs::write(
        root.join("tiered-vault-readback.json"),
        serde_json::to_vec_pretty(&readback).unwrap(),
    )
    .unwrap();
}

pub(super) fn write_compacted_recovery_readback(
    root: &Path,
    vault_root: &Path,
    base_before_removal: &[String],
    slot_before_removal: &[String],
    cold_open_snapshot: u64,
    got: &calyx_core::Constellation,
) {
    fs::create_dir_all(root).unwrap();
    let current_manifest = fs::read(vault_root.join("CURRENT")).unwrap();
    let readback = serde_json::json!({
        "vault_root": vault_root,
        "current_manifest": String::from_utf8_lossy(&current_manifest),
        "base_ssts_before_removal": base_before_removal,
        "slot_ssts_before_removal": slot_before_removal,
        "base_ssts_after_removal": sst_names(&vault_root.join("cf/base")),
        "slot_ssts_after_removal": sst_names(&vault_root.join("cf/slot_00")),
        "cold_open_snapshot": cold_open_snapshot,
        "cx_id": got.cx_id.to_string(),
        "input_pointer": got.input_ref.pointer.clone(),
        "slot_count": got.slots.len(),
        "provenance_seq": got.provenance.seq,
        "provenance_hash_is_nonzero": got.provenance.hash != [0; 32],
    });
    fs::write(
        root.join("compacted-recovery-readback.json"),
        serde_json::to_vec_pretty(&readback).unwrap(),
    )
    .unwrap();
}

pub(super) fn test_dir(name: &str) -> PathBuf {
    let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "calyx-aster-vault-compaction-{name}-{}-{id}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

pub(super) fn cleanup(dir: PathBuf) {
    fs::remove_dir_all(dir).unwrap();
}
