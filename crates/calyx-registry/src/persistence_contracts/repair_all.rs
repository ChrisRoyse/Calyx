use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use calyx_aster::manifest::ImmutableRef;
use calyx_core::{CalyxError, LensId, Panel, Result};

use super::{
    RegistryContractRepairChange, audit_registry_snapshot_contracts,
    derive_runtime_contract_from_spec, registry_contract_drift_error,
    registry_contract_repair_invalid, spec_from_runtime_contract,
};
use crate::RegistryLensSnapshot;
use crate::persistence::{
    VaultRegistrySnapshot, load_manifest_panel_registry_snapshot, persist_vault_panel_state,
    rebuild_registry,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VaultRegistryContractRepairAllWrite {
    pub manifest_seq: u64,
    pub durable_seq: u64,
    pub panel_ref: ImmutableRef,
    pub registry_ref: ImmutableRef,
    pub wrote_manifest: bool,
    pub changes: Vec<RegistryContractRepairChange>,
}

pub fn repair_vault_registry_contracts_from_specs(
    vault_dir: impl AsRef<Path>,
) -> Result<VaultRegistryContractRepairAllWrite> {
    let vault_dir = vault_dir.as_ref();
    let (manifest, mut panel, mut snapshot) = load_manifest_panel_registry_snapshot(vault_dir)?;
    let audit = audit_registry_snapshot_contracts(&snapshot);
    if audit.valid {
        let registry_ref = manifest.registry_ref.clone().ok_or_else(|| {
            CalyxError::aster_corrupt_shard(
                "vault manifest has no registry_ref after loading registry snapshot",
            )
        })?;
        return Ok(VaultRegistryContractRepairAllWrite {
            manifest_seq: manifest.manifest_seq,
            durable_seq: manifest.durable_seq,
            panel_ref: manifest.panel_ref,
            registry_ref,
            wrote_manifest: false,
            changes: Vec::new(),
        });
    }
    let targets = audit
        .diffs
        .iter()
        .map(|diff| diff.lens_id)
        .collect::<BTreeSet<_>>();
    let changes = repair_registry_lens_set(&mut panel, &mut snapshot, &targets)?;
    let post_audit = audit_registry_snapshot_contracts(&snapshot);
    if !post_audit.valid {
        return Err(registry_contract_drift_error(&post_audit));
    }
    let registry = rebuild_registry(&snapshot)?;
    let write = persist_vault_panel_state(vault_dir, &panel, &registry)?;
    Ok(VaultRegistryContractRepairAllWrite {
        manifest_seq: write.manifest_seq,
        durable_seq: write.durable_seq,
        panel_ref: write.panel_ref,
        registry_ref: write.registry_ref,
        wrote_manifest: true,
        changes,
    })
}

fn repair_registry_lens_set(
    panel: &mut Panel,
    snapshot: &mut VaultRegistrySnapshot,
    targets: &BTreeSet<LensId>,
) -> Result<Vec<RegistryContractRepairChange>> {
    let mut replacements = BTreeMap::new();
    for (idx, lens) in snapshot.lenses.iter().enumerate() {
        if !targets.contains(&lens.lens_id) {
            continue;
        }
        let spec = lens.spec.as_ref().ok_or_else(|| {
            registry_contract_repair_invalid(format!(
                "lens {} has no LensSpec; cannot reconstruct a replacement contract",
                lens.lens_id
            ))
        })?;
        let runtime_contract = derive_runtime_contract_from_spec(spec)?;
        replacements.insert(
            idx,
            RegistryLensSnapshot {
                lens_id: runtime_contract.lens_id(),
                contract: runtime_contract.clone(),
                spec: Some(spec_from_runtime_contract(spec.clone(), &runtime_contract)),
                determinism: lens.determinism,
                runtime_golden: if runtime_contract.lens_id() == lens.lens_id {
                    lens.runtime_golden.clone()
                } else {
                    None
                },
            },
        );
    }
    if replacements.len() != targets.len() {
        return Err(registry_contract_repair_invalid(format!(
            "repair target set contains {} lens(es), but {} were found in the registry",
            targets.len(),
            replacements.len()
        )));
    }
    let mut final_ids = BTreeSet::new();
    for (idx, lens) in snapshot.lenses.iter().enumerate() {
        let final_id = replacements
            .get(&idx)
            .map(|replacement| replacement.lens_id)
            .unwrap_or(lens.lens_id);
        if !final_ids.insert(final_id) {
            return Err(registry_contract_repair_invalid(format!(
                "registry repair would create duplicate lens id {final_id}"
            )));
        }
    }
    let old_to_new = replacements
        .iter()
        .map(|(idx, replacement)| (snapshot.lenses[*idx].lens_id, replacement.clone()))
        .collect::<Vec<_>>();
    for (idx, replacement) in replacements {
        snapshot.lenses[idx] = replacement;
    }
    let mut changes = Vec::new();
    for (old_lens_id, replacement) in old_to_new {
        for slot in panel
            .slots
            .iter_mut()
            .filter(|slot| slot.lens_id == old_lens_id)
        {
            let change = RegistryContractRepairChange {
                slot_id: slot.slot_id,
                slot_key: slot.slot_key.key().to_string(),
                old_lens_id,
                new_lens_id: replacement.lens_id,
                old_shape: slot.shape,
                new_shape: replacement.contract.shape(),
                old_modality: slot.modality,
                new_modality: replacement.contract.modality(),
            };
            slot.lens_id = replacement.lens_id;
            slot.shape = replacement.contract.shape();
            slot.modality = replacement.contract.modality();
            changes.push(change);
        }
    }
    Ok(changes)
}
