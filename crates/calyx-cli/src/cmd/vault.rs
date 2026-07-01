use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{Asymmetry, CalyxError, Panel, SlotId, SlotState, VaultId};
use calyx_registry::{
    PanelTemplate, Registry, SlotSpec, SwapController, bio_default, civic_default, code_default,
    legal_default, list_panel, load_vault_panel_state, materialize_panel_template, media_default,
    medical_default, persist_vault_panel_state, profile_lens, text_default,
};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

use super::lens::{build_lens, built_modality, profile_probes};
use super::{
    AddLensArgs, CreateVaultArgs, ProfileLensArgs, SlotCommandArgs, Subcommand, VaultRefArgs,
    vault_retire,
};
use crate::error::{CliError, CliResult};
use crate::output::{print_json, print_table};
use crate::panel_commands::{build_saved_template_panel, saved_template_names};

const DEFAULT_TEMPLATE: &str = "text-default";
const DEFAULT_PROFILE_NAME: &str = "profile-lens";
pub(crate) fn run(command: Subcommand) -> CliResult {
    match command {
        Subcommand::CreateVault(args) => create_vault(args),
        Subcommand::AddLens(args) => add_lens(args),
        Subcommand::RetireLens(args) => set_lens_state(args, LensStateAction::Retire),
        Subcommand::ParkLens(args) => set_lens_state(args, LensStateAction::Park),
        Subcommand::RetireVault(args) => vault_retire::run(args),
        Subcommand::ListPanel(args) => list_panel_command(args),
        Subcommand::ProfileLens(args) => profile_lens_command(args),
        _ => unreachable!("non-vault command routed to vault module"),
    }
}

#[derive(Serialize)]
struct CreateVaultReport {
    vault_id: String,
    name: String,
    panel_template: String,
    template_source: String,
    content_lens_count: usize,
    registered_lenses_added: usize,
    registry_snapshot_written: bool,
    inactive_unmaterialized_slots: Vec<String>,
    a37_gate_eligible: bool,
    a37_status: String,
}

#[derive(Serialize)]
struct AddLensReport {
    lens_id: String,
    slot_id: u16,
    name: String,
}

