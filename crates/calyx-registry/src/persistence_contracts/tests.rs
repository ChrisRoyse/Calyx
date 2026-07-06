use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_aster::manifest::{ImmutableRef, ManifestStore};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Asymmetry, Input, Modality, Panel, QuantPolicy, Slot, SlotId, SlotKey, SlotShape, SlotState,
    SlotVector, VaultId,
};

use super::*;
use crate::frozen::{FrozenLensContract, LensDType, NormPolicy, sha256_digest};
use crate::persistence::{
    VaultRegistrySnapshot, load_vault_panel_state, persist_vault_panel_state, write_asset,
};
use crate::{
    AlgorithmicLens, DeterminismProof, LensRuntime, LensSpec, Registry, RegistryLensSnapshot,
};

#[test]
fn registry_contract_audit_detects_runtime_drift_from_persisted_json() {
    let (vault, lens_id) = test_vault_with_batch_lens("audit-drift", Some(4));
    let before_manifest = ManifestStore::open(&vault).load_current().unwrap();
    let before = manifest_registry_summary(&vault);
    println!(
        "before audit drift edge: manifest_seq={} registry_ref={:?} lenses={:?}",
        before_manifest.manifest_seq,
        before_manifest
            .registry_ref
            .as_ref()
            .map(|reference| reference.logical_path.as_str()),
        before
    );

    corrupt_first_lens_runtime_kind(&vault, "scalar");
    let after_manifest = ManifestStore::open(&vault).load_current().unwrap();
    let audit = audit_vault_registry_contracts(&vault).unwrap();
    let after = manifest_registry_summary(&vault);
    println!(
        "after audit drift edge: manifest_seq={} registry_ref={:?} valid={} diff_count={} lenses={:?}",
        after_manifest.manifest_seq,
        after_manifest
            .registry_ref
            .as_ref()
            .map(|reference| reference.logical_path.as_str()),
        audit.valid,
        audit.diffs.len(),
        after
    );

    assert!(!audit.valid);
    assert_eq!(audit.checked_count, 1);
    assert_eq!(audit.diffs.len(), 1);
    assert_eq!(audit.diffs[0].lens_id, lens_id);
    assert!(audit.diffs[0].runtime_contract_lens_id.is_some());
    assert_ne!(
        audit.diffs[0].runtime_contract_lens_id,
        Some(audit.diffs[0].persisted_contract_lens_id)
    );
    assert!(audit.diffs[0].error_code.is_none());
    assert_eq!(before.len(), 1);
    assert_eq!(after.len(), 1);
}

#[test]
fn registry_contract_repair_rewrites_manifest_backed_panel_and_registry() {
    let (vault, old_lens_id) = test_vault_with_batch_lens("repair-drift", Some(4));
    corrupt_first_lens_runtime_kind(&vault, "scalar");
    let before_manifest = ManifestStore::open(&vault).load_current().unwrap();
    let before_audit = audit_vault_registry_contracts(&vault).unwrap();
    let before_slot = manifest_panel_slot_summary(&vault, SlotId::new(0));
    let before_registry = manifest_registry_summary(&vault);
    println!(
        "before repair happy path: manifest_seq={} panel_ref={} registry_ref={:?} slot={:?} registry={:?} valid={}",
        before_manifest.manifest_seq,
        before_manifest.panel_ref.logical_path,
        before_manifest
            .registry_ref
            .as_ref()
            .map(|reference| reference.logical_path.as_str()),
        before_slot,
        before_registry,
        before_audit.valid
    );

    let write = repair_vault_registry_slot_from_spec(&vault, SlotId::new(0)).unwrap();
    let after_manifest = ManifestStore::open(&vault).load_current().unwrap();
    let after_audit = audit_vault_registry_contracts(&vault).unwrap();
    let after_state = load_vault_panel_state(&vault).unwrap();
    let after_slot = manifest_panel_slot_summary(&vault, SlotId::new(0));
    let after_registry = manifest_registry_summary(&vault);
    let measured = after_state
        .registry
        .measure(
            write.new_lens_id,
            &Input::new(Modality::Text, b"registry repair readback".to_vec()),
        )
        .unwrap();
    println!(
        "after repair happy path: manifest_seq={} panel_ref={} registry_ref={:?} wrote_manifest={} old_lens={} new_lens={} slot={:?} registry={:?} valid={} vector={:?}",
        after_manifest.manifest_seq,
        after_manifest.panel_ref.logical_path,
        after_manifest
            .registry_ref
            .as_ref()
            .map(|reference| reference.logical_path.as_str()),
        write.wrote_manifest,
        write.old_lens_id,
        write.new_lens_id,
        after_slot,
        after_registry,
        after_audit.valid,
        measured
    );

    assert!(!before_audit.valid);
    assert!(after_audit.valid);
    assert!(write.wrote_manifest);
    assert_eq!(write.old_lens_id, old_lens_id);
    assert_ne!(write.old_lens_id, write.new_lens_id);
    assert_ne!(before_manifest.panel_ref, after_manifest.panel_ref);
    assert_ne!(before_manifest.registry_ref, after_manifest.registry_ref);
    assert_eq!(after_slot.lens_id, write.new_lens_id);
    assert_eq!(after_slot.shape, SlotShape::Dense(1));
    assert!(after_state.registry.contains(write.new_lens_id));
    assert!(
        after_state
            .registry_snapshot
            .as_ref()
            .unwrap()
            .lenses
            .iter()
            .any(|lens| lens.lens_id == write.new_lens_id)
    );
    assert!(matches!(measured, SlotVector::Dense { dim: 1, .. }));
}

