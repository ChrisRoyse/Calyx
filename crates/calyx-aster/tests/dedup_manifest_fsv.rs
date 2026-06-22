mod fsv_support;

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use calyx_aster::dedup::{
    CALYX_DEDUP_NO_REQUIRED_SLOTS, CALYX_DEDUP_SLOT_NOT_IN_PANEL,
    CALYX_DEDUP_TEMPORAL_SLOT_IN_REQUIRED, DedupAction, DedupPolicy, TauStrategy, TctCosineConfig,
};
use calyx_aster::manifest::ManifestStore;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    AbsentReason, Asymmetry, Constellation, CxFlags, InputRef, LedgerRef, LensId, Modality, Panel,
    QuantPolicy, Slot, SlotId, SlotKey, SlotShape, SlotState, SlotVector, VaultId, VaultStore,
};
use fsv_support::{blake3_hex, reset_dir, write_blake3_sums, write_json};
use serde_json::json;

#[test]
fn dedup_manifest_fsv_writes_vault_manifest_readbacks() {
    let (root, keep_root) =
        fsv_support::fsv_root("CALYX_DEDUP_POLICY_FSV_ROOT", "calyx-dedup-manifest-fsv");
    reset_dir(&root);
    let vault_dir = root.join("vault");
    let panel = sample_panel();
    let policy = recurrence_policy();
    let before_current_exists = vault_dir.join("CURRENT").exists();
    let before_manifest_policy = read_manifest_policy(&vault_dir);

    policy
        .validate(&panel)
        .expect("policy only references content slots");
    write_json(
        &root.join("dedup-policy-input.json"),
        &json!({
            "policy": policy,
            "panel_temporal_slots": [5, 6, 7],
            "expected_required_slots_are_content": true,
            "expected_action": "RecurrenceSeries"
        }),
    );

    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"dedup-policy-salt",
        VaultOptions {
            dedup_policy: Some(policy.clone()),
            ..VaultOptions::default()
        },
    )
    .expect("open durable vault");
    let cx = sample_constellation(&vault);
    vault.put(cx).expect("put constellation");
    vault.flush().expect("flush durable manifest");

    let current_pointer = fs::read_to_string(vault_dir.join("CURRENT")).expect("read CURRENT");
    let manifest_name = current_pointer.trim();
    let manifest_path = vault_dir.join(manifest_name);
    let manifest_bytes = fs::read(&manifest_path).expect("read pointed manifest");
    let mirror_bytes = fs::read(vault_dir.join("MANIFEST")).expect("read manifest mirror");
    fs::write(
        root.join("dedup-policy-manifest.hex"),
        hex_bytes(&manifest_bytes),
    )
    .expect("write manifest hex");
    let loaded = ManifestStore::open(&vault_dir)
        .load_current()
        .expect("load current manifest");
    let stored_policy = loaded.dedup_policy.clone().expect("dedup policy persisted");

    let reopened = AsterVault::open(
        &vault_dir,
        vault_id(),
        b"dedup-policy-salt",
        VaultOptions::default(),
    )
    .expect("cold open vault");
    reopened.flush().expect("flush after cold open");
    let after_reopen_manifest = ManifestStore::open(&vault_dir)
        .load_current()
        .expect("load after cold open");

    let temporal_required = DedupPolicy::TctCosine(TctCosineConfig {
        required_slots: vec![SlotId::new(5)],
        tau: TauStrategy::Calibrated,
        action: DedupAction::RecurrenceSeries,
    });
    let temporal_required_error = temporal_required
        .validate(&panel)
        .expect_err("temporal slot rejected");
    let missing_required = DedupPolicy::TctCosine(TctCosineConfig {
        required_slots: vec![SlotId::new(9)],
        tau: TauStrategy::Calibrated,
        action: DedupAction::Link,
    });
    let missing_required_error = missing_required
        .validate(&panel)
        .expect_err("missing required slot rejected");

    let temporal_public_dir = root.join("invalid-temporal-required-vaultoptions-vault");
    let temporal_public_before_current = temporal_public_dir.join("CURRENT").exists();
    let temporal_public_before_manifest = temporal_public_dir.join("MANIFEST").exists();
    let temporal_public_error = AsterVault::new_durable(
        &temporal_public_dir,
        vault_id(),
        b"dedup-policy-salt",
        VaultOptions {
            dedup_policy: Some(temporal_required.clone()),
            panel: Some(panel.clone()),
            ..VaultOptions::default()
        },
    )
    .expect_err("temporal required slot rejected through VaultOptions");

    let missing_public_dir = root.join("invalid-missing-required-vaultoptions-vault");
    let missing_public_before_current = missing_public_dir.join("CURRENT").exists();
    let missing_public_before_manifest = missing_public_dir.join("MANIFEST").exists();
    let missing_public_error = AsterVault::new_durable(
        &missing_public_dir,
        vault_id(),
        b"dedup-policy-salt",
        VaultOptions {
            dedup_policy: Some(missing_required.clone()),
            panel: Some(panel.clone()),
            ..VaultOptions::default()
        },
    )
    .expect_err("missing required slot rejected through VaultOptions");

    let invalid_dir = root.join("invalid-empty-required-vault");
    let invalid_before_current = invalid_dir.join("CURRENT").exists();
    let empty_required_error = AsterVault::new_durable(
        &invalid_dir,
        vault_id(),
        b"dedup-policy-salt",
        VaultOptions {
            dedup_policy: Some(DedupPolicy::TctCosine(TctCosineConfig {
                required_slots: Vec::new(),
                tau: TauStrategy::Calibrated,
                action: DedupAction::Link,
            })),
            ..VaultOptions::default()
        },
    )
    .expect_err("empty required slots fail closed");

    let legacy_invalid_dir = root.join("legacy-invalid-temporal-required-vault");
    let legacy_invalid = AsterVault::new_durable(
        &legacy_invalid_dir,
        vault_id(),
        b"dedup-policy-salt",
        VaultOptions {
            dedup_policy: Some(temporal_required.clone()),
            ..VaultOptions::default()
        },
    )
    .expect("legacy invalid policy writes without panel metadata");
    let legacy_cx = sample_constellation(&legacy_invalid);
    legacy_invalid
        .put(legacy_cx)
        .expect("legacy invalid put succeeds before panel metadata is available");
    legacy_invalid.flush().expect("legacy invalid flush");
    drop(legacy_invalid);
    let legacy_invalid_before_reopen_current = legacy_invalid_dir.join("CURRENT").exists();
    let legacy_invalid_before_reopen_policy = read_manifest_policy(&legacy_invalid_dir);
    let recovered_invalid_error = AsterVault::open(
        &legacy_invalid_dir,
        vault_id(),
        b"dedup-policy-salt",
        VaultOptions {
            panel: Some(panel.clone()),
            ..VaultOptions::default()
        },
    )
    .expect_err("recovered temporal required slot rejected");
    let off_validation = DedupPolicy::Off.validate(&panel).is_ok();

    let action_json = serde_json::to_value(DedupAction::RecurrenceSeries).expect("action json");
    let readback = json!({
        "before_current_exists": before_current_exists,
        "before_manifest_policy": before_manifest_policy,
        "current_pointer": manifest_name,
        "current_manifest_path": manifest_path,
        "manifest_blake3": blake3_hex(&manifest_bytes),
        "manifest_mirror_blake3": blake3_hex(&mirror_bytes),
        "manifest_equals_mirror": manifest_bytes == mirror_bytes,
        "manifest_prefix_hex": hex_prefix(&manifest_bytes, 256),
        "loaded_manifest_seq": loaded.manifest_seq,
        "loaded_durable_seq": loaded.durable_seq,
        "stored_dedup_policy": stored_policy,
        "expected_dedup_policy": policy,
        "stored_policy_matches_expected": stored_policy == policy,
        "after_reopen_policy": after_reopen_manifest.dedup_policy,
        "after_reopen_policy_matches_expected": after_reopen_manifest.dedup_policy == Some(policy.clone()),
        "required_slots_are_content": required_slots_are_content(&panel, &policy),
        "action_json": action_json,
        "temporal_required_edge": {
            "before_required_slots": [5],
            "after_error_code": temporal_required_error.code,
            "expected_error_code": CALYX_DEDUP_TEMPORAL_SLOT_IN_REQUIRED
        },
        "missing_required_edge": {
            "before_required_slots": [9],
            "before_panel_slots": panel.slots.iter().map(|slot| slot.slot_id.get()).collect::<Vec<_>>(),
            "after_error_code": missing_required_error.code,
            "expected_error_code": CALYX_DEDUP_SLOT_NOT_IN_PANEL
        },
        "temporal_required_vaultoptions_edge": {
            "before_current_exists": temporal_public_before_current,
            "before_manifest_exists": temporal_public_before_manifest,
            "after_current_exists": temporal_public_dir.join("CURRENT").exists(),
            "after_manifest_exists": temporal_public_dir.join("MANIFEST").exists(),
            "after_vault_dir_exists": temporal_public_dir.exists(),
            "after_error_code": temporal_public_error.code,
            "expected_error_code": CALYX_DEDUP_TEMPORAL_SLOT_IN_REQUIRED
        },
        "missing_required_vaultoptions_edge": {
            "before_current_exists": missing_public_before_current,
            "before_manifest_exists": missing_public_before_manifest,
            "after_current_exists": missing_public_dir.join("CURRENT").exists(),
            "after_manifest_exists": missing_public_dir.join("MANIFEST").exists(),
            "after_vault_dir_exists": missing_public_dir.exists(),
            "after_error_code": missing_public_error.code,
            "expected_error_code": CALYX_DEDUP_SLOT_NOT_IN_PANEL
        },
        "empty_required_edge": {
            "before_current_exists": invalid_before_current,
            "after_current_exists": invalid_dir.join("CURRENT").exists(),
            "after_error_code": empty_required_error.code,
            "expected_error_code": CALYX_DEDUP_NO_REQUIRED_SLOTS
        },
        "recovered_temporal_policy_edge": {
            "before_current_exists": legacy_invalid_before_reopen_current,
            "before_manifest_policy": legacy_invalid_before_reopen_policy,
            "after_current_exists": legacy_invalid_dir.join("CURRENT").exists(),
            "after_manifest_policy": read_manifest_policy(&legacy_invalid_dir),
            "after_error_code": recovered_invalid_error.code,
            "expected_error_code": CALYX_DEDUP_TEMPORAL_SLOT_IN_REQUIRED
        },
        "off_edge": {
            "before_policy": DedupPolicy::Off,
            "after_validate_ok": off_validation
        }
    });
    write_json(&root.join("dedup-policy-readback.json"), &readback);
    write_blake3_sums(&root);

    println!("dedup_manifest_fsv_root={}", root.display());
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    assert!(!before_current_exists);
    assert_eq!(before_manifest_policy, None);
    assert_eq!(manifest_bytes, mirror_bytes);
    assert_eq!(loaded.manifest_seq, 1);
    assert_eq!(loaded.durable_seq, 1);
    assert_eq!(stored_policy, policy);
    assert_eq!(after_reopen_manifest.dedup_policy, Some(policy.clone()));
    assert!(required_slots_are_content(&panel, &policy));
    assert_eq!(action_json, json!("RecurrenceSeries"));
    assert_eq!(
        temporal_required_error.code,
        CALYX_DEDUP_TEMPORAL_SLOT_IN_REQUIRED
    );
    assert_eq!(missing_required_error.code, CALYX_DEDUP_SLOT_NOT_IN_PANEL);
    assert_eq!(
        temporal_public_error.code,
        CALYX_DEDUP_TEMPORAL_SLOT_IN_REQUIRED
    );
    assert!(!temporal_public_dir.join("CURRENT").exists());
    assert!(!temporal_public_dir.join("MANIFEST").exists());
    assert_eq!(missing_public_error.code, CALYX_DEDUP_SLOT_NOT_IN_PANEL);
    assert!(!missing_public_dir.join("CURRENT").exists());
    assert!(!missing_public_dir.join("MANIFEST").exists());
    assert_eq!(empty_required_error.code, CALYX_DEDUP_NO_REQUIRED_SLOTS);
    assert!(!invalid_dir.join("CURRENT").exists());
    assert_eq!(
        recovered_invalid_error.code,
        CALYX_DEDUP_TEMPORAL_SLOT_IN_REQUIRED
    );
    assert_eq!(
        read_manifest_policy(&legacy_invalid_dir),
        Some(temporal_required)
    );
    assert!(off_validation);

    if !keep_root {
        fs::remove_dir_all(root).expect("cleanup temp root");
    }
}

