use super::super::session::BatchIngestSession;
use super::batch_physical::{
    collect_batch_cx_ids, physical_batch_base_state, reconcile_summary_with_physical_base,
    reject_tombstoned_batch_ids,
};
use super::batch_support::{
    BatchOrderRow, append_idempotent_batch_ledger, append_missing_batch_anchors,
    append_oracle_events, current_anchor_kinds, ensure_idempotent_batch_replay,
    should_stage_batch_constellation,
};
use super::replay::{
    backfill_batch_existing_input_pointers, existing_batch_replay_rows,
    existing_plain_batch_replay_rows, flush_existing_batch_replay,
    flush_plain_existing_batch_replay, preflight_batch_existing_identity,
};
use super::*;

type BatchSummaryEmitter<'a> = &'a mut dyn FnMut(&BatchIngestSummary) -> CliResult<()>;

#[cfg(test)]
pub(crate) fn ingest_batch_streaming(
    resolved: &ResolvedVault,
    path: &std::path::Path,
) -> CliResult<BatchIngestSummary> {
    ingest_batch_streaming_with_output(resolved, path, IngestOutput::Summary)
}

#[cfg(test)]
pub(crate) fn ingest_batch_streaming_with_output(
    resolved: &ResolvedVault,
    path: &std::path::Path,
    output: IngestOutput,
) -> CliResult<BatchIngestSummary> {
    let validation = validate_batch_file(path)?;
    if validation.row_count == 0 {
        return Ok(BatchIngestSummary::empty());
    }
    ingest_validated_batch_streaming_with_output(
        resolved,
        path,
        output,
        validation.row_count,
        IngestGpuRoute::cold_workers_allowed(),
        None,
        None,
    )
}

#[cfg(test)]
pub(crate) fn ingest_batch_streaming_with_summary_emitter(
    resolved: &ResolvedVault,
    path: &std::path::Path,
    summary_emitter: &mut dyn FnMut(&BatchIngestSummary) -> CliResult<()>,
) -> CliResult<BatchIngestSummary> {
    let validation = validate_batch_file(path)?;
    if validation.row_count == 0 {
        return Ok(BatchIngestSummary::empty());
    }
    ingest_validated_batch_streaming_with_output(
        resolved,
        path,
        IngestOutput::Summary,
        validation.row_count,
        IngestGpuRoute::cold_workers_allowed(),
        Some(summary_emitter),
        None,
    )
}

