use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;

use calyx_aster::cf::{ColumnFamily, anchor_key, base_key};
use calyx_aster::dedup::{AnchorConflictResult, check_anchor_conflict};
use calyx_aster::vault::AsterVault;
use calyx_aster::vault::encode::{self, decode_constellation_base};
use calyx_core::{
    Anchor, AnchorKind, Constellation, CxId, Input, InputRef, Modality, SlotState, VaultStore,
};
use calyx_ledger::{ActorId, EntryKind, SubjectId};
use calyx_registry::{VaultPanelState, load_vault_panel_state};

use super::super::search::{rebuild_persistent_indexes, rebuild_persistent_indexes_with_progress};
use super::super::vault::{ResolvedVault, now_ms};
use super::super::{AnchorArgs, IngestArgs, MeasureArgs, Subcommand};
use super::anchor::{parse_anchor_kind, parse_anchor_value};
use super::batch::{BatchRow, parse_batch_line, validate_batch_file};
use super::constellation::{
    ensure_content_panel_floor, input_hash, measure_constellation,
    measure_constellation_microbatch_with_runtime_limit, text_input,
};
use super::ledger::{
    append_anchor_ledger, append_anchor_marker_ledger, append_cli_batch_ledger, append_cli_ledger,
};
use super::oracle_event::{OracleEvent, append_recurrence_if_absent};
use super::store::{base_exists, ensure_base_exists, open_vault, resolve_cli_vault};
use super::types::{AnchorReport, BatchIngestSummary, IngestOutput, IngestReport};
use super::verify::verify_base_readback;
use crate::error::{CliError, CliResult};
use crate::media_derived_text::{
    derivation_ledger_payload, derive_text_for_media, derived_artifact_draft,
};
use crate::output::print_json;
use crate::raw_media::{media_metadata, retain_media_input};

const DEFAULT_ANCHOR_SOURCE: &str = "calyx-cli";

/// Default inputs per real runtime call inside a lens worker. This is a CUDA
/// safety limit, not a file-streaming flush size. Bigger = faster GPU
/// utilization, but peak VRAM scales with the transient attention/MLP activation
/// buffers, which grow with `batch x sequence_len`: a single unlucky microbatch
/// of max-length rows can spike past VRAM and OOM mid-ingest (an ingest crash
/// also desyncs the vault ledger — see #866 — so a crash is expensive, not just a
/// retry). Measured on a 14-lens FP32 panel / RTX 5090: batch=8 peaked ~32 GiB
/// and OOM'd on long medmcqa rows, while batch=4 peaks ~19.6 GiB on the
/// worst-case longest corpus rows (13 GiB headroom). So the default is 4; raise
/// `CALYX_MEASURE_BATCH` on a dedicated GPU / short inputs.
const DEFAULT_MEASURE_BATCH: usize = 4;
/// JSONL rows gathered before measurement. Lenses still receive
/// `CALYX_MEASURE_BATCH`-bounded runtime chunks inside the worker, but a larger
/// window prevents a small ingest from spawning one process per lens per 4 rows.
const DEFAULT_MEASURE_WINDOW: usize = 128;
/// Constellations per WAL commit. Small because ColBERT multi-vectors are large;
/// decoupled from the measure batch so we measure big but commit WAL-safe.
const PUT_CHUNK: usize = 8;
/// Existing-row replay does not stage vector payloads, so it can verify and ledger
/// larger groups without the ColBERT WAL pressure that constrains new puts.
const EXISTING_REPLAY_CHUNK: usize = 128;
const MEASURE_BATCH_ENV: &str = "CALYX_MEASURE_BATCH";
const MEASURE_WINDOW_ENV: &str = "CALYX_INGEST_MEASURE_WINDOW";

#[derive(Clone, Copy)]
struct BatchFlushOptions {
    output: IngestOutput,
    runtime_batch_limit: usize,
    resident_addr: Option<std::net::SocketAddr>,
}

pub(crate) fn ingest_runtime_log(args: std::fmt::Arguments<'_>) {
    let mut stderr = std::io::stderr().lock();
    let _ = writeln!(stderr, "CALYX_INGEST_RUNTIME {args}");
    let _ = stderr.flush();
}

/// Resolve the runtime microbatch from `CALYX_MEASURE_BATCH` (>=1), else the
/// conservative default. Operator-tunable so the VRAM/throughput trade-off does
/// not require a recompile.
fn measure_batch_size() -> usize {
    positive_env_usize(MEASURE_BATCH_ENV).unwrap_or(DEFAULT_MEASURE_BATCH)
}