fn recurrence_policy() -> DedupPolicy {
    DedupPolicy::TctCosine(
        TctCosineConfig::new(
            vec![SlotId::new(0), SlotId::new(1)],
            TauStrategy::PerSlot(vec![(SlotId::new(0), 0.91), (SlotId::new(1), 0.88)]),
            DedupAction::RecurrenceSeries,
        )
        .expect("valid recurrence policy"),
    )
}

fn required_slots_are_content(panel: &Panel, policy: &DedupPolicy) -> bool {
    let DedupPolicy::TctCosine(config) = policy else {
        return true;
    };
    config.required_slots.iter().all(|required| {
        panel
            .slots
            .iter()
            .find(|slot| slot.slot_id == *required)
            .is_some_and(|slot| !slot.retrieval_only && !slot.excluded_from_dedup)
    })
}

fn read_manifest_policy(vault_dir: &Path) -> Option<DedupPolicy> {
    if !vault_dir.join("CURRENT").exists() {
        return None;
    }
    ManifestStore::open(vault_dir)
        .load_current()
        .ok()
        .and_then(|manifest| manifest.dedup_policy)
}

fn sample_constellation(vault: &AsterVault) -> Constellation {
    let input = b"dedup manifest fsv input";
    let cx_id = vault.cx_id_for_input(input, 41);
    let mut input_hash = [0_u8; 32];
    input_hash[..input.len()].copy_from_slice(input);
    let mut slots = BTreeMap::new();
    slots.insert(
        SlotId::new(0),
        SlotVector::Dense {
            dim: 2,
            data: vec![0.40, 0.60],
        },
    );
    slots.insert(
        SlotId::new(1),
        SlotVector::Dense {
            dim: 2,
            data: vec![0.55, 0.45],
        },
    );
    slots.insert(
        SlotId::new(5),
        SlotVector::Absent {
            reason: AbsentReason::Deferred,
        },
    );
    Constellation {
        cx_id,
        vault_id: vault_id(),
        panel_version: 41,
        created_at: 1_786_406_400,
        input_ref: InputRef {
            hash: input_hash,
            pointer: Some("synthetic://ph41-dedup-policy".to_string()),
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 0,
            hash: [0; 32],
        },
        flags: CxFlags {
            ungrounded: true,
            ..CxFlags::default()
        },
    }
}

