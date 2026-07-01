use calyx_aster::cf::{ColumnFamily, base_key};
use calyx_aster::vault::encode;
use calyx_core::{SlotId, SlotVector, SparseEntry, VaultId};
use serde_json::json;
use ulid::Ulid;

use super::*;

#[test]
fn streaming_rebuild_reads_physical_slot_cfs_and_validates_sidecars() {
    let root = scratch("streaming-rebuild");
    let vault_id = VaultId::from_ulid(Ulid::from_bytes([0x45; 16]));
    let salt = b"streaming-search-rebuild".to_vec();
    let vault = AsterVault::new_durable(&root, vault_id, salt, VaultOptions::default())
        .expect("open durable vault");
    let mut first = constellation(cx(51), vec![1.0, 0.0]);
    first.vault_id = vault_id;
    first.slots.insert(SlotId::new(1), sparse(16, &[(3, 1.0)]));
    first
        .slots
        .insert(SlotId::new(2), multi(2, &[&[1.0, 0.0], &[0.5, 0.5]]));
    let mut second = constellation(cx(52), vec![0.0, 1.0]);
    second.vault_id = vault_id;
    second.slots.insert(SlotId::new(1), sparse(16, &[(7, 2.0)]));
    second
        .slots
        .insert(SlotId::new(2), multi(2, &[&[0.0, 1.0]]));
    let ids = vault
        .put_batch(vec![first, second])
        .expect("write durable constellations");
    let before = physical_counts(&vault);
    let mut phases = Vec::new();

    rebuild_for_vault_with_progress(&root, &vault, |event| {
        phases.push(event.phase.to_string());
    })
    .expect("streaming rebuild");

    let indexes = PersistedSearchIndexes::open(&root).expect("open indexes");
    let after = physical_counts(&vault);
    let manifest_path = root.join("idx/search/manifest.json");
    let manifest_bytes = fs::read(&manifest_path).expect("read manifest");
    let manifest_json: serde_json::Value =
        serde_json::from_slice(&manifest_bytes).expect("manifest json");
    let entries = indexes
        .manifest
        .slots
        .iter()
        .map(|entry| (entry.slot, entry.kind.clone(), entry.len))
        .collect::<Vec<_>>();
    let dense_hits = indexes
        .search(SlotId::new(0), &dense(vec![1.0, 0.0]), 1)
        .expect("dense search");
    let sidecars = indexes
        .manifest
        .slots
        .iter()
        .filter_map(|entry| {
            entry.index_rel.as_ref().map(|rel| {
                let bytes = fs::read(root.join(rel)).expect("read sidecar");
                json!({
                    "slot": entry.slot,
                    "kind": entry.kind,
                    "rel": rel,
                    "exists": root.join(rel).is_file(),
                    "bytes": bytes.len(),
                    "sha256": sha256_hex(&bytes),
                    "manifest_sha256": entry.sha256,
                })
            })
        })
        .collect::<Vec<_>>();

    assert_eq!(before, after);
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0], (0, "flat_dense".to_string(), 2));
    assert_eq!(entries[1], (1, "sparse_inverted".to_string(), 2));
    assert_eq!(entries[2].0, 2);
    assert_eq!(dense_hits[0].cx_id, ids[0]);
    assert!(manifest_path.is_file());
    assert!(phases.contains(&"base_scan_page".to_string()));
    assert!(phases.contains(&"slot_plan_ok".to_string()));
    assert!(phases.contains(&"slot_build_start".to_string()));
    println!(
        "STREAMING_REBUILD_FSV {}",
        json!({
            "source_of_truth": "durable Aster Base/Slot CF rows plus idx/search manifest and sidecar bytes",
            "before": before,
            "after": after,
            "manifest_path": manifest_path,
            "manifest_sha256": sha256_hex(&manifest_bytes),
            "manifest_slots": manifest_json["slots"],
            "sidecars": sidecars,
            "dense_hit": dense_hits[0].cx_id.to_string(),
            "phases": phases,
        })
    );
    cleanup(root);
}

#[test]
fn missing_slot_cf_row_fails_before_manifest_swap() {
    let root = scratch("missing-slot-streaming");
    let vault_id = VaultId::from_ulid(Ulid::from_bytes([0x46; 16]));
    let salt = b"streaming-search-missing-slot".to_vec();
    let vault = AsterVault::new_durable(&root, vault_id, salt, VaultOptions::default())
        .expect("open durable vault");
    let mut broken = constellation(cx(61), vec![1.0, 0.0]);
    broken.vault_id = vault_id;
    let base = encode::encode_constellation_base(&broken).expect("encode base");
    vault
        .write_cf(ColumnFamily::Base, base_key(broken.cx_id), base)
        .expect("write base without slot row");
    let before = physical_counts(&vault);

    let err = rebuild_for_vault(&root, &vault).unwrap_err();

    let after = physical_counts(&vault);
    let manifest_path = root.join("idx/search/manifest.json");
    assert_eq!(err.code(), "CALYX_ASTER_CORRUPT_SHARD");
    assert!(err.message().contains("slot CF row missing"));
    assert_eq!(before, after);
    assert!(!manifest_path.exists());
    println!(
        "STREAMING_REBUILD_MISSING_SLOT_FSV {}",
        json!({
            "source_of_truth": "durable Aster Base/Slot CF rows and absent idx/search manifest",
            "before": before,
            "after": after,
            "manifest_exists_after": manifest_path.exists(),
            "error_code": err.code(),
            "error_message": err.message(),
        })
    );
    cleanup(root);
}

fn physical_counts(vault: &AsterVault) -> serde_json::Value {
    json!({
        "base_rows": vault.scan_cf_at(vault.latest_seq(), ColumnFamily::Base).unwrap().len(),
        "slot_0_rows": vault.scan_cf_at(vault.latest_seq(), ColumnFamily::slot(SlotId::new(0))).unwrap().len(),
        "slot_1_rows": vault.scan_cf_at(vault.latest_seq(), ColumnFamily::slot(SlotId::new(1))).unwrap().len(),
        "slot_2_rows": vault.scan_cf_at(vault.latest_seq(), ColumnFamily::slot(SlotId::new(2))).unwrap().len(),
    })
}

fn sparse(dim: u32, entries: &[(u32, f32)]) -> SlotVector {
    SlotVector::Sparse {
        dim,
        entries: entries
            .iter()
            .map(|(idx, val)| SparseEntry {
                idx: *idx,
                val: *val,
            })
            .collect(),
    }
}

fn multi(token_dim: u32, rows: &[&[f32]]) -> SlotVector {
    SlotVector::Multi {
        token_dim,
        tokens: rows.iter().map(|row| row.to_vec()).collect(),
    }
}

fn cleanup(root: std::path::PathBuf) {
    if calyx_fsv::fsv_root("CALYX_FSV_ROOT").is_none() {
        fs::remove_dir_all(root).ok();
    }
}