pub(crate) fn ingest_validated_batch_streaming_with_output(
    resolved: &ResolvedVault,
    path: &std::path::Path,
    output: IngestOutput,
    validated_row_count: usize,
    gpu_route: IngestGpuRoute,
    mut summary_emitter: Option<BatchSummaryEmitter<'_>>,
    mut session: Option<&mut BatchIngestSession>,
) -> CliResult<BatchIngestSummary> {
    use std::io::BufRead;
    let file = std::fs::File::open(path)
        .map_err(|err| CliError::io(format!("open batch {}: {err}", path.display())))?;
    let reader = std::io::BufReader::new(file);
    let open_started = std::time::Instant::now();
    if let Some(session) = session.as_deref_mut() {
        session.record_phase("open_vault_start")?;
    }
    ingest_runtime_log(format_args!(
        "phase=open_vault_start vault={} rows={validated_row_count}",
        resolved.path.display()
    ));
    let vault = match open_vault(resolved) {
        Ok(vault) => {
            let recovery = vault.recovery_report();
            ingest_runtime_log(format_args!(
                "phase=open_vault_ok vault={} last_recovered_seq={} torn_tail={} elapsed_ms={}",
                resolved.path.display(),
                recovery.last_recovered_seq,
                recovery.torn_tail.is_some(),
                open_started.elapsed().as_millis()
            ));
            if let Some(session) = session.as_deref_mut() {
                session.record_phase("open_vault_ok")?;
            }
            vault
        }
        Err(error) => {
            ingest_runtime_log(format_args!(
                "phase=open_vault_error vault={} error_code={} error_message_json={} elapsed_ms={}",
                resolved.path.display(),
                error.code(),
                json_string(error.message()),
                open_started.elapsed().as_millis()
            ));
            return Err(error);
        }
    };
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
    if let Some(session) = session.as_deref_mut() {
        session.record_phase("load_vault_panel_state_ok")?;
    }
    let mut seen = BTreeSet::new();
    let runtime_batch_limit = measure_batch_size();
    let measure_window = measure_window_size(runtime_batch_limit);
    let flush_options = BatchFlushOptions {
        output,
        runtime_batch_limit,
        gpu_route,
    };
    ingest_runtime_log(format_args!(
        "phase=batch_ingest_plan rows={} runtime_batch_limit={} measure_window={} put_chunk={} output={:?} resident_addr={:?} allow_cold_gpu_workers={}",
        validated_row_count,
        runtime_batch_limit,
        measure_window,
        PUT_CHUNK,
        output,
        gpu_route.resident_addr,
        gpu_route.allow_cold_gpu_workers
    ));
    preflight_batch_existing_identity(&vault, &state, &resolved.path, path, validated_row_count)?;
    let batch_cx_ids = collect_batch_cx_ids(&vault, &state, path)?;
    let physical_before = physical_batch_base_state(&resolved.path, &batch_cx_ids)?;
    reject_tombstoned_batch_ids(&physical_before)?;
    ingest_runtime_log(format_args!(
        "phase=batch_physical_base_readback_before distinct_cx={} visible={} tombstoned={}",
        batch_cx_ids.len(),
        physical_before.visible.len(),
        physical_before.tombstoned.len()
    ));
    if let Some(session) = session.as_deref_mut() {
        session.record_phase("batch_physical_base_readback_before")?;
    }
    stake_rebuild_required_marker(
        &resolved.path,
        "batch_ingest",
        format!(
            "batch ingest of {validated_row_count} planned rows ({} distinct constellations) from {}; derived search indexes are unproven until the post-commit rebuild republishes the manifest",
            batch_cx_ids.len(),
            path.display()
        ),
        session.as_deref().map(|session| session.session_id()),
        Some(path),
    )?;
    backfill_batch_existing_input_pointers(&vault, &state, &resolved.path, path)?;
    let mut chunk: Vec<BatchRow> = Vec::with_capacity(measure_window);
    let mut summary = BatchIngestSummary::empty();
    for (index, line) in reader.lines().enumerate() {
        let line =
            line.map_err(|err| CliError::io(format!("read batch line {}: {err}", index + 1)))?;
        if let Some(row) = parse_batch_line(index, &line)? {
            chunk.push(row);
            if chunk.len() >= measure_window {
                if let Some(session) = session.as_deref_mut() {
                    session.record_rows_started(
                        summary.row_count + chunk.len(),
                        "batch_flush_start",
                    )?;
                }
                flush_measure_batch(
                    &vault,
                    &state,
                    &resolved.path,
                    &mut chunk,
                    &mut seen,
                    &mut summary,
                    flush_options,
                )?;
                if let Some(session) = session.as_deref_mut() {
                    session.record_summary_progress(&summary, "batch_flush_committed")?;
                }
            }
        }
    }
    if !chunk.is_empty() {
        if let Some(session) = session.as_deref_mut() {
            session.record_rows_started(summary.row_count + chunk.len(), "batch_flush_start")?;
        }
        flush_measure_batch(
            &vault,
            &state,
            &resolved.path,
            &mut chunk,
            &mut seen,
            &mut summary,
            flush_options,
        )?;
        if let Some(session) = session.as_deref_mut() {
            session.record_summary_progress(&summary, "batch_flush_committed")?;
        }
    }
    let physical_after = physical_batch_base_state(&resolved.path, &batch_cx_ids)?;
    reconcile_summary_with_physical_base(&mut summary, &physical_before, &physical_after)?;
    if let Some(session) = session.as_deref_mut() {
        session.record_summary_progress(&summary, "batch_physical_base_readback_after")?;
    }
    let summary_emit_error = emit_batch_summary_if_requested(&mut summary_emitter, &summary)?;
    batch_rebuild::run_post_commit_index_rebuild(resolved, &vault, &summary, &mut session)?;
    if let Some(session) = session {
        session.complete(&summary, vault.snapshot())?;
    }
    if let Some(error) = summary_emit_error {
        return Err(error);
    }
    Ok(summary)
}

