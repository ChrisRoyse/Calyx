mod fsv_support;

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use calyx_aster::manifest::ManifestStore;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    AbsentReason, BoostConfig, CALYX_TEMPORAL_AP60_VIOLATION, CALYX_TEMPORAL_INVALID_PERIOD,
    CALYX_TEMPORAL_WEIGHT_SUM, Constellation, CxFlags, DecayFunction, FusionWeights, InputRef,
    LedgerRef, Modality, PeriodicOptions, SlotId, SlotVector, TemporalPolicy, VaultId, VaultStore,
};
use fsv_support::{blake3_hex, reset_dir, write_blake3_sums, write_json};
use serde_json::json;

#[test]
fn temporal_manifest_fsv_writes_vault_manifest_readbacks() {
    let (root, keep_root) = fsv_support::fsv_root(
        "CALYX_TEMPORAL_POLICY_FSV_ROOT",
        "calyx-temporal-manifest-fsv",
    );
    reset_dir(&root);
    let vault_dir = root.join("vault");
    let before_current_exists = vault_dir.join("CURRENT").exists();

    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"temporal-policy-salt",
        default_options(),
    )
    .expect("open durable vault");
    let cx = sample_constellation(&vault);
    let cx_id = cx.cx_id;
    let before_manifest_policy = read_manifest_policy(&vault_dir);

    vault.put(cx).expect("put constellation");
    vault.flush().expect("flush durable manifest");

    let current_pointer = fs::read_to_string(vault_dir.join("CURRENT")).expect("read CURRENT");
    let manifest_name = current_pointer.trim();
    let manifest_path = vault_dir.join(manifest_name);
    let manifest_bytes = fs::read(&manifest_path).expect("read pointed manifest");
    let mirror_bytes = fs::read(vault_dir.join("MANIFEST")).expect("read manifest mirror");
    let loaded = ManifestStore::open(&vault_dir)
        .load_current()
        .expect("load current manifest");
    let stored_policy = loaded.temporal_policy.expect("temporal policy persisted");
    stored_policy.validate().expect("stored policy valid");

    let reopened = AsterVault::open(
        &vault_dir,
        vault_id(),
        b"temporal-policy-salt",
        default_options(),
    )
    .expect("cold open vault");
    let reopened_manifest = ManifestStore::open(&vault_dir)
        .load_current()
        .expect("load after cold open");

    let bad_policy = TemporalPolicy {
        never_dominant: false,
        ..TemporalPolicy::default()
    };
    let invalid_dir = root.join("invalid-never-dominant-vault");
    let invalid_before_current = invalid_dir.join("CURRENT").exists();
    let invalid_create_error = AsterVault::new_durable(
        &invalid_dir,
        vault_id(),
        b"temporal-policy-salt",
        VaultOptions {
            temporal_policy: Some(bad_policy),
            ..VaultOptions::default()
        },
    )
    .expect_err("invalid temporal policy rejected");

    let invalid_weight = FusionWeights::new(0.0, 0.0, 0.0).expect_err("zero weights fail closed");
    let invalid_period =
        PeriodicOptions::new(Some(24), None).expect_err("invalid hour fails closed");
    let invalid_alpha = BoostConfig::new(0.11, 1.10, 0.85).expect_err("alpha cap fails closed");
    let custom_policy = custom_policy();
    let custom_dir = root.join("custom-policy-vault");
    let custom_vault = AsterVault::new_durable(
        &custom_dir,
        vault_id(),
        b"temporal-custom-policy-salt",
        VaultOptions {
            temporal_policy: Some(custom_policy),
            ..VaultOptions::default()
        },
    )
    .expect("custom policy vault");
    let custom_cx = sample_constellation(&custom_vault);
    custom_vault
        .put(custom_cx)
        .expect("put custom constellation");
    custom_vault.flush().expect("flush custom policy");
    let custom_first_manifest = ManifestStore::open(&custom_dir)
        .load_current()
        .expect("load custom manifest");
    let custom_reopened = AsterVault::open(
        &custom_dir,
        vault_id(),
        b"temporal-custom-policy-salt",
        VaultOptions::default(),
    )
    .expect("cold open custom policy vault");
    custom_reopened.flush().expect("second flush custom policy");
    let custom_after_second_flush = ManifestStore::open(&custom_dir)
        .load_current()
        .expect("load custom after second flush");

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
        "stored_temporal_policy": stored_policy,
        "default_temporal_policy": TemporalPolicy::default(),
        "stored_policy_is_default": stored_policy == TemporalPolicy::default(),
        "cold_open_snapshot": reopened.snapshot(),
        "cold_open_policy": reopened_manifest.temporal_policy,
        "stored_cx_id": cx_id.to_string(),
        "invalid_never_dominant": {
            "before_current_exists": invalid_before_current,
            "after_current_exists": invalid_dir.join("CURRENT").exists(),
            "error_code": invalid_create_error.code,
            "expected_error_code": CALYX_TEMPORAL_AP60_VIOLATION
        },
        "invalid_weight": {
            "error_code": invalid_weight.code,
            "expected_error_code": CALYX_TEMPORAL_WEIGHT_SUM
        },
        "invalid_period": {
            "error_code": invalid_period.code,
            "expected_error_code": CALYX_TEMPORAL_INVALID_PERIOD
        },
        "invalid_alpha": {
            "error_code": invalid_alpha.code,
            "expected_error_code": CALYX_TEMPORAL_AP60_VIOLATION
        },
        "custom_policy_cold_open": {
            "first_manifest_policy": custom_first_manifest.temporal_policy,
            "after_second_flush_policy": custom_after_second_flush.temporal_policy,
            "persisted_policy_survived_reopen": custom_after_second_flush.temporal_policy == Some(custom_policy)
        }
    });
    write_json(&root.join("temporal-manifest-readback.json"), &readback);
    write_blake3_sums(&root);

    println!("temporal_manifest_fsv_root={}", root.display());
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    assert!(!before_current_exists);
    assert_eq!(before_manifest_policy, None);
    assert_eq!(manifest_bytes, mirror_bytes);
    assert_eq!(loaded.manifest_seq, 1);
    assert_eq!(loaded.durable_seq, 1);
    assert_eq!(stored_policy, TemporalPolicy::default());
    assert_eq!(
        reopened_manifest.temporal_policy,
        Some(TemporalPolicy::default())
    );
    assert_eq!(invalid_create_error.code, CALYX_TEMPORAL_AP60_VIOLATION);
    assert!(!invalid_dir.join("CURRENT").exists());
    assert_eq!(invalid_alpha.code, CALYX_TEMPORAL_AP60_VIOLATION);
    assert_eq!(custom_first_manifest.temporal_policy, Some(custom_policy));
    assert_eq!(
        custom_after_second_flush.temporal_policy,
        Some(custom_policy)
    );

    if !keep_root {
        fs::remove_dir_all(root).expect("cleanup temp root");
    }
}

