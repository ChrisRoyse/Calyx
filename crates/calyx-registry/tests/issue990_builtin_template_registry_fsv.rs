use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use calyx_aster::manifest::ManifestStore;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{Input, Modality, SlotShape, SlotState, SlotVector, VaultId};
use calyx_registry::{
    civic_default, load_vault_panel_state, materialize_panel_template, media_default,
    persist_vault_panel_state, text_default,
};
use serde_json::json;

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

#[test]
fn civic_default_materializes_active_content_slots_and_registry_snapshot() {
    let materialized = materialize_panel_template(&civic_default(), 990).unwrap();
    let active = active_slot_names(&materialized.panel);

    assert_eq!(active.len(), 21);
    assert!(active.iter().all(|name| name.starts_with("polis_axis_")));
    assert_eq!(materialized.registered_lenses_added, 21);
    assert_eq!(
        materialized.inactive_unmaterialized_slots,
        vec!["E2_recency", "E3_periodic", "E4_positional"]
    );
    for slot in materialized
        .panel
        .slots
        .iter()
        .filter(|slot| slot.state == SlotState::Active)
    {
        assert!(materialized.registry.contains(slot.lens_id));
        assert_eq!(slot.shape, SlotShape::Dense(1));
        let vector = materialized
            .registry
            .measure(
                slot.lens_id,
                &Input::new(Modality::Text, b"issue990 civic probe".to_vec()),
            )
            .unwrap();
        assert!(matches!(vector, SlotVector::Dense { dim: 1, .. }));
    }
}

#[test]
fn text_default_parks_external_slots_and_keeps_local_keyword_lens_active() {
    let materialized = materialize_panel_template(&text_default(), 990).unwrap();

    assert_eq!(
        active_slot_names(&materialized.panel),
        vec!["keyword_splade"]
    );
    assert_eq!(materialized.registered_lenses_added, 1);
    assert_eq!(
        materialized.inactive_unmaterialized_slots,
        vec![
            "E1_semantic",
            "paraphrase",
            "entity",
            "causal_dual",
            "E2_recency",
            "E3_periodic",
            "E4_positional",
        ]
    );
    let keyword = materialized
        .panel
        .slots
        .iter()
        .find(|slot| slot.slot_key.key() == "keyword_splade")
        .unwrap();
    assert!(materialized.registry.contains(keyword.lens_id));
    assert!(matches!(
        materialized
            .registry
            .measure(
                keyword.lens_id,
                &Input::new(Modality::Text, b"alpha beta alpha".to_vec())
            )
            .unwrap(),
        SlotVector::Sparse { dim: 30_522, .. }
    ));
}

#[test]
fn media_default_does_not_create_active_unmaterialized_registry_placeholders() {
    let materialized = materialize_panel_template(&media_default(), 990).unwrap();

    assert!(active_slot_names(&materialized.panel).is_empty());
    assert_eq!(materialized.registered_lenses_added, 0);
    assert_eq!(
        materialized.inactive_unmaterialized_slots.len(),
        media_default().slots.len()
    );
}

#[test]
fn persisted_civic_default_reopens_with_registered_active_slots() {
    let root = temp_dir("issue990-civic-persisted");
    let materialized = materialize_panel_template(&civic_default(), 990).unwrap();
    AsterVault::new_durable(
        &root,
        vault_id(),
        b"issue990-builtins".to_vec(),
        VaultOptions {
            panel: Some(materialized.panel.clone()),
            ..VaultOptions::default()
        },
    )
    .unwrap();

    let write =
        persist_vault_panel_state(&root, &materialized.panel, &materialized.registry).unwrap();
    let manifest = ManifestStore::open(&root).load_current().unwrap();
    let loaded = load_vault_panel_state(&root).unwrap();
    let active = loaded
        .panel
        .slots
        .iter()
        .filter(|slot| slot.state == SlotState::Active)
        .collect::<Vec<_>>();

    assert_eq!(manifest.panel_ref, write.panel_ref);
    assert_eq!(manifest.registry_ref.as_ref(), Some(&write.registry_ref));
    assert!(root.join(&write.panel_ref.logical_path).is_file());
    assert!(root.join(&write.registry_ref.logical_path).is_file());
    assert_eq!(loaded.registry_snapshot.as_ref().unwrap().lenses.len(), 21);
    assert_eq!(active.len(), 21);
    assert!(
        active
            .iter()
            .all(|slot| loaded.registry.contains(slot.lens_id))
    );

    write_fsv_evidence(
        &root,
        &json!({
            "issue": 990,
            "source_of_truth": "vault CURRENT manifest, immutable panel asset, immutable registry snapshot asset, and reloaded VaultPanelState",
            "panel_ref": write.panel_ref.logical_path,
            "registry_ref": write.registry_ref.logical_path,
            "active_slot_count": active.len(),
            "registry_snapshot_lenses": loaded.registry_snapshot.as_ref().unwrap().lenses.len(),
            "inactive_unmaterialized_slots": materialized.inactive_unmaterialized_slots,
        }),
    );

    if calyx_fsv::fsv_root("CALYX_FSV_ROOT").is_none() {
        fs::remove_dir_all(root).ok();
    }
}

fn active_slot_names(panel: &calyx_core::Panel) -> Vec<&str> {
    panel
        .slots
        .iter()
        .filter(|slot| slot.state == SlotState::Active)
        .map(|slot| slot.slot_key.key())
        .collect()
}

fn write_fsv_evidence(vault_dir: &Path, value: &serde_json::Value) {
    let Some(root) = calyx_fsv::fsv_root("CALYX_FSV_ROOT") else {
        return;
    };
    let root = root.join("issue990-builtin-template-registry");
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("persisted-civic-default-readback.json"),
        serde_json::to_vec_pretty(&json!({
            "vault_dir": vault_dir.display().to_string(),
            "evidence": value,
        }))
        .unwrap(),
    )
    .unwrap();
}

fn temp_dir(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "{name}-{}-{}",
        std::process::id(),
        NEXT_DIR.fetch_add(1, Ordering::Relaxed)
    ));
    fs::remove_dir_all(&root).ok();
    fs::create_dir_all(&root).unwrap();
    root
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}