fn emit_batch_summary_if_requested(
    summary_emitter: &mut Option<BatchSummaryEmitter<'_>>,
    summary: &BatchIngestSummary,
) -> CliResult<Option<CliError>> {
    let Some(emitter) = summary_emitter.as_mut() else {
        return Ok(None);
    };
    match (*emitter)(summary) {
        Ok(()) => {
            ingest_runtime_log(format_args!(
                "phase=batch_summary_emitted row_count={} new_count={} already_count={} verified_base_rows={}",
                summary.row_count,
                summary.new_count,
                summary.already_count,
                summary.verified_base_rows
            ));
            Ok(None)
        }
        Err(error) => {
            ingest_runtime_log(format_args!(
                "phase=batch_summary_emit_error error_code={} error_message_json={} row_count={} new_count={} already_count={}",
                error.code(),
                json_string(error.message()),
                summary.row_count,
                summary.new_count,
                summary.already_count
            ));
            Ok(Some(error))
        }
    }
}

fn flush_measure_batch(
    vault: &AsterVault,
    state: &VaultPanelState,
    vault_path: &std::path::Path,
    chunk: &mut Vec<BatchRow>,
    seen: &mut BTreeSet<CxId>,
    summary: &mut BatchIngestSummary,
    options: BatchFlushOptions,
) -> CliResult<()> {
    let rows: Vec<BatchRow> = std::mem::take(chunk);
    if rows.iter().all(|(_, _, _, oracle)| oracle.is_none())
        && let Some(existing_rows) =
            existing_plain_batch_replay_rows(vault, state, vault_path, &rows)?
    {
        ingest_runtime_log(format_args!(
            "phase=batch_existing_replay_base_only_fast_path rows={} runtime_batch_limit={} measurement_skipped=true slot_decode_skipped=true",
            existing_rows.len(),
            options.runtime_batch_limit
        ));
        flush_plain_existing_batch_replay(
            vault,
            vault_path,
            existing_rows,
            summary,
            options.output,
        )?;
        return Ok(());
    }
    if let Some(existing_rows) = existing_batch_replay_rows(vault, state, vault_path, &rows)? {
        ingest_runtime_log(format_args!(
            "phase=batch_existing_replay_fast_path rows={} runtime_batch_limit={} measurement_skipped=true",
            existing_rows.len(),
            options.runtime_batch_limit
        ));
        flush_existing_batch_replay(vault, state, existing_rows, summary, options.output)?;
        return Ok(());
    }
    let inputs: Vec<Input> = rows
        .iter()
        .map(|(text, _, _, _)| retention::retained_text_input(vault_path, text))
        .collect::<CliResult<_>>()?;
    let constellations = measure_constellation_microbatch_with_runtime_limit(
        vault,
        state,
        &inputs,
        now_ms(),
        Some(options.runtime_batch_limit),
        options.gpu_route,
    )?;
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
            let existing = if exists {
                Some(ensure_idempotent_batch_replay(vault, cx)?)
            } else {
                None
            };
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
            let mut expected_readback = existing.as_ref().cloned().unwrap_or_else(|| cx.clone());
            if should_stage_batch_constellation(new, &marker_kinds) {
                if new {
                    staged.push(cx.clone());
                    expected_readback = cx.clone();
                } else if let Some(existing) = existing.as_ref() {
                    expected_readback =
                        append_missing_batch_anchors(vault, existing, cx, &marker_kinds)?;
                }
            }
            order.push(BatchOrderRow {
                cx_id: cx.cx_id,
                expected_readback,
                new,
                marker_kinds,
                oracle: oracle.clone(),
            });
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
        for row in &order {
            verify_base_readback(
                vault,
                snapshot,
                &row.expected_readback,
                row.cx_id,
                &row.marker_kinds,
            )?;
        }
        append_oracle_events(vault, &order)?;
        let idempotent_ledger_seq = append_idempotent_batch_ledger(vault, &order)?;
        for row in order {
            let cx_id = row.cx_id;
            let ledger_seq = if row.new {
                vault.get(cx_id, snapshot)?.provenance.seq
            } else {
                idempotent_ledger_seq.ok_or_else(|| {
                    CliError::usage("missing idempotent batch ledger seq for replay row")
                })?
            };
            for kind in row.marker_kinds {
                append_anchor_marker_ledger(vault, cx_id, &kind)?;
            }
            let report = IngestReport {
                cx_id: cx_id.to_string(),
                new: row.new,
                ledger_seq,
            };
            summary.record(cx_id, &report);
            if options.output == IngestOutput::Rows {
                print_json(&report)?;
            }
        }
        vault.flush()?;
    }
    Ok(())
}
