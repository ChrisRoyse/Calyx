//! Ingest, anchor, and measure MCP tools for PH63 T03.

mod anchor;
mod derived_text;
mod input_retention;
mod measure;
mod media;
mod report;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};

use calyx_aster::cf::{ColumnFamily, base_key};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{Anchor, CalyxError, CxId, Input, Modality, StructuredRecord, VaultStore};
use calyx_ledger::{ActorId, EntryKind, RedactionPolicy, SubjectId};
use calyx_registry::load_vault_panel_state;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::protocol::ToolDef;
use crate::schema::{array_schema, boolean_schema, number_schema, object_schema, string_schema};
use crate::server::{McpServer, Tool, ToolError, ToolResult};

use self::anchor::{
    append_anchor_ledger, parse_anchor_kind, parse_anchor_value, validate_confidence,
};
use self::input_retention::prepared_text_input;
use self::measure::measure_constellation;
use self::report::constellation_report;
use super::vault::now_ms;
use super::vault::store::{ResolvedVault, home_dir, resolve_vault_info, vault_salt};
use crate::tools::search_generation::publish_search_generation;

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
                ("record_schema", string_schema(), false),
                ("record", json!({ "type": "object" }), false),
            ]),
        )
    }

    fn call(&self, params: Value) -> ToolResult<Value> {
        let args: IngestArgs = decode("calyx.ingest", params)?;
        let texts = ingest_texts_arg(args.input, args.batch, args.record_schema, args.record)?;
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
        publish_search_generation(
            &resolved.path,
            &vault,
            &load_vault_panel_state(&resolved.path)?,
        )?;
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
                ("record_schema", string_schema(), false),
                ("record", json!({ "type": "object" }), false),
            ]),
        )
    }

    fn call(&self, params: Value) -> ToolResult<Value> {
        let args: MeasureArgs = decode("calyx.measure", params)?;
        validate_text(&args.input)?;
        let resolved = resolve_requested_vault(&args.vault)?;
        let vault = open_vault(&resolved)?;
        let state = load_vault_panel_state(&resolved.path)?;
        let input = attach_structured(text_input(args.input), args.record_schema, args.record)?;
        let measured = measure_constellation(&vault, &state, input, now_ms())?;
        Ok(constellation_report(&measured.constellation, &state))
    }

    fn requires_authn(&self) -> bool {
        false
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IngestArgs {
    vault: String,
    input: Option<String>,
    batch: Option<Vec<String>>,
    record_schema: Option<String>,
    record: Option<Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
struct MeasureArgs {
    vault: String,
    input: String,
    record_schema: Option<String>,
    record: Option<Value>,
}

#[derive(serde::Serialize)]
struct IngestReport {
    cx_id: String,
    new: bool,
    ledger_seq: u64,
}

struct PreparedInput {
    input: Input,
    metadata: BTreeMap<String, String>,
}

struct PreparedReport {
    cx_id: CxId,
    new: bool,
    existed_before: bool,
    input_ref: calyx_core::InputRef,
    retained_bytes: Vec<u8>,
}

struct TextSource {
    text: String,
    structured: Option<StructuredRecord>,
}

fn ingest_texts_arg(
    input: Option<String>,
    batch: Option<Vec<String>>,
    record_schema: Option<String>,
    record: Option<Value>,
) -> ToolResult<Vec<TextSource>> {
    let structured = structured_record(record_schema, record)?;
    match (input, batch) {
        (Some(_), Some(_)) => Err(ToolError::invalid_params(
            "input and batch are mutually exclusive",
        )),
        (Some(input), None) => {
            validate_text(&input)?;
            Ok(vec![TextSource {
                text: input,
                structured,
            }])
        }
        (None, Some(batch)) if batch.is_empty() => {
            Err(ToolError::invalid_params("batch must not be empty"))
        }
        (None, Some(batch)) => {
            if structured.is_some() {
                return Err(ToolError::invalid_params(
                    "record_schema/record can only accompany one input; use one calyx.ingest call per structured record",
                ));
            }
            for item in &batch {
                validate_text(item)?;
            }
            Ok(batch
                .into_iter()
                .map(|text| TextSource {
                    text,
                    structured: None,
                })
                .collect())
        }
        (None, None) => Err(ToolError::invalid_params(
            "calyx.ingest requires input or batch",
        )),
    }
}

fn ingest_texts(resolved: &ResolvedVault, texts: &[TextSource]) -> ToolResult<Vec<IngestReport>> {
    let inputs = texts
        .iter()
        .map(|source| {
            let input = prepared_text_input(&source.text);
            Ok(PreparedInput {
                input: match &source.structured {
                    Some(record) => input.with_structured(record.clone()),
                    None => input,
                },
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
        let retained_bytes = input.input.bytes.clone();
        let mut measured = measure_constellation(&vault, &state, input.input, now_ms())?;
        for (key, value) in input.metadata {
            if measured
                .constellation
                .metadata
                .insert(key.clone(), value)
                .is_some()
            {
                return Err(CalyxError::aster_corrupt_shard(format!(
                    "MCP ingest metadata key {key:?} collides with engine-owned structured state"
                ))
                .into());
            }
        }
        let cx_id = measured.constellation.cx_id;
        let existed_before = base_exists(&vault, cx_id)?;
        let new = !existed_before && first_new.insert(cx_id);
        if new {
            staged.push((measured.constellation.clone(), retained_bytes.clone()));
        }
        prepared.push(PreparedReport {
            cx_id,
            new,
            existed_before,
            input_ref: measured.constellation.input_ref,
            retained_bytes,
        });
    }
    for row in prepared.iter().filter(|row| row.existed_before) {
        vault.backfill_input_pointer_with_retained_input(
            row.cx_id,
            &row.input_ref,
            &row.retained_bytes,
        )?;
    }
    match staged.len() {
        0 => {}
        1 => {
            vault.put_batch_with_retained_inputs([staged
                .pop()
                .expect("one staged constellation")])?;
        }
        _ => {
            vault.put_batch_with_retained_inputs(staged)?;
        }
    }
    vault.flush()?;
    let snapshot = vault.snapshot();
    let mut reports = Vec::with_capacity(prepared.len());
    for row in prepared {
        let stored = vault.get(row.cx_id, snapshot)?;
        let ledger_seq = if row.new {
            stored.provenance.seq
        } else {
            append_ingest_retry_ledger(&vault, row.cx_id)?
        };
        reports.push(IngestReport {
            cx_id: row.cx_id.to_string(),
            new: row.new,
            ledger_seq,
        });
    }
    vault.flush()?;
    publish_search_generation(&resolved.path, &vault, &state)?;
    Ok(reports)
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

fn structured_record(
    schema: Option<String>,
    record: Option<Value>,
) -> ToolResult<Option<StructuredRecord>> {
    match (schema, record) {
        (None, None) => Ok(None),
        (Some(schema), Some(record)) => {
            Ok(Some(StructuredRecord::from_json_value(schema, &record)?))
        }
        _ => Err(CalyxError {
            code: calyx_core::CALYX_STRUCTURED_SCHEMA_MISMATCH,
            message: "record_schema and record must be provided together".to_string(),
            remediation: "supply both the exact stable schema id and its typed JSON object",
        }
        .into()),
    }
}

fn attach_structured(
    input: Input,
    schema: Option<String>,
    record: Option<Value>,
) -> ToolResult<Input> {
    Ok(match structured_record(schema, record)? {
        Some(record) => input.with_structured(record),
        None => input,
    })
}

fn text_input(text: String) -> Input {
    Input::new(Modality::Text, text.into_bytes())
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
