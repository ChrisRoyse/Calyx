//! Vault and panel MCP tools for PH63 T02.

mod lens;
pub(super) mod store;

use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{Asymmetry, CalyxError, Panel, Slot, SlotId, SlotState};
use calyx_registry::{
    SlotSpec, SwapController, civic_default, code_default, instantiate_panel,
    load_vault_panel_state, media_default, persist_vault_panel_state, text_default,
};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use ulid::Ulid;

use crate::protocol::ToolDef;
use crate::schema::{integer_schema, object_schema, string_schema};
use crate::server::{McpServer, Tool, ToolError, ToolResult};

use self::lens::build_lens;
use self::store::{VaultIndexEntry, home_dir, read_index, resolve_vault, vault_salt, write_index};

const DEFAULT_TEMPLATE: &str = "text-default";

pub fn register(server: &mut McpServer) -> Result<(), CalyxError> {
    server.register(Box::new(CreateVaultTool))?;
    server.register(Box::new(AddLensTool))?;
    server.register(Box::new(RetireLensTool))?;
    server.register(Box::new(ParkLensTool))?;
    server.register(Box::new(ListPanelTool))?;
    server.register(Box::new(ProfileLensTool))?;
    Ok(())
}

struct CreateVaultTool;
struct AddLensTool;
struct RetireLensTool;
struct ParkLensTool;
struct ListPanelTool;
struct ProfileLensTool;

impl Tool for CreateVaultTool {
    fn def(&self) -> ToolDef {
        def(
            "calyx.create_vault",
            "create a durable Calyx vault",
            "start a new database; picks text/code/civic/media-default panel",
            object_schema(&[
                ("name", string_schema(), true),
                (
                    "panel_template",
                    enum_string(&[
                        "text-default",
                        "code-default",
                        "civic-default",
                        "media-default",
                    ]),
                    false,
                ),
            ]),
        )
    }

    fn call(&self, params: Value) -> ToolResult<Value> {
        let args: CreateVaultArgs = decode("calyx.create_vault", params)?;
        validate_path_safe("vault name", &args.name)?;
        let template = args.panel_template.as_deref().unwrap_or(DEFAULT_TEMPLATE);
        let panel = panel_for_template(template)?;
        let home = home_dir()?;
        let mut index = read_index(&home)?;
        if index.vaults.iter().any(|entry| entry.name == args.name) {
            return Err(ToolError::invalid_params(format!(
                "vault name {} already exists",
                args.name
            )));
        }

        let vault_id = calyx_core::VaultId::from_ulid(Ulid::new());
        let relative = format!("vaults/{vault_id}");
        let vault_dir = home.join(&relative);
        if vault_dir.exists() {
            return Err(ToolError::invalid_params(format!(
                "vault directory for {vault_id} already exists"
            )));
        }
        let options = VaultOptions {
            panel: Some(panel),
            ..VaultOptions::default()
        };
        AsterVault::new_durable(
            &vault_dir,
            vault_id,
            vault_salt(vault_id, &args.name),
            options,
        )?;
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
        Ok(json!({
            "vault_id": vault_id.to_string(),
            "name": args.name,
            "panel_template": template,
        }))
    }

    fn requires_authn(&self) -> bool {
        true
    }
}

impl Tool for AddLensTool {
    fn def(&self) -> ToolDef {
        def(
            "calyx.add_lens",
            "add one frozen lens to a vault panel",
            "add a measurement axis - the one call that replaces a whole pipeline",
            object_schema(&[
                ("vault", string_schema(), true),
                ("name", string_schema(), true),
                (
                    "runtime",
                    enum_string(&["tei-http", "onnx", "candle", "algorithmic"]),
                    true,
                ),
                ("endpoint", string_schema(), false),
                ("weights", string_schema(), false),
                ("shape", string_schema(), false),
                (
                    "modality",
                    enum_string(&[
                        "text",
                        "code",
                        "image",
                        "audio",
                        "video",
                        "structured",
                        "mixed",
                    ]),
                    false,
                ),
            ]),
        )
    }