#[test]
fn registry_contract_repair_clean_slot_is_noop_with_state_readback() {
    let (vault, lens_id) = test_vault_with_batch_lens("repair-clean", Some(4));
    let before_manifest = ManifestStore::open(&vault).load_current().unwrap();
    let before_slot = manifest_panel_slot_summary(&vault, SlotId::new(0));
    println!(
        "before clean edge: manifest_seq={} registry_ref={:?} slot={:?}",
        before_manifest.manifest_seq,
        before_manifest
            .registry_ref
            .as_ref()
            .map(|reference| reference.logical_path.as_str()),
        before_slot
    );

    let write = repair_vault_registry_slot_from_spec(&vault, SlotId::new(0)).unwrap();
    let after_manifest = ManifestStore::open(&vault).load_current().unwrap();
    let after_slot = manifest_panel_slot_summary(&vault, SlotId::new(0));
    println!(
        "after clean edge: manifest_seq={} registry_ref={:?} wrote_manifest={} old_lens={} new_lens={} slot={:?}",
        after_manifest.manifest_seq,
        after_manifest
            .registry_ref
            .as_ref()
            .map(|reference| reference.logical_path.as_str()),
        write.wrote_manifest,
        write.old_lens_id,
        write.new_lens_id,
        after_slot
    );

    assert!(!write.wrote_manifest);
    assert_eq!(write.old_lens_id, lens_id);
    assert_eq!(write.new_lens_id, lens_id);
    assert_eq!(before_manifest.manifest_seq, after_manifest.manifest_seq);
    assert_eq!(before_manifest.registry_ref, after_manifest.registry_ref);
    assert_eq!(before_slot, after_slot);
}

#[test]
fn registry_contract_repair_missing_slot_preserves_manifest() {
    let (vault, lens_id) = test_vault_with_batch_lens("repair-missing-slot", Some(4));
    let before_manifest = ManifestStore::open(&vault).load_current().unwrap();
    let before_slot = manifest_panel_slot_summary(&vault, SlotId::new(0));
    println!(
        "before missing-slot edge: manifest_seq={} registry_ref={:?} existing_slot={:?}",
        before_manifest.manifest_seq,
        before_manifest
            .registry_ref
            .as_ref()
            .map(|reference| reference.logical_path.as_str()),
        before_slot
    );

    let error = repair_vault_registry_slot_from_spec(&vault, SlotId::new(9)).unwrap_err();
    let after_manifest = ManifestStore::open(&vault).load_current().unwrap();
    let after_slot = manifest_panel_slot_summary(&vault, SlotId::new(0));
    println!(
        "after missing-slot edge: error_code={} manifest_seq={} registry_ref={:?} existing_lens={} existing_slot={:?}",
        error.code,
        after_manifest.manifest_seq,
        after_manifest
            .registry_ref
            .as_ref()
            .map(|reference| reference.logical_path.as_str()),
        lens_id,
        after_slot
    );

    assert_eq!(error.code, REGISTRY_CONTRACT_REPAIR_INVALID);
    assert_eq!(before_manifest.manifest_seq, after_manifest.manifest_seq);
    assert_eq!(before_manifest.registry_ref, after_manifest.registry_ref);
    assert_eq!(before_slot, after_slot);
}

