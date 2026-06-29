use super::batch_support::{
    BatchOrderRow, append_idempotent_batch_ledger, append_missing_batch_anchors,
    append_oracle_events, current_anchor_kinds, ensure_idempotent_batch_replay,
    should_stage_batch_constellation,
};
use super::replay::{
    existing_batch_replay_rows, existing_plain_batch_replay_rows, flush_existing_batch_replay,
    flush_plain_existing_batch_replay, preflight_batch_existing_identity,
};
use super::*;

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
    ingest_validated_batch_streaming_with_output(resolved, path, output, validation.row_count, None)
}

pub(super) fn ingest_validated_batch_streaming_with_output(
    resolved: &ResolvedVault,
    path: &std::path::Path,
    output: IngestOutput,
    validated_row_count: usize,
    resident_addr: Option<std::net::SocketAddr>,
) -> CliResult<BatchIngestSummary> {
    use std::io::BufRead;
    let file = std::fs::File::open(path)
        .map_err(|err| CliError::io(format!("open batch {}: {err}", path.display())))?;
    let reader = std::io::BufReader::new(file);
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
    let mut seen = BTreeSet::new();
    let runtime_batch_limit = measure_batch_size();
    let measure_window = measure_window_size(runtime_batch_limit);
    let flush_options = BatchFlushOptions {
        output,
        runtime_batch_limit,
        resident_addr,
    };
    ingest_runtime_log(format_args!(
        "phase=batch_ingest_plan rows={} runtime_batch_limit={} measure_window={} put_chunk={} output={:?} resident_addr={:?}",
        validated_row_count, runtime_batch_limit, measure_window, PUT_CHUNK, output, resident_addr
    ));
    preflight_batch_existing_identity(&vault, &state, path, validated_row_count)?;
    let mut chunk: Vec<BatchRow> = Vec::with_capacity(measure_window);
    let mut summary = BatchIngestSummary::empty();
    for (index, line) in reader.lines().enumerate() {
        let line =
            line.map_err(|err| CliError::io(format!("read batch line {}: {err}", index + 1)))?;
        if let Some(row) = parse_batch_line(index, &line)? {
            chunk.push(row);
            if chunk.len() >= measure_window {
                flush_measure_batch(
                    &vault,
                    &state,
                    &mut chunk,
                    &mut seen,
                    &mut summary,
                    flush_options,
                )?;
            }
        }
    }
    if !chunk.is_empty() {
        flush_measure_batch(
            &vault,
            &state,
            &mut chunk,
            &mut seen,
            &mut summary,
            flush_options,
        )?;
    }
    if summary.new_count > 0 {
        ingest_runtime_log(format_args!(
            "phase=batch_index_rebuild_start new_count={} already_count={}",
            summary.new_count, summary.already_count
        ));
        rebuild_persistent_indexes(&resolved.path, &vault)?;
        ingest_runtime_log(format_args!(
            "phase=batch_index_rebuild_ok new_count={} already_count={}",
            summary.new_count, summary.already_count
        ));
    } else {
        ingest_runtime_log(format_args!(
            "phase=batch_index_rebuild_skip reason=no_new_constellations already_count={}",
            summary.already_count
        ));
    }
    Ok(summary)
}

fn flush_measure_batch(
    vault: &AsterVault,
    state: &VaultPanelState,
    chunk: &mut Vec<BatchRow>,
    seen: &mut BTreeSet<CxId>,
    summary: &mut BatchIngestSummary,
    options: BatchFlushOptions,
) -> CliResult<()> {
    let rows: Vec<BatchRow> = std::mem::take(chunk);
    if rows.iter().all(|(_, _, _, oracle)| oracle.is_none())
        && let Some(existing_rows) = existing_plain_batch_replay_rows(vault, state, &rows)?
    {
        ingest_runtime_log(format_args!(
            "phase=batch_existing_replay_base_only_fast_path rows={} runtime_batch_limit={} measurement_skipped=true slot_decode_skipped=true",
            existing_rows.len(),
            options.runtime_batch_limit
        ));
        flush_plain_existing_batch_replay(vault, existing_rows, summary, options.output)?;
        return Ok(());
    }
    if let Some(existing_rows) = existing_batch_replay_rows(vault, state, &rows)? {
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
        .map(|(text, _, _, _)| text_input(text.clone()))
        .collect();
    let constellations = measure_constellation_microbatch_with_runtime_limit(
        vault,
        state,
        &inputs,
        now_ms(),
        Some(options.runtime_batch_limit),
        options.resident_addr,
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
            summary.record(&report);
            if options.output == IngestOutput::Rows {
                print_json(&report)?;
            }
        }
        vault.flush()?;
    }
    Ok(())
}