    fn call(&self, params: Value) -> ToolResult<Value> {
        let args: AddLensArgs = decode("calyx.add_lens", params)?;
        validate_path_safe("lens name", &args.name)?;
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
        Ok(json!({
            "lens_id": lens_id.to_string(),
            "slot_id": outcome.slot.slot_id.get(),
            "name": args.name,
            "state": "active",
        }))
    }

    fn requires_authn(&self) -> bool {
        true
    }
}

impl Tool for RetireLensTool {
    fn def(&self) -> ToolDef {
        lifecycle_def(
            "calyx.retire_lens",
            "retire a panel slot",
            "drop a low-signal lens permanently",
        )
    }

    fn call(&self, params: Value) -> ToolResult<Value> {
        set_lens_state(params, SlotStateAction::Retire)
    }

    fn requires_authn(&self) -> bool {
        true
    }
}

impl Tool for ParkLensTool {
    fn def(&self) -> ToolDef {
        lifecycle_def(
            "calyx.park_lens",
            "park a panel slot",
            "sideline a lens without deleting its data",
        )
    }

    fn call(&self, params: Value) -> ToolResult<Value> {
        set_lens_state(params, SlotStateAction::Park)
    }

    fn requires_authn(&self) -> bool {
        true
    }
}

impl Tool for ListPanelTool {
    fn def(&self) -> ToolDef {
        def(
            "calyx.list_panel",
            "list panel slots",
            "see lenses, their bits signal, and state",
            object_schema(&[("vault", string_schema(), true)]),
        )
    }

    fn call(&self, params: Value) -> ToolResult<Value> {
        let args: VaultRefArgs = decode("calyx.list_panel", params)?;
        let home = home_dir()?;
        let vault_dir = resolve_vault(&home, &args.vault)?;
        let state = load_vault_panel_state(&vault_dir)?;
        let slots = state
            .panel
            .slots
            .iter()
            .map(slot_report)
            .collect::<Vec<_>>();
        Ok(json!({ "slots": slots }))
    }

    fn requires_authn(&self) -> bool {
        false
    }
}

impl Tool for ProfileLensTool {
    fn def(&self) -> ToolDef {
        def(
            "calyx.profile_lens",
            "profile a candidate lens",
            "get a capability card before committing to a lens",
            object_schema(&[
                (
                    "runtime",
                    enum_string(&["tei-http", "onnx", "candle", "algorithmic"]),
                    true,
                ),
                ("endpoint", string_schema(), false),
                ("weights", string_schema(), false),
                ("probe", string_schema(), false),
                (
                    "modality",
                    enum_string(&[
                        "text",
                        "code",
                        "image",
                        "audio",
                        "video",
                        "structured",
                        "mixed",
                    ]),
                    false,
                ),
            ]),
        )
    }

    fn call(&self, params: Value) -> ToolResult<Value> {
        let args: ProfileLensArgs = decode("calyx.profile_lens", params)?;
        Ok(json!(lens::profile_candidate(
            &args.runtime,
            args.endpoint.as_deref(),
            args.weights.as_deref(),
            args.probe.as_deref(),
            args.modality.as_deref(),
        )?))
    }

    fn requires_authn(&self) -> bool {
        false
    }
}

#[derive(Deserialize)]
struct CreateVaultArgs {
    name: String,
    panel_template: Option<String>,
}

#[derive(Deserialize)]
struct AddLensArgs {
    vault: String,
    name: String,
    runtime: String,
    endpoint: Option<String>,
    weights: Option<String>,
    shape: Option<String>,
    modality: Option<String>,
}

#[derive(Deserialize)]
struct SlotCommandArgs {
    vault: String,
    slot: u16,
}

#[derive(Deserialize)]
struct VaultRefArgs {
    vault: String,
}