#[test]
fn registry_contract_audit_empty_snapshot_is_valid() {
    let vault = temp_vault_dir("audit-empty");
    let vault_id: VaultId = "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap();
    let panel = Panel {
        version: 1,
        slots: Vec::new(),
        created_at: 1,
        kernel_ref: None,
        guard_ref: None,
    };
    AsterVault::new_durable(
        &vault,
        vault_id,
        [0x6B; 32],
        VaultOptions {
            panel: Some(panel.clone()),
            ..VaultOptions::default()
        },
    )
    .unwrap();
    persist_vault_panel_state(&vault, &panel, &Registry::new()).unwrap();
    let before_manifest = ManifestStore::open(&vault).load_current().unwrap();
    println!(
        "before empty audit edge: manifest_seq={} registry_ref={:?}",
        before_manifest.manifest_seq,
        before_manifest
            .registry_ref
            .as_ref()
            .map(|reference| reference.logical_path.as_str())
    );

    let audit = audit_vault_registry_contracts(&vault).unwrap();
    let after_manifest = ManifestStore::open(&vault).load_current().unwrap();
    println!(
        "after empty audit edge: valid={} checked_count={} diff_count={} manifest_seq={} registry_ref={:?}",
        audit.valid,
        audit.checked_count,
        audit.diffs.len(),
        after_manifest.manifest_seq,
        after_manifest
            .registry_ref
            .as_ref()
            .map(|reference| reference.logical_path.as_str())
    );

    assert!(audit.valid);
    assert_eq!(audit.checked_count, 0);
    assert!(audit.diffs.is_empty());
    assert_eq!(before_manifest.registry_ref, after_manifest.registry_ref);
}

#[test]
fn gdelt_algorithmic_specs_reconstruct_runtime_contracts() {
    let cases = [
        ("gdelt-cameo", SlotShape::Dense(16)),
        ("gdelt-actor-geo", SlotShape::Sparse(512)),
        ("gdelt-source-domain", SlotShape::Sparse(512)),
        ("gdelt-event-geo", SlotShape::Sparse(512)),
        ("gdelt-actor-pair", SlotShape::Sparse(512)),
        ("gdelt-event-actor", SlotShape::Sparse(512)),
        ("gdelt-tone-signal", SlotShape::Sparse(512)),
        ("gdelt-source-event", SlotShape::Sparse(512)),
    ];

    for (kind, output) in cases {
        let spec = LensSpec {
            name: format!("persist-{kind}"),
            runtime: LensRuntime::Algorithmic {
                kind: kind.to_string(),
            },
            output,
            modality: Modality::Text,
            weights_sha256: [0; 32],
            corpus_hash: [0; 32],
            norm_policy: NormPolicy::None,
            max_batch: None,
            axis: None,
            asymmetry: Asymmetry::None,
            quant_default: QuantPolicy::turboquant_default(),
            truncate_dim: None,
            recall_delta: crate::spec::default_recall_delta(),
            retrieval_only: false,
            excluded_from_dedup: false,
        };
        let contract = derive_runtime_contract_from_spec(&spec).unwrap();
        let (lens, runtime_contract) = load_runtime_lens_from_spec(&spec).unwrap();

        println!("GDELT_PERSISTED_KIND={kind} lens={}", contract.lens_id());
        assert_eq!(contract.shape(), output);
        assert_eq!(runtime_contract.lens_id(), contract.lens_id());
        assert_eq!(lens.shape(), output);
    }
}

mod spec_hashes;

