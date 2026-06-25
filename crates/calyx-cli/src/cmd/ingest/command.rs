use std::collections::{BTreeMap, BTreeSet};

use calyx_aster::vault::AsterVault;
use calyx_core::{Anchor, AnchorKind, CxId, Input, VaultStore};
use calyx_ledger::EntryKind;
use calyx_registry::{VaultPanelState, load_vault_panel_state};

use super::super::search::rebuild_persistent_indexes;
use super::super::vault::{ResolvedVault, now_ms};
use super::super::{AnchorArgs, IngestArgs, MeasureArgs, Subcommand};
use super::anchor::{parse_anchor_kind, parse_anchor_value};
use super::batch::{BatchRow, parse_batch_line};
use super::constellation::{
    ensure_content_panel_floor, measure_constellation, measure_constellation_microbatch, text_input,
};
use super::ledger::{append_anchor_ledger, append_anchor_marker_ledger, append_cli_ledger};
use super::oracle_event::{OracleEvent, append_recurrence_if_absent};
use super::store::{base_exists, ensure_base_exists, open_vault, resolve_cli_vault};
use super::types::{AnchorReport, IngestReport};
use crate::error::{CliError, CliResult};
use crate::output::print_json;
use crate::raw_media::{media_metadata, retain_media_input};

const DEFAULT_ANCHOR_SOURCE: &str = "calyx-cli";

/// Default inputs measured per GPU microbatch (one batched forward pass per lens).
/// Bigger = faster GPU utilization, but peak VRAM scales with the transient
/// attention/MLP activation buffers, which grow with `batch x sequence_len`: a
/// single unlucky microbatch of max-length rows can spike past VRAM and OOM
/// mid-ingest (an ingest crash also desyncs the vault ledger — see #866 — so a
/// crash is expensive, not just a retry). Measured on a 14-lens FP32 panel /
/// RTX 5090: batch=8 peaked ~32 GiB and OOM'd on long medmcqa rows, while batch=4
/// peaks ~19.6 GiB on the worst-case longest corpus rows (13 GiB headroom). So
/// the default is 4; raise `CALYX_MEASURE_BATCH` on a dedicated GPU / short inputs.
const DEFAULT_MEASURE_BATCH: usize = 4;
/// Constellations per WAL commit. Small because ColBERT multi-vectors are large;
/// decoupled from the measure batch so we measure big but commit WAL-safe.
const PUT_CHUNK: usize = 8;

/// Resolve the measure microbatch from `CALYX_MEASURE_BATCH` (>=1), else the
/// conservative default. Operator-tunable so the VRAM/throughput trade-off does
/// not require a recompile.
fn measure_batch_size() -> usize {
    std::env::var("CALYX_MEASURE_BATCH")
        .ok()
        .and_then(|raw| raw.parse::<usize>().ok())
        .filter(|&n| n >= 1)
        .unwrap_or(DEFAULT_MEASURE_BATCH)
}

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
    if let Some(path) = args.file {
        let modality = args.modality.expect("parser requires modality with --file");
        let retained = retain_media_input(&resolved.path, &path, modality)?;
        let metadata = media_metadata(&retained);
        let reports = ingest_prepared_inputs(
            &resolved,
            vec![PreparedInput {
                input: retained.input,
                metadata,
            }],
        )?;
        for report in reports {
            print_json(&report)?;
        }
    } else if let Some(text) = args.text {
        for report in ingest_texts(&resolved, &[text])? {
            print_json(&report)?;
        }
    } else {
        // Streaming batch path: warm models, WAL-safe chunked commits, bounded
        // memory — required for massive datasets (millions of rows).
        ingest_batch_streaming(
            &resolved,
            args.batch.as_deref().expect("validated batch path"),
        )?;
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
    let rows = texts
        .iter()
        .map(|text| (text.clone(), BTreeMap::new()))
        .collect();
    ingest_text_rows(resolved, rows)
}

