use super::*;

pub(super) fn test_vault(name: &str, panel: Panel) -> (std::path::PathBuf, ResolvedVault) {
    let root = temp_root(name);
    let vault_id = VaultId::from_ulid(Ulid::new());
    let path = root.join("vaults").join(vault_id.to_string());
    AsterVault::new_durable(
        &path,
        vault_id,
        vault_salt(vault_id, name),
        VaultOptions {
            panel: Some(panel),
            ..VaultOptions::default()
        },
    )
    .unwrap();
    (
        root,
        ResolvedVault {
            path,
            name: name.to_string(),
            vault_id,
        },
    )
}

pub(super) fn test_vault_with_registered_dense_lens(
    name: &str,
) -> (std::path::PathBuf, ResolvedVault) {
    test_vault_with_registered_dense_lens_and_panel(name, false)
}

pub(super) fn test_vault_with_registered_dense_lens_and_temporal_sidecar(
    name: &str,
) -> (std::path::PathBuf, ResolvedVault) {
    test_vault_with_registered_dense_lens_and_panel(name, true)
}

pub(super) fn test_vault_with_registered_dense_lens_and_panel(
    name: &str,
    temporal_sidecar: bool,
) -> (std::path::PathBuf, ResolvedVault) {
    let root = temp_root(name);
    let vault_id = VaultId::from_ulid(Ulid::new());
    let path = root.join("vaults").join(vault_id.to_string());
    let mut registry = Registry::new();
    let built = super::super::super::lens::build_lens(
        "algo16",
        "algorithmic",
        None,
        None,
        Some("Dense(16)"),
        Some("text"),
    )
    .unwrap();
    let lens_id = built.lens_id;
    built.register(&mut registry).unwrap();
    let panel = if temporal_sidecar {
        panel_with_text_slot_and_temporal_sidecar(lens_id, SlotShape::Dense(16))
    } else {
        panel_with_text_slot(lens_id, SlotShape::Dense(16))
    };
    AsterVault::new_durable(
        &path,
        vault_id,
        vault_salt(vault_id, name),
        VaultOptions {
            panel: Some(panel.clone()),
            ..VaultOptions::default()
        },
    )
    .unwrap();
    persist_vault_panel_state(&path, &panel, &registry).unwrap();
    (
        root,
        ResolvedVault {
            path,
            name: name.to_string(),
            vault_id,
        },
    )
}

pub(super) fn panel_with_unregistered_text_slot() -> Panel {
    panel_with_text_slot(LensId::from_bytes([7; 16]), SlotShape::Dense(3))
}

pub(super) fn panel_with_text_slot_and_temporal_sidecar(
    lens_id: LensId,
    shape: SlotShape,
) -> Panel {
    let mut panel = panel_with_text_slot(lens_id, shape);
    let slot = SlotId::new(1);
    panel.version = 2;
    panel.slots.push(Slot {
        slot_id: slot,
        slot_key: SlotKey::new(slot, "E2_recency"),
        lens_id: LensId::from_bytes([8; 16]),
        shape: SlotShape::Dense(1),
        modality: Modality::Structured,
        asymmetry: Asymmetry::None,
        quant: QuantPolicy::None,
        resource: Default::default(),
        axis: Some("E2_recency".to_string()),
        retrieval_only: true,
        excluded_from_dedup: true,
        bits_about: BTreeMap::new(),
        state: SlotState::Active,
        added_at_panel_version: 2,
    });
    panel
}

pub(super) fn panel_with_text_slot(lens_id: LensId, shape: SlotShape) -> Panel {
    let slot = SlotId::new(0);
    Panel {
        version: 1,
        slots: vec![Slot {
            slot_id: slot,
            slot_key: SlotKey::new(slot, "synthetic"),
            lens_id,
            shape,
            modality: Modality::Text,
            asymmetry: Asymmetry::None,
            quant: QuantPolicy::None,
            resource: Default::default(),
            axis: Some("synthetic".to_string()),
            retrieval_only: false,
            excluded_from_dedup: false,
            bits_about: BTreeMap::new(),
            state: SlotState::Active,
            added_at_panel_version: 1,
        }],
        created_at: 1,
        kernel_ref: None,
        guard_ref: None,
    }
}

pub(super) fn temp_root(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "calyx-cli-ingest-{name}-{}-{}",
        std::process::id(),
        now_ms()
    ))
}

pub(super) fn tokens<const N: usize>(items: [&str; N]) -> Vec<String> {
    items.into_iter().map(str::to_string).collect()
}