#[derive(Deserialize)]
struct ProfileLensArgs {
    runtime: String,
    endpoint: Option<String>,
    weights: Option<String>,
    probe: Option<String>,
    modality: Option<String>,
}

#[derive(Clone, Copy)]
enum SlotStateAction {
    Retire,
    Park,
}

fn set_lens_state(params: Value, action: SlotStateAction) -> ToolResult<Value> {
    let tool = match action {
        SlotStateAction::Retire => "calyx.retire_lens",
        SlotStateAction::Park => "calyx.park_lens",
    };
    let args: SlotCommandArgs = decode(tool, params)?;
    let home = home_dir()?;
    let vault_dir = resolve_vault(&home, &args.vault)?;
    let state = load_vault_panel_state(&vault_dir)?;
    let slot_id = SlotId::new(args.slot);
    if !state.panel.slots.iter().any(|slot| slot.slot_id == slot_id) {
        return Err(
            CalyxError::vault_access_denied(format!("slot {slot_id} does not exist")).into(),
        );
    }
    let mut controller = SwapController::new(state.panel);
    let status = match action {
        SlotStateAction::Retire => {
            controller.retire_lens(slot_id, now_ms())?;
            "retired"
        }
        SlotStateAction::Park => {
            controller.park_lens(slot_id, now_ms())?;
            "parked"
        }
    };
    persist_vault_panel_state(&vault_dir, controller.panel(), &state.registry)?;
    Ok(json!({ "status": status, "slot": args.slot }))
}

fn slot_report(slot: &Slot) -> Value {
    let signal = slot
        .bits_about
        .values()
        .max_by(|left, right| left.bits.total_cmp(&right.bits));
    let (bits, ci) = match signal {
        Some(signal) => (json!(signal.bits), json!([signal.ci.low, signal.ci.high])),
        None => (Value::Null, Value::Null),
    };
    json!({
        "slot": slot.slot_id.get(),
        "name": slot.slot_key.key(),
        "state": state_name(slot.state),
        "modality": slot.modality,
        "bits": bits,
        "ci": ci,
        "lens_id": slot.lens_id.to_string(),
    })
}

fn state_name(state: SlotState) -> &'static str {
    match state {
        SlotState::Active => "active",
        SlotState::Parked => "parked",
        SlotState::Retired => "retired",
    }
}

fn panel_for_template(name: &str) -> ToolResult<Panel> {
    let template = match name {
        "text-default" => text_default(),
        "code-default" => code_default(),
        "civic-default" => civic_default(),
        "media-default" => media_default(),
        other => {
            return Err(ToolError::invalid_params(format!(
                "unknown panel_template {other}; expected text-default, code-default, civic-default, or media-default"
            )));
        }
    };
    Ok(instantiate_panel(&template, now_ms()).panel)
}

pub(super) fn validate_path_safe(kind: &str, value: &str) -> ToolResult<()> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.chars().any(char::is_whitespace)
        || value.contains(['/', '\\'])
    {
        return Err(ToolError::invalid_params(format!(
            "{kind} must be non-empty and path-safe"
        )));
    }
    Ok(())
}

pub(super) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as u64)
}

fn decode<T: DeserializeOwned>(tool: &str, params: Value) -> ToolResult<T> {
    serde_json::from_value(params)
        .map_err(|err| ToolError::invalid_params(format!("{tool} invalid arguments: {err}")))
}

fn def(name: &str, description: &str, use_when: &str, input_schema: Value) -> ToolDef {
    ToolDef {
        name: name.to_string(),
        description: description.to_string(),
        use_when: use_when.to_string(),
        input_schema,
    }
}

fn lifecycle_def(name: &str, description: &str, use_when: &str) -> ToolDef {
    def(
        name,
        description,
        use_when,
        object_schema(&[
            ("vault", string_schema(), true),
            ("slot", integer_schema(), true),
        ]),
    )
}

fn enum_string(values: &[&str]) -> Value {
    json!({ "type": "string", "enum": values })
}

#[cfg(test)]
mod tests;