#[derive(Clone, Debug, PartialEq, Eq)]
struct RegistrySummary {
    lens_id: calyx_core::LensId,
    name: Option<String>,
    contract_lens_id: calyx_core::LensId,
    declared_contract_lens_id: Option<calyx_core::LensId>,
    runtime: Option<String>,
    output: SlotShape,
    modality: Modality,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SlotSummary {
    slot_id: SlotId,
    slot_key: String,
    lens_id: calyx_core::LensId,
    shape: SlotShape,
    modality: Modality,
}

fn corrupt_first_lens_runtime_kind(vault: &Path, kind: &str) {
    let manifest = ManifestStore::open(vault).load_current().unwrap();
    let registry_ref = manifest.registry_ref.as_ref().unwrap();
    let bytes = fs::read(vault.join(&registry_ref.logical_path)).unwrap();
    let mut snapshot: VaultRegistrySnapshot = serde_json::from_slice(&bytes).unwrap();
    let spec = snapshot
        .lenses
        .first_mut()
        .and_then(|lens| lens.spec.as_mut())
        .unwrap();
    spec.runtime = LensRuntime::Algorithmic {
        kind: kind.to_string(),
    };
    install_registry_snapshot(vault, &snapshot);
}

fn manifest_registry_summary(vault: &Path) -> Vec<RegistrySummary> {
    let manifest = ManifestStore::open(vault).load_current().unwrap();
    let registry_ref = manifest.registry_ref.as_ref().unwrap();
    let bytes = fs::read(vault.join(&registry_ref.logical_path)).unwrap();
    let snapshot: VaultRegistrySnapshot = serde_json::from_slice(&bytes).unwrap();
    snapshot
        .lenses
        .iter()
        .map(|lens| RegistrySummary {
            lens_id: lens.lens_id,
            name: lens.spec.as_ref().map(|spec| spec.name.clone()),
            contract_lens_id: lens.contract.lens_id(),
            declared_contract_lens_id: lens
                .spec
                .as_ref()
                .map(|spec| spec.declared_contract().lens_id()),
            runtime: lens.spec.as_ref().map(|spec| format!("{:?}", spec.runtime)),
            output: lens.contract.shape(),
            modality: lens.contract.modality(),
        })
        .collect()
}

fn manifest_panel_slot_summary(vault: &Path, slot_id: SlotId) -> SlotSummary {
    let manifest = ManifestStore::open(vault).load_current().unwrap();
    let bytes = fs::read(vault.join(&manifest.panel_ref.logical_path)).unwrap();
    let panel: Panel = serde_json::from_slice(&bytes).unwrap();
    let slot = panel
        .slots
        .iter()
        .find(|slot| slot.slot_id == slot_id)
        .unwrap();
    SlotSummary {
        slot_id: slot.slot_id,
        slot_key: slot.slot_key.key().to_string(),
        lens_id: slot.lens_id,
        shape: slot.shape,
        modality: slot.modality,
    }
}

fn install_registry_snapshot(vault: &Path, snapshot: &VaultRegistrySnapshot) -> ImmutableRef {
    let bytes = serde_json::to_vec_pretty(snapshot).unwrap();
    let hash = blake3::hash(&bytes).to_hex().to_string();
    let logical = format!("registry/registry-{}.json", &hash[..16]);
    write_asset(&vault.join(&logical), &bytes).unwrap();
    let registry_ref = ImmutableRef::from_bytes(logical, &bytes).unwrap();
    let store = ManifestStore::open(vault);
    let mut manifest = store.load_current().unwrap();
    manifest.manifest_seq = manifest.manifest_seq.checked_add(1).unwrap();
    manifest.registry_ref = Some(registry_ref.clone());
    manifest.validate().unwrap();
    store.write_current(&manifest).unwrap();
    registry_ref
}

fn test_vault_with_batch_lens(
    name: &str,
    max_batch: Option<usize>,
) -> (PathBuf, calyx_core::LensId) {
    let vault = temp_vault_dir(name);
    let vault_id: VaultId = "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap();
    let mut registry = Registry::new();
    let lens = AlgorithmicLens::byte_features("batch-limit-lens", Modality::Text);
    let contract = lens.contract().clone();
    let lens_id = contract.lens_id();
    let spec = LensSpec {
        name: contract.name().to_string(),
        runtime: LensRuntime::Algorithmic {
            kind: "byte-features".to_string(),
        },
        output: contract.shape(),
        modality: contract.modality(),
        weights_sha256: contract.weights_sha256(),
        corpus_hash: contract.corpus_hash(),
        norm_policy: contract.norm_policy(),
        max_batch,
        axis: None,
        asymmetry: Asymmetry::None,
        quant_default: QuantPolicy::turboquant_default(),
        truncate_dim: None,
        recall_delta: crate::spec::default_recall_delta(),
        retrieval_only: false,
        excluded_from_dedup: false,
    };
    registry
        .register_frozen_with_spec(lens, contract, spec)
        .unwrap();
    let panel = panel_with_lens(lens_id);
    AsterVault::new_durable(
        &vault,
        vault_id,
        [0x5A; 32],
        VaultOptions {
            panel: Some(panel.clone()),
            ..VaultOptions::default()
        },
    )
    .unwrap();
    persist_vault_panel_state(&vault, &panel, &registry).unwrap();
    (vault, lens_id)
}

fn panel_with_lens(lens_id: calyx_core::LensId) -> Panel {
    panel_with_lens_shape(lens_id, SlotShape::Dense(16))
}

fn panel_with_lens_shape(lens_id: calyx_core::LensId, shape: SlotShape) -> Panel {
    let slot = SlotId::new(0);
    Panel {
        version: 1,
        slots: vec![Slot {
            slot_id: slot,
            slot_key: SlotKey::new(slot, "batch-limit-lens"),
            lens_id,
            shape,
            modality: Modality::Text,
            asymmetry: Asymmetry::None,
            quant: QuantPolicy::None,
            resource: Default::default(),
            axis: Some("batch-limit-lens".to_string()),
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

fn temp_vault_dir(name: &str) -> PathBuf {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "calyx-registry-contract-{name}-{}-{now}",
        std::process::id()
    ))
}