#[derive(Serialize)]
struct LifecycleReport {
    status: &'static str,
    slot: u16,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct VaultIndex {
    vaults: Vec<VaultIndexEntry>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    retired_vaults: Vec<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct VaultIndexEntry {
    name: String,
    vault_id: VaultId,
    path: String,
    panel_template: String,
}

#[derive(Clone, Debug)]
pub(crate) struct ResolvedVault {
    pub path: PathBuf,
    pub name: String,
    pub vault_id: VaultId,
}

#[derive(Clone, Copy)]
enum LensStateAction {
    Retire,
    Park,
}

struct PreparedVaultPanel {
    panel: Panel,
    registry: Registry,
    template_source: String,
    content_lens_count: usize,
    registered_lenses_added: usize,
    inactive_unmaterialized_slots: Vec<String>,
    a37_gate_eligible: bool,
    a37_status: String,
}

fn create_vault(args: CreateVaultArgs) -> CliResult {
    let home = home_dir()?;
    let template = args.panel_template.as_deref().unwrap_or(DEFAULT_TEMPLATE);
    super::validate_panel_template_name(template)?;
    let mut index = read_index(&home)?;
    if index.vaults.iter().any(|entry| entry.name == args.name) {
        return Err(CliError::usage(format!(
            "vault name {} already exists",
            args.name
        )));
    }

    let prepared = prepare_vault_panel(&home, template)?;
    let vault_id = VaultId::from_ulid(Ulid::new());
    let relative = format!("vaults/{vault_id}");
    let vault_dir = home.join(&relative);
    if vault_dir.exists() {
        return Err(CliError::usage(format!(
            "vault directory for {vault_id} already exists"
        )));
    }
    let options = VaultOptions {
        panel: Some(prepared.panel.clone()),
        ..VaultOptions::default()
    };
    AsterVault::new_durable(
        &vault_dir,
        vault_id,
        vault_salt(vault_id, &args.name),
        options,
    )?;
    persist_vault_panel_state(&vault_dir, &prepared.panel, &prepared.registry)?;
    let registry_snapshot_written = true;

    index.vaults.push(VaultIndexEntry {
        name: args.name.clone(),
        vault_id,
        path: relative,
        panel_template: template.to_string(),
    });
    index
        .vaults
        .sort_by(|left, right| left.name.cmp(&right.name));
    write_index(&home, &index)?;
    print_json(&CreateVaultReport {
        vault_id: vault_id.to_string(),
        name: args.name,
        panel_template: template.to_string(),
        template_source: prepared.template_source,
        content_lens_count: prepared.content_lens_count,
        registered_lenses_added: prepared.registered_lenses_added,
        registry_snapshot_written,
        inactive_unmaterialized_slots: prepared.inactive_unmaterialized_slots,
        a37_gate_eligible: prepared.a37_gate_eligible,
        a37_status: prepared.a37_status,
    })
}

fn add_lens(args: AddLensArgs) -> CliResult {
    let home = home_dir()?;
    let vault_dir = resolve_vault(&home, &args.vault)?;
    let mut state = load_vault_panel_state(&vault_dir)?;
    let built = build_lens(
        &args.name,
        &args.runtime,
        args.endpoint.as_deref(),
        args.weights.as_deref(),
        args.shape.as_deref(),
        args.modality.as_deref(),
    )?;
    let lens_id = built.lens_id;
    let shape = built.spec.output;
    let modality = built.spec.modality;
    let quant = built.spec.quant_default;
    if !state.registry.contains(lens_id) {
        built.register(&mut state.registry)?;
    }

    let mut controller = SwapController::new(state.panel);
    let outcome = controller.add_lens(
        &state.registry,
        SlotSpec {
            key: args.name.clone(),
            lens_id,
            shape,
            modality,
            asymmetry: Asymmetry::None,
            quant,
            axis: Some(args.name.clone()),
            retrieval_only: false,
            excluded_from_dedup: false,
        },
        [],
        now_ms(),
    )?;
    persist_vault_panel_state(&vault_dir, controller.panel(), &state.registry)?;
    print_json(&AddLensReport {
        lens_id: lens_id.to_string(),
        slot_id: outcome.slot.slot_id.get(),
        name: args.name,
    })
}

fn set_lens_state(args: SlotCommandArgs, action: LensStateAction) -> CliResult {
    let home = home_dir()?;
    let vault_dir = resolve_vault(&home, &args.vault)?;
    let state = load_vault_panel_state(&vault_dir)?;
    let slot = SlotId::new(args.slot);
    ensure_slot_can_transition(&state.panel, slot, action)?;
    let mut controller = SwapController::new(state.panel);
    match action {
        LensStateAction::Retire => {
            controller.retire_lens(slot, now_ms())?;
            persist_vault_panel_state(&vault_dir, controller.panel(), &state.registry)?;
            print_json(&LifecycleReport {
                status: "retired",
                slot: args.slot,
            })
        }
        LensStateAction::Park => {
            controller.park_lens(slot, now_ms())?;
            persist_vault_panel_state(&vault_dir, controller.panel(), &state.registry)?;
            print_json(&LifecycleReport {
                status: "parked",
                slot: args.slot,
            })
        }
    }
}

fn ensure_slot_can_transition(panel: &Panel, slot: SlotId, action: LensStateAction) -> CliResult {
    let current = panel
        .slots
        .iter()
        .find(|candidate| candidate.slot_id == slot)
        .map(|candidate| candidate.state)
        .ok_or_else(|| CalyxError::lens_frozen_violation(format!("slot {slot} is not in panel")))?;
    let target = match action {
        LensStateAction::Retire => SlotState::Retired,
        LensStateAction::Park => SlotState::Parked,
    };
    if current == target {
        return Err(CalyxError::lens_frozen_violation(format!(
            "slot {slot} is already {}",
            format!("{target:?}").to_ascii_lowercase()
        ))
        .into());
    }
    Ok(())
}

fn list_panel_command(args: VaultRefArgs) -> CliResult {
    let home = home_dir()?;
    let vault_dir = resolve_vault(&home, &args.vault)?;
    let state = load_vault_panel_state(&vault_dir)?;
    let rows = list_panel(&state.panel, &state.registry)
        .into_iter()
        .map(|slot| {
            vec![
                slot.slot_id.get().to_string(),
                slot.key,
                format!("{:?}", slot.state).to_ascii_lowercase(),
                slot.bits_about
                    .map(|bits| format!("{bits:.6}"))
                    .unwrap_or_default(),
                String::new(),
                String::new(),
            ]
        })
        .collect::<Vec<_>>();
    print_table(&["slot", "name", "state", "bits", "ci_lo", "ci_hi"], &rows)
}

fn profile_lens_command(args: ProfileLensArgs) -> CliResult {
    let name = args.name.as_deref().unwrap_or(DEFAULT_PROFILE_NAME);
    let runtime = args.runtime.as_deref().unwrap_or("algorithmic");
    let built = build_lens(
        name,
        runtime,
        args.endpoint.as_deref(),
        args.weights.as_deref(),
        args.shape.as_deref(),
        args.modality.as_deref(),
    )?;
    let lens_id = built.lens_id;
    let mut registry = Registry::new();
    built.register(&mut registry)?;
    let probes = profile_probes(args.probe.as_deref(), built_modality(&registry, lens_id)?)?;
    let card = profile_lens(&registry, lens_id, &probes)?;
    print_json(&card)
}

fn builtin_panel_template(name: &str) -> CliResult<PanelTemplate> {
    Ok(match name {
        "text-default" => text_default(),
        "code-default" => code_default(),
        "civic-default" => civic_default(),
        "legal-default" => legal_default(),
        "medical-default" => medical_default(),
        "bio-default" => bio_default(),
        "media-default" => media_default(),
        other => {
            return Err(CliError::usage(format!(
                "unknown --panel-template {other}; expected one of {}",
                super::PANEL_TEMPLATES.join(", ")
            )));
        }
    })
}

fn prepare_vault_panel(home: &Path, template: &str) -> CliResult<PreparedVaultPanel> {
    if super::PANEL_TEMPLATES.contains(&template) {
        let materialized =
            materialize_panel_template(&builtin_panel_template(template)?, now_ms())?;
        return Ok(PreparedVaultPanel {
            content_lens_count: panel_content_lens_count(&materialized.panel),
            panel: materialized.panel,
            registry: materialized.registry,
            template_source: "built_in_materialized".to_string(),
            registered_lenses_added: materialized.registered_lenses_added,
            inactive_unmaterialized_slots: materialized.inactive_unmaterialized_slots,
            a37_gate_eligible: false,
            a37_status: "built_in_template_not_a37_profiled".to_string(),
        });
    }

    match build_saved_template_panel(home, template, now_ms()) {
        Ok(saved) => Ok(PreparedVaultPanel {
            panel: saved.panel,
            registry: saved.registry,
            template_source: format!("saved:{}:{}", saved.template_name, saved.template_id),
            content_lens_count: saved.content_lens_count,
            registered_lenses_added: saved.registered_lenses_added,
            inactive_unmaterialized_slots: Vec::new(),
            a37_gate_eligible: saved.a37_gate_eligible,
            a37_status: saved.a37_status,
        }),
        Err(error) if error.code() == "CALYX_PANEL_TEMPLATE_NOT_FOUND" => {
            let saved = saved_template_names(home).unwrap_or_default();
            Err(CliError::usage(format!(
                "unknown --panel-template {template}; built-ins are {}; saved templates in {} are [{}]",
                super::PANEL_TEMPLATES.join(", "),
                home.join("panels")
                    .join("templates")
                    .join("index.json")
                    .display(),
                saved.join(", ")
            )))
        }
        Err(error) => Err(error),
    }
}

fn panel_content_lens_count(panel: &Panel) -> usize {
    panel
        .slots
        .iter()
        .filter(|slot| !slot.retrieval_only && !slot.excluded_from_dedup)
        .count()
}

pub(crate) fn home_dir() -> CliResult<PathBuf> {
    env::var_os("CALYX_HOME")
        .map(PathBuf::from)
        .ok_or_else(|| CliError::usage("CALYX_HOME is required for vault commands"))
}

fn read_index(home: &Path) -> CliResult<VaultIndex> {
    let path = index_path(home);
    if !path.exists() {
        return Ok(VaultIndex::default());
    }
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

fn write_index(home: &Path, index: &VaultIndex) -> CliResult {
    let path = index_path(home);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let value = serde_json::to_value(index)?;
    crate::durable_write::write_json_value_atomic(&path, &value, "vault index")
}

fn index_path(home: &Path) -> PathBuf {
    home.join("vaults").join("index.json")
}

mod resolve;
pub(crate) use resolve::{resolve_vault, resolve_vault_info};

pub(crate) fn vault_salt(vault_id: VaultId, name: &str) -> Vec<u8> {
    format!("calyx-cli-vault:{vault_id}:{name}").into_bytes()
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as u64)
}

#[cfg(test)]
mod tests;