fn default_options() -> VaultOptions {
    VaultOptions::default()
}

fn custom_policy() -> TemporalPolicy {
    TemporalPolicy::new(
        true,
        DecayFunction::Linear {
            max_age_secs: 7_200,
        },
        PeriodicOptions::new(Some(14), Some(1)).expect("periodic"),
        Default::default(),
        FusionWeights::default(),
        BoostConfig::new(0.05, 1.10, 0.85).expect("boost"),
        true,
    )
    .expect("custom temporal policy")
}

fn read_manifest_policy(vault_dir: &Path) -> Option<TemporalPolicy> {
    if !vault_dir.join("CURRENT").exists() {
        return None;
    }
    ManifestStore::open(vault_dir)
        .load_current()
        .ok()
        .and_then(|manifest| manifest.temporal_policy)
}

fn sample_constellation(vault: &AsterVault) -> Constellation {
    let input = b"temporal manifest fsv input";
    let cx_id = vault.cx_id_for_input(input, 40);
    let mut input_hash = [0_u8; 32];
    input_hash[..input.len()].copy_from_slice(input);
    let mut slots = BTreeMap::new();
    slots.insert(
        SlotId::new(0),
        SlotVector::Dense {
            dim: 2,
            data: vec![0.25, 0.75],
        },
    );
    slots.insert(
        SlotId::new(1),
        SlotVector::Absent {
            reason: AbsentReason::LensUnavailable,
        },
    );
    Constellation {
        cx_id,
        vault_id: vault_id(),
        panel_version: 40,
        created_at: 1_786_233_600,
        input_ref: InputRef {
            hash: input_hash,
            pointer: Some("synthetic://ph40-temporal-policy".to_string()),
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

fn hex_prefix(bytes: &[u8], limit: usize) -> String {
    bytes
        .iter()
        .take(limit)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("valid ULID")
}