fn measure_window_size(runtime_batch_limit: usize) -> usize {
    positive_env_usize(MEASURE_WINDOW_ENV)
        .unwrap_or(DEFAULT_MEASURE_WINDOW)
        .max(runtime_batch_limit.max(1))
}

fn positive_env_usize(name: &str) -> Option<usize> {
    std::env::var(name)
        .ok()
        .and_then(|raw| raw.parse::<usize>().ok())
        .filter(|&n| n >= 1)
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
    if let Some(batch_path) = args.batch.as_deref() {
        let validation = validate_batch_file(batch_path)?;
        let resolved = resolve_cli_vault(&args.vault)?;
        let mut emitted_summary = false;
        let summary = if validation.row_count == 0 {
            BatchIngestSummary::empty()
        } else if args.output == IngestOutput::Summary {
            let mut emit_summary = |summary: &BatchIngestSummary| {
                emitted_summary = true;
                print_json(summary)
            };
            batch_stream::ingest_validated_batch_streaming_with_output(
                &resolved,
                batch_path,
                args.output,
                validation.row_count,
                args.resident_addr,
                Some(&mut emit_summary),
            )?
        } else {
            batch_stream::ingest_validated_batch_streaming_with_output(
                &resolved,
                batch_path,
                args.output,
                validation.row_count,
                args.resident_addr,
                None,
            )?
        };
        if args.output == IngestOutput::Summary && !emitted_summary {
            print_json(&summary)?;
        }
    } else {
        let resolved = resolve_cli_vault(&args.vault)?;
        if let Some(path) = args.file {
            let modality = args.modality.expect("parser requires modality with --file");
            let retained = retain_media_input(&resolved.path, &path, modality)?;
            let reports = ingest_media_with_derived_text(&resolved, retained)?;
            for report in reports {
                print_json(&report)?;
            }
        } else if let Some(text) = args.text {
            for report in ingest_texts(&resolved, &[text])? {
                print_json(&report)?;
            }
        }
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
    ingest_runtime_log(format_args!(
        "phase=load_vault_panel_state_start vault={}",
        resolved.path.display()
    ));
    let state = load_vault_panel_state(&resolved.path)?;
    ingest_runtime_log(format_args!(
        "phase=load_vault_panel_state_ok vault={} panel_version={} slots={}",
        resolved.path.display(),
        state.panel.version,
        state.panel.slots.len()
    ));
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

fn ingest_media_with_derived_text(
    resolved: &ResolvedVault,
    retained: crate::raw_media::RetainedMediaInput,
) -> CliResult<Vec<IngestReport>> {
    let vault = open_vault(resolved)?;
    ingest_runtime_log(format_args!(
        "phase=load_vault_panel_state_start vault={}",
        resolved.path.display()
    ));
    let state = load_vault_panel_state(&resolved.path)?;
    ingest_runtime_log(format_args!(
        "phase=load_vault_panel_state_ok vault={} panel_version={} slots={}",
        resolved.path.display(),
        state.panel.version,
        state.panel.slots.len()
    ));

    ensure_raw_media_panel_route(retained.input.modality, &state)?;
    let source_cx_id = vault.cx_id_for_input(&retained.input.bytes, state.panel.version);
    let derived = derive_text_for_media(&resolved.path, &retained, source_cx_id)?;

    let mut media_cx = measure_constellation(&vault, &state, retained.input.clone(), now_ms())?;
    media_cx.metadata = media_metadata(&retained);
    ensure_content_panel_floor(&media_cx, &state)?;
    let mut text_cx = measure_constellation(&vault, &state, derived.input.clone(), now_ms())?;
    text_cx.metadata = derived.metadata.clone();
    ensure_content_panel_floor(&text_cx, &state)?;

    let media_new = !base_exists(&vault, media_cx.cx_id)?;
    let text_new = !base_exists(&vault, text_cx.cx_id)?;
    let payload = derivation_ledger_payload(&retained, &derived, media_cx.cx_id, text_cx.cx_id)?;
    let mut staged = Vec::with_capacity(2);
    if media_new {
        staged.push(media_cx.clone());
    }
    if text_new && text_cx.cx_id != media_cx.cx_id {
        staged.push(text_cx.clone());
    }
    let artifact_draft =
        derived_artifact_draft(&retained, &derived, media_cx.cx_id, text_cx.cx_id)?;
    let commit = vault.put_batch_with_ingest_ledger_and_media_artifact(
        staged,
        SubjectId::Cx(text_cx.cx_id),
        payload,
        ActorId::Service("calyx-cli".to_string()),
        artifact_draft,
    )?;
    vault.flush()?;
    rebuild_persistent_indexes(&resolved.path, &vault)?;

    let snapshot = vault.snapshot();
    if media_new {
        verify_base_readback(&vault, snapshot, &media_cx, media_cx.cx_id, &[])?;
    } else {
        verify_existing_media_or_text_readback(&vault, snapshot, &media_cx)?;
    }
    if text_new {
        verify_base_readback(&vault, snapshot, &text_cx, text_cx.cx_id, &[])?;
    } else {
        verify_existing_media_or_text_readback(&vault, snapshot, &text_cx)?;
    }
    verify_media_artifact_readback(&vault, snapshot, &commit.artifact)?;

    let media_ledger_seq = if media_new {
        vault.get(media_cx.cx_id, snapshot)?.provenance.seq
    } else {
        commit.artifact.ledger_ref.seq
    };
    let text_ledger_seq = if text_new {
        vault.get(text_cx.cx_id, snapshot)?.provenance.seq
    } else {
        commit.artifact.ledger_ref.seq
    };
    vault.flush()?;
    Ok(vec![
        IngestReport {
            cx_id: media_cx.cx_id.to_string(),
            new: media_new,
            ledger_seq: media_ledger_seq,
        },
        IngestReport {
            cx_id: text_cx.cx_id.to_string(),
            new: text_new,
            ledger_seq: text_ledger_seq,
        },
    ])
}

fn ensure_raw_media_panel_route(modality: Modality, state: &VaultPanelState) -> CliResult {
    if !matches!(
        modality,
        Modality::Image | Modality::Audio | Modality::Video
    ) {
        return Ok(());
    }
    let has_declared_route = state
        .panel
        .slots
        .iter()
        .any(|slot| slot.state == SlotState::Active && slot.counts_toward_degraded(modality));
    if has_declared_route {
        return Ok(());
    }
    Err(calyx_core::CalyxError {
        code: "CALYX_MEDIA_ROUTE_UNAVAILABLE",
        message: format!(
            "raw {modality:?} ingest requires an active {modality:?} content lens before derived text can be attached"
        ),
        remediation:
            "add or activate an image/audio/video lens for the raw media modality, then re-run ingest so the media constellation is measured instead of empty",
    }
    .into())
}

fn verify_existing_media_or_text_readback(
    vault: &AsterVault,
    snapshot: u64,
    expected: &calyx_core::Constellation,
) -> CliResult {
    let stored = vault.get(expected.cx_id, snapshot)?;
    if stored.panel_version != expected.panel_version
        || stored.input_ref.hash != expected.input_ref.hash
        || stored.modality != expected.modality
        || stored.slots != expected.slots
    {
        return Err(calyx_core::CalyxError::aster_corrupt_shard(format!(
            "durable media ingest readback mismatch for existing cx {}",
            expected.cx_id
        ))
        .into());
    }
    Ok(())
}

fn verify_media_artifact_readback(
    vault: &AsterVault,
    snapshot: u64,
    expected: &calyx_aster::media_artifact::DerivedMediaArtifactRecord,
) -> CliResult {
    let stored = vault
        .get_derived_media_artifact(snapshot, &expected.artifact_id)?
        .ok_or_else(|| {
            calyx_core::CalyxError::aster_corrupt_shard(format!(
                "derived media artifact {} missing after commit",
                expected.artifact_id
            ))
        })?;
    if stored != *expected {
        return Err(calyx_core::CalyxError::aster_corrupt_shard(format!(
            "derived media artifact {} readback mismatch",
            expected.artifact_id
        ))
        .into());
    }
    let source_records =
        vault.derived_media_artifacts_for_source(snapshot, expected.source_cx_id)?;
    if !source_records.iter().any(|record| record == expected) {
        return Err(calyx_core::CalyxError::aster_corrupt_shard(format!(
            "derived media artifact {} missing from source index",
            expected.artifact_id
        ))
        .into());
    }
    let target_records =
        vault.derived_media_artifacts_for_target(snapshot, expected.target_cx_id)?;
    if !target_records.iter().any(|record| record == expected) {
        return Err(calyx_core::CalyxError::aster_corrupt_shard(format!(
            "derived media artifact {} missing from target index",
            expected.artifact_id
        ))
        .into());
    }
    Ok(())
}

mod batch_physical;
mod batch_stream;
mod batch_support;
mod replay;

#[cfg(test)]
pub(super) use batch_stream::{
    ingest_batch_streaming, ingest_batch_streaming_with_summary_emitter,
};
#[cfg(test)]
pub(crate) use batch_support::should_stage_batch_constellation;