fn sample_panel() -> Panel {
    Panel {
        version: 8,
        slots: vec![
            slot(0, "E1_semantic", false, false),
            slot(1, "keyword_splade", false, false),
            slot(5, "E2_recency", true, true),
            slot(6, "E3_periodic", true, true),
            slot(7, "E4_positional", true, true),
        ],
        created_at: 1_786_406_300,
        kernel_ref: None,
        guard_ref: None,
    }
}

fn slot(id: u16, key: &str, retrieval_only: bool, excluded_from_dedup: bool) -> Slot {
    let slot_id = SlotId::new(id);
    Slot {
        slot_id,
        slot_key: SlotKey::new(slot_id, key),
        lens_id: LensId::from_bytes([id as u8; 16]),
        shape: SlotShape::Dense(2),
        modality: Modality::Text,
        asymmetry: Asymmetry::None,
        quant: QuantPolicy::None,
        resource: Default::default(),
        axis: Some(key.to_string()),
        retrieval_only,
        excluded_from_dedup,
        bits_about: BTreeMap::new(),
        state: SlotState::Active,
        added_at_panel_version: u32::from(id) + 1,
    }
}

fn hex_prefix(bytes: &[u8], limit: usize) -> String {
    bytes
        .iter()
        .take(limit)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("valid ULID")
}
