use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use calyx_aster::cf::{ColumnFamily, base_key};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    AbsentReason, Anchor, AnchorKind, AnchorValue, CalyxError, Constellation, CxFlags, CxId, Input,
    InputRef, LedgerRef, Modality, SlotState, SlotVector, VaultStore,
};
use calyx_ledger::{ActorId, EntryKind, PayloadBuilder, RedactionPolicy, SubjectId};
use calyx_registry::{VaultPanelState, load_vault_panel_state};
use serde::{Deserialize, Serialize};

use super::vault::{ResolvedVault, home_dir, now_ms, resolve_vault_info, vault_salt};
use super::{AnchorArgs, IngestArgs, MeasureArgs, Subcommand, value};
use crate::error::{CliError, CliResult};
use crate::output::print_json;

const DEFAULT_ANCHOR_SOURCE: &str = "calyx-cli";

#[derive(Serialize)]
struct IngestReport {
    cx_id: String,
    new: bool,
    ledger_seq: u64,
}

#[derive(Serialize)]
struct AnchorReport {
    status: &'static str,
    cx_id: String,
    ledger_seq: u64,
}

#[derive(Deserialize)]
struct BatchLine {
    text: String,
}

pub(crate) fn run(command: Subcommand) -> CliResult {
    match command {
        Subcommand::Ingest(args) => ingest_command(args),
        Subcommand::Anchor(args) => anchor_command(args),
        Subcommand::Measure(args) => measure_command(args),
        _ => unreachable!("non-ingest command routed to ingest module"),
    }
}

pub(crate) fn parse_ingest(rest: &[String]) -> CliResult<Subcommand> {
    let vault = rest
        .first()
        .ok_or_else(|| CliError::usage("ingest requires <vault>"))?
        .clone();
    let mut text = None;
    let mut batch = None;
    let mut idempotent = true;
    let mut idx = 1;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--text" => {
                idx += 1;
                let raw = value(rest, idx, "--text")?;
                validate_text(raw)?;
                text = Some(raw.to_string());
            }
            "--batch" => {
                idx += 1;
                batch = Some(value(rest, idx, "--batch")?.into());
            }
            "--idempotent" => {
                if let Some(raw) = rest.get(idx + 1).filter(|next| !next.starts_with("--")) {
                    idx += 1;
                    idempotent = parse_bool(raw, "--idempotent")?;
                } else {
                    idempotent = true;
                }
            }
            "--no-idempotent" => idempotent = false,
            other => return Err(CliError::usage(format!("unexpected ingest flag {other}"))),
        }
        idx += 1;
    }
    if text.is_some() == batch.is_some() {
        return Err(CliError::usage(
            "ingest requires exactly one of --text <s> or --batch <jsonl-path>",
        ));
    }
    if !idempotent {
        return Err(CliError::usage(
            "non-idempotent ingest is not supported by Calyx",
        ));
    }
    Ok(Subcommand::Ingest(IngestArgs {
        vault,
        text,
        batch,
        idempotent,
    }))
}

pub(crate) fn parse_anchor(rest: &[String]) -> CliResult<Subcommand> {
    let vault = rest
        .first()
        .ok_or_else(|| CliError::usage("anchor requires <vault>"))?
        .clone();
    let cx_id = rest
        .get(1)
        .ok_or_else(|| CliError::usage("anchor requires <cx_id>"))?
        .clone();
    let mut kind = None;
    let mut anchor_value = None;
    let mut confidence = None;
    let mut source = None;
    let mut idx = 2;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--kind" => {
                idx += 1;
                kind = Some(value(rest, idx, "--kind")?.to_string());
            }
            "--value" => {
                idx += 1;
                anchor_value = Some(value(rest, idx, "--value")?.to_string());
            }
            "--confidence" => {
                idx += 1;
                let raw = value(rest, idx, "--confidence")?;
                let parsed = raw
                    .parse::<f32>()
                    .map_err(|err| CliError::usage(format!("parse --confidence {raw}: {err}")))?;
                validate_confidence(parsed)?;
                confidence = Some(parsed);
            }
            "--source" => {
                idx += 1;
                source = Some(value(rest, idx, "--source")?.to_string());
            }
            other => return Err(CliError::usage(format!("unexpected anchor flag {other}"))),
        }
        idx += 1;
    }
    Ok(Subcommand::Anchor(AnchorArgs {
        vault,
        cx_id,
        kind: kind.ok_or_else(|| CliError::usage("anchor requires --kind <kind>"))?,
        value: anchor_value.ok_or_else(|| CliError::usage("anchor requires --value <v>"))?,
        confidence,
        source,
    }))
}

