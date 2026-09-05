use std::path::{Path, PathBuf};

use calyx_core::{CalyxError, Modality, SlotId, SlotShape};
use calyx_registry::{
    RegistryContractAudit, RegistryContractRepairChange, VaultRegistryContractRepairAllWrite,
    VaultRegistryContractRepairWrite, audit_vault_registry_contracts, load_vault_panel_state,
    load_vault_stored_vector_panel_state, registry_contract_audit_error,
    repair_vault_registry_contracts_from_specs, repair_vault_registry_slot_from_spec,
};
use serde::Serialize;

use crate::cmd::vault::{home_dir, resolve_vault};
use crate::error::{CliError, CliResult};
use crate::output::print_json;

#[derive(Serialize)]
struct RegistryAuditReport {
    status: &'static str,
    mode: &'static str,
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
    mode: &'static str,
    source_of_truth: &'static str,
    vault: PathBuf,
    manifest_seq: u64,
    durable_seq: u64,
    panel_ref: String,
    registry_ref: String,
    wrote_manifest: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    old_lens_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    new_lens_id: Option<String>,
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
    vault: String,
}

struct RegistryRepairFlags {
    vault: String,
    target: RegistryRepairTarget,
}

enum RegistryRepairTarget {
    Slot(SlotId),
    All,
}

pub(super) fn registry_audit(args: &[String]) -> CliResult {
    let flags = RegistryAuditFlags::parse(args)?;
    let vault = resolve_registry_vault(&flags.vault)?;
    let audit = audit_vault_registry_contracts(&vault)?;
    let valid = audit.valid;
    // The contract-drift audit compares every frozen registry lens with its
    // derivation spec, but stored-vector consumers have an additional
    // invariant: every persisted panel slot except an explicit
    // compute-on-recall sidecar must resolve to one of those authenticated
    // frozen contracts. Exercise the same loader used by weave/search before
    // reporting this vault as valid so an incomplete snapshot cannot receive
    // a false-green audit (#2012).
    if valid {
        load_vault_stored_vector_panel_state(&vault)?;
    }
    let checked_count = audit.checked_count;
    let diff_count = audit.diffs.len();
    let report_audit = audit.clone();
    print_json(&RegistryAuditReport {
        status: if valid {
            "registry_contracts_valid"
        } else {
            "registry_contracts_invalid"
        },
        mode: "audit",
        source_of_truth: "vault MANIFEST registry_ref plus manifest-backed registry asset read from disk",
        vault,
        checked_count,
        valid,
        diff_count,
        audit,
    })?;
    if valid {
        Ok(())
    } else {
        // Dispatch through the shared classifier so a drift fully explained by
        // a recorded derivation change reports as a typed migration event with
        // its policy delta. Pointing an operator at `registry-repair` for a
        // measurement-changing drift would send them at a path that now
        // (correctly) refuses them.
        Err(CliError::from(registry_contract_audit_error(&report_audit)))
    }
}

