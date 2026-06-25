//! Ingest, anchor, and measure MCP tools for PH63 T03.

mod anchor;
mod input_retention;
mod media;
mod report;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};

use calyx_aster::cf::{ColumnFamily, base_key};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    AbsentReason, Anchor, CalyxError, Constellation, CxFlags, CxId, Input, InputRef, LedgerRef,
    Modality, Slot, SlotState, SlotVector, VaultStore,
};
use calyx_ledger::{ActorId, EntryKind, RedactionPolicy, SubjectId};
use calyx_registry::{VaultPanelState, load_vault_panel_state};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::protocol::ToolDef;
use crate::schema::{array_schema, boolean_schema, number_schema, object_schema, string_schema};
use crate::server::{McpServer, Tool, ToolError, ToolResult};

use self::anchor::{
    append_anchor_ledger, parse_anchor_kind, parse_anchor_value, validate_confidence,
};
use self::input_retention::{input_hash, retained_text_input};
use self::report::constellation_report;
use super::vault::now_ms;
use super::vault::store::{ResolvedVault, home_dir, resolve_vault_info, vault_salt};

const DEFAULT_ANCHOR_SOURCE: &str = "calyx-mcp";

pub fn register(server: &mut McpServer) -> Result<(), CalyxError> {
    server.register(Box::new(IngestTool))?;
    media::register(server)?;
    server.register(Box::new(AnchorTool))?;
    server.register(Box::new(MeasureTool))?;
    Ok(())
}

struct IngestTool;
struct AnchorTool;
struct MeasureTool;

impl Tool for IngestTool {
    fn def(&self) -> ToolDef {
        def(
            "calyx.ingest",
            "ingest text into a Calyx vault",
            "store data -> constellation (auto multi-lens, idempotent)",
            object_schema(&[
                ("vault", string_schema(), true),
                ("input", string_schema(), false),
                ("batch", array_schema(string_schema()), false),
            ]),
        )
    }

    fn call(&self, params: Value) -> ToolResult<Value> {
        let args: IngestArgs = decode("calyx.ingest", params)?;
        let texts = ingest_texts_arg(args.input, args.batch)?;
        let resolved = resolve_requested_vault(&args.vault)?;
        let reports = ingest_texts(&resolved, &texts)?;
        if reports.len() == 1 {
            Ok(serde_json::to_value(&reports[0])
                .map_err(|err| CalyxError::aster_corrupt_shard(format!("encode ingest: {err}")))?)
        } else {
            Ok(json!({ "results": reports }))
        }
    }

    fn requires_authn(&self) -> bool {
        true
    }
}

impl Tool for AnchorTool {
    fn def(&self) -> ToolDef {
        def(
            "calyx.anchor",
            "attach a grounded outcome",
            "attach a grounded outcome (test pass, thumbs, label)",
            object_schema(&[
                ("vault", string_schema(), true),
                ("cx_id", string_schema(), true),
                (
                    "kind",
                    enum_string(&[
                        "test_pass",
                        "thumbs_up",
                        "thumbs_down",
                        "speaker_match",
                        "style_hold",
                        "label",
                    ]),
                    true,
                ),
                ("label", string_schema(), false),
                ("value", value_schema(), true),
                ("confidence", number_schema(), false),
                ("source", string_schema(), false),
            ]),
        )
    }

    fn call(&self, params: Value) -> ToolResult<Value> {
        let args: AnchorArgs = decode("calyx.anchor", params)?;
        let resolved = resolve_requested_vault(&args.vault)?;
        let vault = open_vault(&resolved)?;
        let cx_id = parse_cx_id(&args.cx_id)?;
        ensure_base_exists(&vault, cx_id)?;
        let kind = parse_anchor_kind(&args.kind, args.label.as_deref())?;
        let anchor = Anchor {
            value: parse_anchor_value(&args.kind, &args.value)?,
            kind: kind.clone(),
            source: args
                .source
                .unwrap_or_else(|| DEFAULT_ANCHOR_SOURCE.to_string()),
            observed_at: now_ms(),
            confidence: args.confidence.unwrap_or(1.0),
        };
        validate_confidence(anchor.confidence)?;
        let ledger_seq = append_anchor_ledger(&vault, cx_id, &kind, anchor)?;
        vault.flush()?;
        Ok(json!({
            "status": "anchored",
            "cx_id": cx_id.to_string(),
            "ledger_seq": ledger_seq,
        }))
    }

    fn requires_authn(&self) -> bool {
        true
    }
}

impl Tool for MeasureTool {
    fn def(&self) -> ToolDef {
        def(
            "calyx.measure",
            "measure text without storing it",
            "get the constellation without storing (for guarding a candidate)",
            object_schema(&[
                ("vault", string_schema(), true),
                ("input", string_schema(), true),
            ]),
        )
    }