pub(super) fn ingest_text_rows(
    resolved: &ResolvedVault,
    rows: Vec<(String, BTreeMap<String, String>)>,
) -> CliResult<Vec<IngestReport>> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let prepared = rows
        .into_iter()
        .map(|(text, metadata)| {
            super::parse::validate_text(&text)?;
            Ok(PreparedInput {
                input: text_input(text),
                metadata,
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
        ensure_content_panel_floor(&cx, &state)?;
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

pub(super) fn ingest_batch_streaming(
    resolved: &ResolvedVault,
    path: &std::path::Path,
) -> CliResult<()> {
    use std::io::BufRead;
    let file = std::fs::File::open(path)
        .map_err(|err| CliError::io(format!("open batch {}: {err}", path.display())))?;
    let reader = std::io::BufReader::new(file);
    let vault = open_vault(resolved)?;
    let state = load_vault_panel_state(&resolved.path)?;
    let mut seen = BTreeSet::new();
    let measure_batch = measure_batch_size();
    let mut chunk: Vec<BatchRow> = Vec::with_capacity(measure_batch);
    for (index, line) in reader.lines().enumerate() {
        let line =
            line.map_err(|err| CliError::io(format!("read batch line {}: {err}", index + 1)))?;
        if let Some(row) = parse_batch_line(index, &line)? {
            chunk.push(row);
            if chunk.len() >= measure_batch {
                flush_measure_batch(&vault, &state, &mut chunk, &mut seen)?;
            }
        }
    }
    if !chunk.is_empty() {
        flush_measure_batch(&vault, &state, &mut chunk, &mut seen)?;
    }
    rebuild_persistent_indexes(&resolved.path, &vault)?;
    Ok(())
}

fn flush_measure_batch(
    vault: &AsterVault,
    state: &VaultPanelState,
    chunk: &mut Vec<BatchRow>,
    seen: &mut BTreeSet<CxId>,
) -> CliResult<()> {
    let rows: Vec<BatchRow> = std::mem::take(chunk);
    let inputs: Vec<Input> = rows
        .iter()
        .map(|(text, _, _, _)| text_input(text.clone()))
        .collect();
    let constellations = measure_constellation_microbatch(vault, state, &inputs, now_ms())?;
    let mut measured = Vec::with_capacity(constellations.len());
    for (mut cx, (_, mut metadata, anchors, oracle)) in constellations.into_iter().zip(rows) {
        if let Some(event) = &oracle {
            event.apply_metadata(&mut metadata)?;
        }
        cx.metadata = metadata;
        // A constellation carrying its own anchor is grounded at distance 0; mirror
        // the canonical `ungrounded = anchors.is_empty()` rule (dedup/ingest_input.rs)
        // so the flag reflects reality rather than the measure-time default of true.
        cx.flags.ungrounded = anchors.is_empty();
        cx.anchors = anchors;
        measured.push((cx, oracle));
    }
    // Doctrine #1273 rule 3: validate the whole flush before any put so a fully
    // degraded constellation aborts the batch loudly instead of being persisted.
    for (cx, _) in &measured {
        ensure_content_panel_floor(cx, state)?;
    }
    for sub in measured.chunks(PUT_CHUNK) {
        let mut staged = Vec::new();
        let mut order = Vec::with_capacity(sub.len());
        let mut known_anchor_kinds = BTreeMap::<CxId, BTreeSet<AnchorKind>>::new();
        for (cx, oracle) in sub {
            let exists = base_exists(vault, cx.cx_id)?;
            let new = !exists && seen.insert(cx.cx_id);
            let known = match known_anchor_kinds.entry(cx.cx_id) {
                std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(current_anchor_kinds(vault, cx.cx_id, exists)?)
                }
            };
            let mut marker_kinds = Vec::new();
            for anchor in &cx.anchors {
                if known.insert(anchor.kind.clone()) {
                    marker_kinds.push(anchor.kind.clone());
                }
            }
            if new || !cx.anchors.is_empty() {
                staged.push(cx.clone());
            }
            order.push((cx.cx_id, new, marker_kinds, oracle.clone()));
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
        append_oracle_events(vault, &order)?;
        let snapshot = vault.snapshot();
        for (cx_id, new, marker_kinds, _) in order {
            let ledger_seq = if new {
                vault.get(cx_id, snapshot)?.provenance.seq
            } else {
                append_cli_ledger(vault, EntryKind::Ingest, cx_id, "cli-idempotent-ingest")?
            };
            for kind in marker_kinds {
                append_anchor_marker_ledger(vault, cx_id, &kind)?;
            }
            print_json(&IngestReport {
                cx_id: cx_id.to_string(),
                new,
                ledger_seq,
            })?;
        }
        vault.flush()?;
    }
    Ok(())
}

fn append_oracle_events(
    vault: &AsterVault,
    order: &[(CxId, bool, Vec<AnchorKind>, Option<OracleEvent>)],
) -> CliResult<()> {
    for (cx_id, _, _, oracle) in order {
        if let Some(event) = oracle {
            append_recurrence_if_absent(vault, *cx_id, event, now_ms())?;
        }
    }
    Ok(())
}

fn current_anchor_kinds(
    vault: &AsterVault,
    cx_id: CxId,
    exists: bool,
) -> CliResult<BTreeSet<AnchorKind>> {
    if !exists {
        return Ok(BTreeSet::new());
    }
    Ok(vault
        .get(cx_id, vault.snapshot())?
        .anchors
        .into_iter()
        .map(|anchor| anchor.kind)
        .collect())
}