pub(crate) fn parse_measure(rest: &[String]) -> CliResult<Subcommand> {
    let vault = rest
        .first()
        .ok_or_else(|| CliError::usage("measure requires <vault>"))?
        .clone();
    let mut text = None;
    let mut idx = 1;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--text" => {
                idx += 1;
                let raw = value(rest, idx, "--text")?;
                validate_text(raw)?;
                text = Some(raw.to_string());
            }
            other => return Err(CliError::usage(format!("unexpected measure flag {other}"))),
        }
        idx += 1;
    }
    Ok(Subcommand::Measure(MeasureArgs {
        vault,
        text: text.ok_or_else(|| CliError::usage("measure requires --text <s>"))?,
    }))
}

fn ingest_command(args: IngestArgs) -> CliResult {
    let resolved = resolve_cli_vault(&args.vault)?;
    let texts = if let Some(text) = args.text {
        vec![text]
    } else {
        read_batch_texts(args.batch.as_deref().expect("validated batch path"))?
    };
    for report in ingest_texts(&resolved, &texts)? {
        print_json(&report)?;
    }
    Ok(())
}

fn anchor_command(args: AnchorArgs) -> CliResult {
    let resolved = resolve_cli_vault(&args.vault)?;
    let vault = open_vault(&resolved)?;
    let cx_id = args
        .cx_id
        .parse::<CxId>()
        .map_err(|err| CliError::usage(format!("parse <cx_id> {}: {err}", args.cx_id)))?;
    ensure_base_exists(&vault, cx_id)?;
    let kind = parse_anchor_kind(&args.kind)?;
    let anchor = Anchor {
        value: parse_anchor_value(&kind, &args.kind, &args.value)?,
        kind,
        source: args
            .source
            .unwrap_or_else(|| DEFAULT_ANCHOR_SOURCE.to_string()),
        observed_at: now_ms(),
        confidence: args.confidence.unwrap_or(1.0),
    };
    vault.anchor(cx_id, anchor)?;
    let ledger_seq = append_cli_ledger(&vault, EntryKind::Ingest, cx_id, "cli-anchor")?;
    vault.flush()?;
    print_json(&AnchorReport {
        status: "anchored",
        cx_id: cx_id.to_string(),
        ledger_seq,
    })
}

fn measure_command(args: MeasureArgs) -> CliResult {
    let resolved = resolve_cli_vault(&args.vault)?;
    let vault = open_vault(&resolved)?;
    let state = load_vault_panel_state(&resolved.path)?;
    let cx = measure_constellation(&vault, &state, text_input(args.text), now_ms())?;
    print_json(&cx)
}

fn resolve_cli_vault(vault: &str) -> CliResult<ResolvedVault> {
    let home = home_dir()?;
    resolve_vault_info(&home, vault)
}

fn open_vault(resolved: &ResolvedVault) -> CliResult<AsterVault> {
    Ok(AsterVault::open(
        &resolved.path,
        resolved.vault_id,
        vault_salt(resolved.vault_id, &resolved.name),
        VaultOptions::default(),
    )?)
}

