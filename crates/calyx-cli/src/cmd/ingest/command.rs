use std::collections::{BTreeMap, BTreeSet};

use calyx_core::{Anchor, CxId, Input, VaultStore};
use calyx_ledger::EntryKind;
use calyx_registry::load_vault_panel_state;

use super::super::search::rebuild_persistent_indexes;
use super::super::vault::{ResolvedVault, now_ms};
use super::super::{AnchorArgs, IngestArgs, MeasureArgs, Subcommand};
use super::anchor::{parse_anchor_kind, parse_anchor_value};
use super::batch::read_batch_texts;
use super::constellation::{measure_constellation, text_input};
use super::ledger::{append_anchor_ledger, append_cli_ledger};
use super::store::{base_exists, ensure_base_exists, open_vault, resolve_cli_vault};
use super::types::{AnchorReport, IngestReport};
use crate::error::{CliError, CliResult};
use crate::output::print_json;
use crate::raw_media::{media_metadata, retain_media_input};

const DEFAULT_ANCHOR_SOURCE: &str = "calyx-cli";

pub(crate) fn run(command: Subcommand) -> CliResult {
    match command {
        Subcommand::Ingest(args) => ingest_command(args),
        Subcommand::Anchor(args) => anchor_command(args),
        Subcommand::Measure(args) => measure_command(args),
        _ => unreachable!("non-ingest command routed to ingest module"),
    }
}

fn ingest_command(args: IngestArgs) -> CliResult {
    let resolved = resolve_cli_vault(&args.vault)?;
    let reports = if let Some(path) = args.file {
        let modality = args.modality.expect("parser requires modality with --file");
        let retained = retain_media_input(&resolved.path, &path, modality)?;
        let metadata = media_metadata(&retained);
        ingest_prepared_inputs(
            &resolved,
            vec![PreparedInput {
                input: retained.input,
                metadata,
            }],
        )?
    } else if let Some(text) = args.text {
        ingest_texts(&resolved, &[text])?
    } else {
        let texts = read_batch_texts(args.batch.as_deref().expect("validated batch path"))?;
        ingest_texts(&resolved, &texts)?
    };
    for report in reports {
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
        kind: kind.clone(),
        source: args
            .source
            .unwrap_or_else(|| DEFAULT_ANCHOR_SOURCE.to_string()),
        observed_at: now_ms(),
        confidence: args.confidence.unwrap_or(1.0),
    };
    let ledger_seq = append_anchor_ledger(&vault, cx_id, &kind, anchor)?;
    vault.flush()?;
    rebuild_persistent_indexes(&resolved.path, &vault)?;
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

pub(super) fn ingest_texts(
    resolved: &ResolvedVault,
    texts: &[String],
) -> CliResult<Vec<IngestReport>> {
    if texts.is_empty() {
        return Ok(Vec::new());
    }
    let prepared = texts
        .iter()
        .map(|text| {
            super::parse::validate_text(text)?;
            Ok(PreparedInput {
                input: text_input(text.clone()),
                metadata: BTreeMap::new(),
            })
        })
        .collect::<CliResult<Vec<_>>>()?;
    ingest_prepared_inputs(resolved, prepared)
}

struct PreparedInput {
    input: Input,
    metadata: BTreeMap<String, String>,
}

fn ingest_prepared_inputs(
    resolved: &ResolvedVault,
    inputs: Vec<PreparedInput>,
) -> CliResult<Vec<IngestReport>> {
    if inputs.is_empty() {
        return Ok(Vec::new());
    }
    let vault = open_vault(resolved)?;
    let state = load_vault_panel_state(&resolved.path)?;
    let mut staged = Vec::new();
    let mut prepared = Vec::with_capacity(inputs.len());
    let mut first_new = BTreeSet::new();
    for prepared_input in inputs {
        let mut cx = measure_constellation(&vault, &state, prepared_input.input, now_ms())?;
        cx.metadata = prepared_input.metadata;
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
    rebuild_persistent_indexes(&resolved.path, &vault)?;
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
