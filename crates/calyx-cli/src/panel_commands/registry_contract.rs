use std::path::{Path, PathBuf};

use calyx_core::{CalyxError, Modality, SlotId, SlotShape};
use calyx_registry::{
    RegistryContractAudit, RegistryContractRepairChange, VaultRegistryContractRepairWrite,
    audit_vault_registry_contracts, load_vault_panel_state, repair_vault_registry_slot_from_spec,
};
use serde::Serialize;

use crate::error::{CliError, CliResult};
use crate::output::print_json;

#[derive(Serialize)]
struct RegistryAuditReport {
    status: &'static str,
    source_of_truth: &'static str,
    vault: PathBuf,
    checked_count: usize,
    valid: bool,
    diff_count: usize,
    audit: RegistryContractAudit,
}

#[derive(Serialize)]
struct RegistryRepairReport {
    status: &'static str,
    source_of_truth: &'static str,
    vault: PathBuf,
    manifest_seq: u64,
    durable_seq: u64,
    panel_ref: String,
    registry_ref: String,
    wrote_manifest: bool,
    old_lens_id: String,
    new_lens_id: String,
    changes: Vec<RegistryRepairChangeReport>,
    before_audit: RegistryContractAudit,
    after_audit: RegistryContractAudit,
}

#[derive(Serialize)]
struct RegistryRepairChangeReport {
    slot: u16,
    slot_key: String,
    old_lens_id: String,
    new_lens_id: String,
    old_shape: SlotShape,
    new_shape: SlotShape,
    old_modality: Modality,
    new_modality: Modality,
    reloaded_lens_id: String,
    reloaded_shape: SlotShape,
    reloaded_modality: Modality,
}

struct RegistryAuditFlags {
    vault: PathBuf,
}

struct RegistryRepairFlags {
    vault: PathBuf,
    slot: SlotId,
}

pub(super) fn registry_audit(args: &[String]) -> CliResult {
    let flags = RegistryAuditFlags::parse(args)?;
    let audit = audit_vault_registry_contracts(&flags.vault)?;
    let valid = audit.valid;
    let checked_count = audit.checked_count;
    let diff_count = audit.diffs.len();
    print_json(&RegistryAuditReport {
        status: if valid {
            "registry_contracts_valid"
        } else {
            "registry_contracts_invalid"
        },
        source_of_truth: "vault MANIFEST registry_ref plus manifest-backed registry asset read from disk",
        vault: flags.vault,
        checked_count,
        valid,
        diff_count,
        audit,
    })?;
    if valid {
        Ok(())
    } else {
        Err(CliError::from(CalyxError {
            code: "CALYX_REGISTRY_CONTRACT_DRIFT",
            message: format!(
                "persisted registry contract audit failed for {diff_count} of {checked_count} lens(es); see stdout audit report for per-lens diffs"
            ),
            remediation: "run `calyx panel registry-repair --vault <vault> --slot <slot>` after inspecting the emitted registry diff",
        }))
    }
}

pub(super) fn registry_repair(args: &[String]) -> CliResult {
    let flags = RegistryRepairFlags::parse(args)?;
    let before_audit = audit_vault_registry_contracts(&flags.vault)?;
    let write = repair_vault_registry_slot_from_spec(&flags.vault, flags.slot)?;
    let after_audit = audit_vault_registry_contracts(&flags.vault)?;
    let changes = verify_registry_repair_write(&flags.vault, &write)?;
    let valid = after_audit.valid;
    print_json(&RegistryRepairReport {
        status: if write.wrote_manifest {
            "registry_contract_repaired"
        } else {
            "registry_contract_repair_noop"
        },
        source_of_truth: "vault MANIFEST panel_ref and registry_ref plus manifest-backed JSON assets reloaded from disk",
        vault: flags.vault,
        manifest_seq: write.manifest_seq,
        durable_seq: write.durable_seq,
        panel_ref: write.panel_ref.logical_path,
        registry_ref: write.registry_ref.logical_path,
        wrote_manifest: write.wrote_manifest,
        old_lens_id: write.old_lens_id.to_string(),
        new_lens_id: write.new_lens_id.to_string(),
        changes,
        before_audit,
        after_audit,
    })?;
    if valid {
        Ok(())
    } else {
        Err(CliError::from(CalyxError {
            code: "CALYX_REGISTRY_CONTRACT_DRIFT",
            message: "registry repair completed but post-repair audit still has diffs; see stdout repair report".to_string(),
            remediation: "inspect the remaining per-lens diffs and repair each affected slot before search or probe",
        }))
    }
}