fn ingest_texts(resolved: &ResolvedVault, texts: &[String]) -> CliResult<Vec<IngestReport>> {
    if texts.is_empty() {
        return Ok(Vec::new());
    }
    let vault = open_vault(resolved)?;
    let state = load_vault_panel_state(&resolved.path)?;
    let mut staged = Vec::new();
    let mut prepared = Vec::with_capacity(texts.len());
    let mut first_new = BTreeSet::new();
    for text in texts {
        validate_text(text)?;
        let cx = measure_constellation(&vault, &state, text_input(text.clone()), now_ms())?;
        let new = !base_exists(&vault, cx.cx_id)? && first_new.insert(cx.cx_id);
        if new {
            staged.push(cx.clone());
        }
        prepared.push((cx.cx_id, new));
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
            append_cli_ledger(&vault, EntryKind::Ingest, cx_id, "cli-idempotent-ingest")?
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

fn append_cli_ledger(
    vault: &AsterVault,
    kind: EntryKind,
    cx_id: CxId,
    mode: &'static str,
) -> CliResult<u64> {
    let mut payload = PayloadBuilder::default();
    payload.insert_str("mode", mode);
    let bytes = RedactionPolicy::default().apply_to_payload(&payload);
    Ok(vault
        .append_ledger_entry(
            kind,
            SubjectId::Cx(cx_id),
            bytes,
            ActorId::Service("calyx-cli".to_string()),
        )?
        .seq)
}

fn measure_constellation(
    vault: &AsterVault,
    state: &VaultPanelState,
    input: Input,
    now: u64,
) -> CliResult<Constellation> {
    let cx_id = vault.cx_id_for_input(&input.bytes, state.panel.version);
    let mut slots = BTreeMap::new();
    let mut degraded = false;
    for slot in &state.panel.slots {
        let vector = if slot.state != SlotState::Active {
            absent(AbsentReason::LensInactive)
        } else if slot.modality != input.modality {
            absent(AbsentReason::NotApplicable)
        } else if !state.registry.contains(slot.lens_id) {
            absent(AbsentReason::LensUnavailable)
        } else {
            state.registry.measure(slot.lens_id, &input)?
        };
        degraded |= vector.is_absent();
        slots.insert(slot.slot_id, vector);
    }
    Ok(Constellation {
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
    })
}

fn read_batch_texts(path: &Path) -> CliResult<Vec<String>> {
    let raw = fs::read_to_string(path)?;
    let mut texts = Vec::new();
    for (index, line) in raw.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let parsed: BatchLine = serde_json::from_str(line).map_err(|err| {
            CliError::io(format!("batch JSONL line {} is invalid: {err}", index + 1))
        })?;
        validate_text(&parsed.text)?;
        texts.push(parsed.text);
    }
    Ok(texts)
}

fn parse_anchor_kind(value: &str) -> CliResult<AnchorKind> {
    Ok(match value {
        "test-pass" => AnchorKind::TestPass,
        "thumbs-up" | "thumbs-down" => AnchorKind::Thumbs,
        "speaker-match" => AnchorKind::SpeakerMatch,
        "style-hold" => AnchorKind::StyleHold,
        label if label.starts_with("label:") && label.len() > "label:".len() => {
            AnchorKind::Label(label["label:".len()..].to_string())
        }
        other => return Err(CliError::usage(format!("unknown anchor kind {other}"))),
    })
}

fn parse_anchor_value(kind: &AnchorKind, raw_kind: &str, value: &str) -> CliResult<AnchorValue> {
    match raw_kind {
        "thumbs-up" => return Ok(AnchorValue::Bool(true)),
        "thumbs-down" => return Ok(AnchorValue::Bool(false)),
        _ => {}
    }
    match kind {
        AnchorKind::TestPass => parse_bool(value, "--value").map(AnchorValue::Bool),
        AnchorKind::Label(_) => Ok(AnchorValue::Enum(value.to_string())),
        AnchorKind::SpeakerMatch | AnchorKind::StyleHold => Ok(parse_general_value(value)),
        AnchorKind::Thumbs
        | AnchorKind::TieFormed
        | AnchorKind::Reward
        | AnchorKind::Recurrence => Ok(parse_general_value(value)),
    }
}

fn parse_general_value(value: &str) -> AnchorValue {
    if let Ok(parsed) = value.parse::<bool>() {
        return AnchorValue::Bool(parsed);
    }
    if let Ok(parsed) = value.parse::<f64>()
        && parsed.is_finite()
    {
        return AnchorValue::Number(parsed);
    }
    AnchorValue::Text(value.to_string())
}

fn ensure_base_exists(vault: &AsterVault, cx_id: CxId) -> CliResult {
    if base_exists(vault, cx_id)? {
        return Ok(());
    }
    Err(CalyxError::vault_access_denied(format!("cx_id {cx_id} does not exist in vault")).into())
}

fn base_exists(vault: &AsterVault, cx_id: CxId) -> CliResult<bool> {
    Ok(vault
        .read_cf_at(vault.snapshot(), ColumnFamily::Base, &base_key(cx_id))?
        .is_some())
}

fn text_input(text: String) -> Input {
    Input::new(Modality::Text, text.into_bytes())
}

fn absent(reason: AbsentReason) -> SlotVector {
    SlotVector::Absent { reason }
}

fn validate_text(value: &str) -> CliResult {
    if value.is_empty() {
        return Err(CliError::usage("--text must not be empty"));
    }
    Ok(())
}

fn validate_confidence(value: f32) -> CliResult {
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        return Ok(());
    }
    Err(CliError::usage(
        "--confidence must be finite and within [0, 1]",
    ))
}

fn parse_bool(value: &str, flag: &str) -> CliResult<bool> {
    value
        .parse::<bool>()
        .map_err(|err| CliError::usage(format!("parse {flag} {value}: {err}")))
}

fn input_hash(bytes: &[u8]) -> [u8; 32] {
    *blake3::hash(bytes).as_bytes()
}

#[cfg(test)]
mod tests;