pub(super) fn registry_repair(args: &[String]) -> CliResult {
    let flags = RegistryRepairFlags::parse(args)?;
    let vault = resolve_registry_vault(&flags.vault)?;
    let before_audit = audit_vault_registry_contracts(&vault)?;
    let write = match flags.target {
        RegistryRepairTarget::Slot(slot) => {
            RegistryRepairReportInput::Slot(repair_vault_registry_slot_from_spec(&vault, slot)?)
        }
        RegistryRepairTarget::All => {
            RegistryRepairReportInput::All(repair_vault_registry_contracts_from_specs(&vault)?)
        }
    };
    let after_audit = audit_vault_registry_contracts(&vault)?;
    let changes = verify_registry_repair_write(&vault, write.changes())?;
    let valid = after_audit.valid;
    print_json(&RegistryRepairReport {
        status: if write.wrote_manifest() {
            "registry_contract_repaired"
        } else {
            "registry_contract_repair_noop"
        },
        mode: write.mode(),
        source_of_truth: "vault MANIFEST panel_ref and registry_ref plus manifest-backed JSON assets reloaded from disk",
        vault,
        manifest_seq: write.manifest_seq(),
        durable_seq: write.durable_seq(),
        panel_ref: write.panel_ref().logical_path.clone(),
        registry_ref: write.registry_ref().logical_path.clone(),
        wrote_manifest: write.wrote_manifest(),
        old_lens_id: write.old_lens_id().map(|id| id.to_string()),
        new_lens_id: write.new_lens_id().map(|id| id.to_string()),
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

fn resolve_registry_vault(reference: &str) -> CliResult<PathBuf> {
    let home = home_dir()?;
    let resolved = resolve_vault(&home, reference)?;
    resolved.canonicalize().map_err(|error| {
        CliError::io(format!(
            "canonicalize panel registry vault {}: {error}",
            resolved.display()
        ))
    })
}

enum RegistryRepairReportInput {
    Slot(VaultRegistryContractRepairWrite),
    All(VaultRegistryContractRepairAllWrite),
}

impl RegistryRepairReportInput {
    fn mode(&self) -> &'static str {
        match self {
            Self::Slot(_) => "slot",
            Self::All(_) => "all",
        }
    }

    fn manifest_seq(&self) -> u64 {
        match self {
            Self::Slot(write) => write.manifest_seq,
            Self::All(write) => write.manifest_seq,
        }
    }

    fn durable_seq(&self) -> u64 {
        match self {
            Self::Slot(write) => write.durable_seq,
            Self::All(write) => write.durable_seq,
        }
    }

    fn panel_ref(&self) -> &calyx_aster::manifest::ImmutableRef {
        match self {
            Self::Slot(write) => &write.panel_ref,
            Self::All(write) => &write.panel_ref,
        }
    }

    fn registry_ref(&self) -> &calyx_aster::manifest::ImmutableRef {
        match self {
            Self::Slot(write) => &write.registry_ref,
            Self::All(write) => &write.registry_ref,
        }
    }

    fn wrote_manifest(&self) -> bool {
        match self {
            Self::Slot(write) => write.wrote_manifest,
            Self::All(write) => write.wrote_manifest,
        }
    }

    fn changes(&self) -> &[RegistryContractRepairChange] {
        match self {
            Self::Slot(write) => &write.changes,
            Self::All(write) => &write.changes,
        }
    }

    fn old_lens_id(&self) -> Option<calyx_core::LensId> {
        match self {
            Self::Slot(write) => Some(write.old_lens_id),
            Self::All(_) => None,
        }
    }

    fn new_lens_id(&self) -> Option<calyx_core::LensId> {
        match self {
            Self::Slot(write) => Some(write.new_lens_id),
            Self::All(_) => None,
        }
    }
}

fn verify_registry_repair_write(
    vault: &Path,
    changes: &[RegistryContractRepairChange],
) -> CliResult<Vec<RegistryRepairChangeReport>> {
    let reloaded = load_vault_panel_state(vault)?;
    let snapshot = reloaded.registry_snapshot.as_ref().ok_or_else(|| {
        CliError::from(CalyxError::aster_corrupt_shard(format!(
            "vault {} lost registry snapshot after registry repair",
            vault.display()
        )))
    })?;
    for change in changes {
        if !snapshot
            .lenses
            .iter()
            .any(|lens| lens.lens_id == change.new_lens_id)
        {
            return Err(CliError::from(CalyxError {
                code: "CALYX_REGISTRY_CONTRACT_REPAIR_VERIFY_FAILED",
                message: format!(
                    "vault {} reloaded registry is missing repaired lens {}",
                    vault.display(),
                    change.new_lens_id
                ),
                remediation: "inspect MANIFEST registry_ref and rerun registry-audit before search or probe",
            }));
        }
    }
    let mut reports = Vec::with_capacity(changes.len());
    for change in changes {
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
                    vault = Some(super::value(args, idx, "--vault")?.to_string());
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
        let mut target = None;
        let mut idx = 0;
        while idx < args.len() {
            match args[idx].as_str() {
                "--vault" => {
                    idx += 1;
                    vault = Some(super::value(args, idx, "--vault")?.to_string());
                }
                "--slot" => {
                    idx += 1;
                    if target.is_some() {
                        return Err(CliError::usage(
                            "panel registry-repair accepts --slot or --all, not both",
                        ));
                    }
                    target = Some(RegistryRepairTarget::Slot(parse_slot_id(super::value(
                        args, idx, "--slot",
                    )?)?));
                }
                "--all" => {
                    if target.is_some() {
                        return Err(CliError::usage(
                            "panel registry-repair accepts --slot or --all, not both",
                        ));
                    }
                    target = Some(RegistryRepairTarget::All);
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
            target: target
                .ok_or_else(|| CliError::usage("panel registry-repair requires --slot or --all"))?,
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