fn verify_registry_repair_write(
    vault: &Path,
    write: &VaultRegistryContractRepairWrite,
) -> CliResult<Vec<RegistryRepairChangeReport>> {
    let reloaded = load_vault_panel_state(vault)?;
    let snapshot = reloaded.registry_snapshot.as_ref().ok_or_else(|| {
        CliError::from(CalyxError::aster_corrupt_shard(format!(
            "vault {} lost registry snapshot after registry repair",
            vault.display()
        )))
    })?;
    if !snapshot
        .lenses
        .iter()
        .any(|lens| lens.lens_id == write.new_lens_id)
    {
        return Err(CliError::from(CalyxError {
            code: "CALYX_REGISTRY_CONTRACT_REPAIR_VERIFY_FAILED",
            message: format!(
                "vault {} reloaded registry is missing repaired lens {}",
                vault.display(),
                write.new_lens_id
            ),
            remediation: "inspect MANIFEST registry_ref and rerun registry-audit before search or probe",
        }));
    }
    let mut reports = Vec::with_capacity(write.changes.len());
    for change in &write.changes {
        let slot = reloaded
            .panel
            .slots
            .iter()
            .find(|slot| slot.slot_id == change.slot_id)
            .ok_or_else(|| {
                CliError::from(CalyxError {
                    code: "CALYX_REGISTRY_CONTRACT_REPAIR_VERIFY_FAILED",
                    message: format!(
                        "vault {} reloaded panel is missing repaired slot {}",
                        vault.display(),
                        change.slot_id
                    ),
                    remediation: "inspect MANIFEST panel_ref and rerun registry-audit before search or probe",
                })
            })?;
        if slot.lens_id != change.new_lens_id
            || slot.shape != change.new_shape
            || slot.modality != change.new_modality
        {
            return Err(CliError::from(CalyxError {
                code: "CALYX_REGISTRY_CONTRACT_REPAIR_VERIFY_FAILED",
                message: format!(
                    "vault {} reloaded slot {} is lens={} shape={:?} modality={:?}; expected lens={} shape={:?} modality={:?}",
                    vault.display(),
                    change.slot_id,
                    slot.lens_id,
                    slot.shape,
                    slot.modality,
                    change.new_lens_id,
                    change.new_shape,
                    change.new_modality
                ),
                remediation: "inspect MANIFEST panel_ref and registry_ref; do not rebuild search indexes until the repaired slot readback matches",
            }));
        }
        reports.push(RegistryRepairChangeReport::from_change(change, slot));
    }
    Ok(reports)
}

impl RegistryAuditFlags {
    fn parse(args: &[String]) -> CliResult<Self> {
        let mut vault = None;
        let mut idx = 0;
        while idx < args.len() {
            match args[idx].as_str() {
                "--vault" => {
                    idx += 1;
                    vault = Some(super::value(args, idx, "--vault")?.into());
                }
                other => {
                    return Err(CliError::usage(format!(
                        "unexpected panel registry-audit flag {other}"
                    )));
                }
            }
            idx += 1;
        }
        Ok(Self {
            vault: vault.ok_or_else(|| CliError::usage("panel registry-audit requires --vault"))?,
        })
    }
}

impl RegistryRepairFlags {
    fn parse(args: &[String]) -> CliResult<Self> {
        let mut vault = None;
        let mut slot = None;
        let mut idx = 0;
        while idx < args.len() {
            match args[idx].as_str() {
                "--vault" => {
                    idx += 1;
                    vault = Some(super::value(args, idx, "--vault")?.into());
                }
                "--slot" => {
                    idx += 1;
                    slot = Some(parse_slot_id(super::value(args, idx, "--slot")?)?);
                }
                other => {
                    return Err(CliError::usage(format!(
                        "unexpected panel registry-repair flag {other}"
                    )));
                }
            }
            idx += 1;
        }
        Ok(Self {
            vault: vault
                .ok_or_else(|| CliError::usage("panel registry-repair requires --vault"))?,
            slot: slot.ok_or_else(|| CliError::usage("panel registry-repair requires --slot"))?,
        })
    }
}

fn parse_slot_id(raw: &str) -> CliResult<SlotId> {
    let value = raw
        .parse::<u16>()
        .map_err(|error| CliError::usage(format!("parse --slot {raw}: {error}")))?;
    Ok(SlotId::new(value))
}

impl RegistryRepairChangeReport {
    fn from_change(change: &RegistryContractRepairChange, slot: &calyx_core::Slot) -> Self {
        Self {
            slot: change.slot_id.get(),
            slot_key: change.slot_key.clone(),
            old_lens_id: change.old_lens_id.to_string(),
            new_lens_id: change.new_lens_id.to_string(),
            old_shape: change.old_shape,
            new_shape: change.new_shape,
            old_modality: change.old_modality,
            new_modality: change.new_modality,
            reloaded_lens_id: slot.lens_id.to_string(),
            reloaded_shape: slot.shape,
            reloaded_modality: slot.modality,
        }
    }
}