    fn call(&self, params: Value) -> ToolResult<Value> {
        let args: MeasureArgs = decode("calyx.measure", params)?;
        validate_text(&args.input)?;
        let resolved = resolve_requested_vault(&args.vault)?;
        let vault = open_vault(&resolved)?;
        let state = load_vault_panel_state(&resolved.path)?;
        let measured = measure_constellation(&vault, &state, text_input(args.input), now_ms())?;
        Ok(constellation_report(&measured.constellation, &state))
    }

    fn requires_authn(&self) -> bool {
        false
    }
}

#[derive(Deserialize)]
struct IngestArgs {
    vault: String,
    input: Option<String>,
    batch: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct AnchorArgs {
    vault: String,
    cx_id: String,
    kind: String,
    label: Option<String>,
    value: Value,
    confidence: Option<f32>,
    source: Option<String>,
}

#[derive(Deserialize)]
struct MeasureArgs {
    vault: String,
    input: String,
}

#[derive(serde::Serialize)]
struct IngestReport {
    cx_id: String,
    new: bool,
    ledger_seq: u64,
}

struct MeasuredConstellation {
    constellation: Constellation,
}

struct PreparedInput {
    input: Input,
    metadata: BTreeMap<String, String>,
}

fn ingest_texts_arg(input: Option<String>, batch: Option<Vec<String>>) -> ToolResult<Vec<String>> {
    match (input, batch) {
        (Some(_), Some(_)) => Err(ToolError::invalid_params(
            "input and batch are mutually exclusive",
        )),
        (Some(input), None) => {
            validate_text(&input)?;
            Ok(vec![input])
        }
        (None, Some(batch)) if batch.is_empty() => {
            Err(ToolError::invalid_params("batch must not be empty"))
        }
        (None, Some(batch)) => {
            for item in &batch {
                validate_text(item)?;
            }
            Ok(batch)
        }
        (None, None) => Err(ToolError::invalid_params(
            "calyx.ingest requires input or batch",
        )),
    }
}

fn ingest_texts(resolved: &ResolvedVault, texts: &[String]) -> ToolResult<Vec<IngestReport>> {
    let inputs = texts
        .iter()
        .map(|text| {
            Ok(PreparedInput {
                input: retained_text_input(resolved, text)?,
                metadata: BTreeMap::new(),
            })
        })
        .collect::<ToolResult<Vec<_>>>()?;
    ingest_prepared_inputs(resolved, inputs)
}

fn ingest_prepared_inputs(
    resolved: &ResolvedVault,
    inputs: Vec<PreparedInput>,
) -> ToolResult<Vec<IngestReport>> {
    let vault = open_vault(resolved)?;
    let state = load_vault_panel_state(&resolved.path)?;
    let mut staged = Vec::new();
    let mut prepared = Vec::with_capacity(inputs.len());
    let mut first_new = BTreeSet::new();
    for input in inputs {
        let mut measured = measure_constellation(&vault, &state, input.input, now_ms())?;
        measured.constellation.metadata = input.metadata;
        let cx_id = measured.constellation.cx_id;
        let new = !base_exists(&vault, cx_id)? && first_new.insert(cx_id);
        if new {
            staged.push(measured.constellation);
        }
        prepared.push((cx_id, new));
    }
    match staged.len() {
        0 => {}
        1 => {
            vault.put(staged.pop().expect("one staged constellation"))?;
        }
        _ => {
            vault.put_batch(staged)?;
        }
    }
    vault.flush()?;
    let snapshot = vault.snapshot();
    let mut reports = Vec::with_capacity(prepared.len());
    for (cx_id, new) in prepared {
        let stored = vault.get(cx_id, snapshot)?;
        let ledger_seq = if new {
            stored.provenance.seq
        } else {
            append_ingest_retry_ledger(&vault, cx_id)?
        };
        reports.push(IngestReport {
            cx_id: cx_id.to_string(),
            new,
            ledger_seq,
        });
    }
    vault.flush()?;
    Ok(reports)
}

fn measure_constellation(
    vault: &AsterVault,
    state: &VaultPanelState,
    input: Input,
    now: u64,
) -> ToolResult<MeasuredConstellation> {
    let cx_id = vault.cx_id_for_input(&input.bytes, state.panel.version);
    let mut slots = BTreeMap::new();
    let mut degraded = false;
    let mut applicable = 0_usize;
    let mut produced = 0_usize;
    let mut unavailable = 0_usize;
    for slot in &state.panel.slots {
        let vector = measure_slot(
            slot,
            state,
            &input,
            &mut applicable,
            &mut produced,
            &mut unavailable,
        )?;
        degraded |= slot.counts_toward_degraded(input.modality) && vector.is_absent();
        slots.insert(slot.slot_id, vector);
    }
    if applicable == 0 {
        return Err(ToolError::invalid_params(format!(
            "panel has no active {:?}-compatible slots",
            input.modality
        )));
    }
    if produced == 0 && unavailable == applicable {
        return Err(
            CalyxError::lens_unreachable("all applicable lens runtimes unreachable").into(),
        );
    }
    Ok(MeasuredConstellation {
        constellation: Constellation {
            cx_id,
            vault_id: vault.vault_id(),
            panel_version: state.panel.version,
            created_at: now,
            input_ref: InputRef {
                hash: input_hash(&input.bytes),
                pointer: input.pointer,
                redacted: false,
            },
            modality: input.modality,
            slots,
            scalars: BTreeMap::new(),
            metadata: BTreeMap::new(),
            anchors: Vec::new(),
            provenance: LedgerRef {
                seq: vault.latest_seq().saturating_add(1),
                hash: [0; 32],
            },
            flags: CxFlags {
                ungrounded: true,
                degraded,
                novel_region: false,
                redacted_input: false,
            },
        },
    })
}

fn measure_slot(
    slot: &Slot,
    state: &VaultPanelState,
    input: &Input,
    applicable: &mut usize,
    produced: &mut usize,
    unavailable: &mut usize,
) -> ToolResult<SlotVector> {
    if slot.state != SlotState::Active {
        return Ok(absent(AbsentReason::LensInactive));
    }
    if slot.modality != input.modality {
        return Ok(absent(AbsentReason::NotApplicable));
    }
    *applicable += 1;
    if !state.registry.contains(slot.lens_id) {
        *unavailable += 1;
        return Ok(absent(AbsentReason::LensUnavailable));
    }
    match state.registry.measure(slot.lens_id, input) {
        Ok(vector) => {
            *produced += 1;
            Ok(vector)
        }
        Err(error) if error.code == "CALYX_LENS_UNREACHABLE" => {
            *unavailable += 1;
            Ok(absent(AbsentReason::LensUnavailable))
        }
        Err(error) => Err(error.into()),
    }
}

fn append_ingest_retry_ledger(vault: &AsterVault, cx_id: CxId) -> ToolResult<u64> {
    let bytes = serde_json::to_vec(&json!({ "mode": "mcp-idempotent-ingest" }))
        .map_err(|err| CalyxError::aster_corrupt_shard(format!("encode retry ledger: {err}")))?;
    RedactionPolicy::check_payload(&bytes)?;
    append_ledger_payload(vault, EntryKind::Ingest, cx_id, bytes)
}

fn append_ledger_payload(
    vault: &AsterVault,
    kind: EntryKind,
    cx_id: CxId,
    bytes: Vec<u8>,
) -> ToolResult<u64> {
    Ok(vault
        .append_ledger_entry(
            kind,
            SubjectId::Cx(cx_id),
            bytes,
            ActorId::Service(DEFAULT_ANCHOR_SOURCE.to_string()),
        )?
        .seq)
}

fn resolve_requested_vault(vault: &str) -> ToolResult<ResolvedVault> {
    let home = home_dir()?;
    resolve_vault_info(&home, vault)
}

fn open_vault(resolved: &ResolvedVault) -> ToolResult<AsterVault> {
    Ok(AsterVault::open(
        &resolved.path,
        resolved.vault_id,
        vault_salt(resolved.vault_id, &resolved.name),
        VaultOptions::default(),
    )?)
}

fn ensure_base_exists(vault: &AsterVault, cx_id: CxId) -> ToolResult<()> {
    if base_exists(vault, cx_id)? {
        return Ok(());
    }
    Err(CalyxError::vault_access_denied(format!("cx_id {cx_id} does not exist in vault")).into())
}

fn base_exists(vault: &AsterVault, cx_id: CxId) -> ToolResult<bool> {
    Ok(vault
        .read_cf_at(vault.snapshot(), ColumnFamily::Base, &base_key(cx_id))?
        .is_some())
}

fn parse_cx_id(value: &str) -> ToolResult<CxId> {
    value
        .parse::<CxId>()
        .map_err(|err| ToolError::invalid_params(format!("parse cx_id {value}: {err}")))
}

fn validate_text(value: &str) -> ToolResult<()> {
    if value.is_empty() {
        return Err(ToolError::invalid_params("input must not be empty"));
    }
    Ok(())
}

fn text_input(text: String) -> Input {
    Input::new(Modality::Text, text.into_bytes())
}

fn absent(reason: AbsentReason) -> SlotVector {
    SlotVector::Absent { reason }
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

fn enum_string(values: &[&str]) -> Value {
    json!({ "type": "string", "enum": values })
}

fn value_schema() -> Value {
    json!({ "oneOf": [boolean_schema(), number_schema()] })
}
