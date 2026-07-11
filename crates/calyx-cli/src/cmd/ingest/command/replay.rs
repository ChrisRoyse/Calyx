use super::batch_support::{
    BatchOrderRow, IdentityFields, append_idempotent_batch_ledger, append_missing_batch_anchors,
    append_oracle_events, current_anchor_kinds, existing_replay_incoming, identity_mismatch_reason,
    verify_existing_batch_replay_identity,
};
use super::*;

pub(crate) fn preflight_batch_existing_identity(
    vault: &AsterVault,
    state: &VaultPanelState,
    vault_path: &std::path::Path,
    path: &std::path::Path,
    validated_row_count: usize,
) -> CliResult<()> {
    use std::io::BufRead;

    let started = std::time::Instant::now();
    ingest_runtime_log(format_args!(
        "phase=batch_existing_identity_preflight_start rows={validated_row_count}"
    ));
    let file = std::fs::File::open(path)
        .map_err(|err| CliError::io(format!("open batch {}: {err}", path.display())))?;
    let reader = std::io::BufReader::new(file);
    let snapshot = vault.snapshot();
    let mut checked_existing = 0_usize;
    let mut not_existing_or_incomplete = 0_usize;
    for (index, line) in reader.lines().enumerate() {
        let line =
            line.map_err(|err| CliError::io(format!("read batch line {}: {err}", index + 1)))?;
        let Some((text, mut metadata, anchors, oracle)) = parse_batch_line(index, &line)? else {
            continue;
        };
        if let Some(event) = &oracle {
            event.apply_metadata(&mut metadata)?;
        }
        let input = retention::retained_text_input(vault_path, &text)?;
        let row = ExistingPlainReplayRow {
            cx_id: vault.cx_id_for_input(&input.bytes, state.panel.version),
            panel_version: state.panel.version,
            input_ref: InputRef {
                hash: input_hash(&input.bytes),
                pointer: input.pointer,
                redacted: false,
            },
            modality: input.modality,
            metadata,
            anchors,
        };
        if verify_existing_base_replay_row(vault, snapshot, &row, true)? {
            checked_existing += 1;
        } else {
            not_existing_or_incomplete += 1;
        }
    }
    ingest_runtime_log(format_args!(
        "phase=batch_existing_identity_preflight_ok rows={} existing_checked={} not_existing_or_incomplete={} elapsed_ms={}",
        validated_row_count,
        checked_existing,
        not_existing_or_incomplete,
        started.elapsed().as_millis()
    ));
    Ok(())
}

pub(crate) fn backfill_batch_existing_input_pointers(
    vault: &AsterVault,
    state: &VaultPanelState,
    vault_path: &std::path::Path,
    path: &std::path::Path,
) -> CliResult<()> {
    use std::io::BufRead;

    let file = std::fs::File::open(path)
        .map_err(|error| CliError::io(format!("open batch {}: {error}", path.display())))?;
    let reader = std::io::BufReader::new(file);
    let mut seen = BTreeSet::new();
    let mut existing = 0_usize;
    let mut changed = 0_usize;
    for (index, line) in reader.lines().enumerate() {
        let line =
            line.map_err(|error| CliError::io(format!("read batch line {}: {error}", index + 1)))?;
        let Some((text, _, _, _)) = parse_batch_line(index, &line)? else {
            continue;
        };
        let input = retention::retained_text_input(vault_path, &text)?;
        let cx_id = vault.cx_id_for_input(&input.bytes, state.panel.version);
        if seen.insert(cx_id) && base_exists(vault, cx_id)? {
            existing += 1;
            let expected = InputRef {
                hash: input_hash(&input.bytes),
                pointer: input.pointer,
                redacted: false,
            };
            if retention::apply_existing_input_pointer(vault, cx_id, &expected)? {
                changed += 1;
            }
        }
    }
    ingest_runtime_log(format_args!(
        "phase=batch_retained_input_pointer_readback distinct_existing={} changed={changed}",
        existing
    ));
    Ok(())
}

pub(crate) struct ExistingPlainReplayRow {
    cx_id: CxId,
    panel_version: u32,
    input_ref: InputRef,
    modality: Modality,
    metadata: BTreeMap<String, String>,
    anchors: Vec<Anchor>,
}

pub(crate) struct ExistingBatchReplayRow {
    pub(crate) cx_id: CxId,
    pub(crate) input_ref: InputRef,
    pub(crate) modality: Modality,
    pub(crate) metadata: BTreeMap<String, String>,
    pub(crate) anchors: Vec<Anchor>,
    pub(crate) oracle: Option<OracleEvent>,
}

pub(crate) fn existing_plain_batch_replay_rows(
    vault: &AsterVault,
    state: &VaultPanelState,
    vault_path: &std::path::Path,
    rows: &[BatchRow],
) -> CliResult<Option<Vec<ExistingPlainReplayRow>>> {
    let mut out = Vec::with_capacity(rows.len());
    let snapshot = vault.snapshot();
    let mut all_materialized = true;
    let mut checked_existing = 0_usize;
    for (text, metadata, anchors, oracle) in rows {
        let input = retention::retained_text_input(vault_path, text)?;
        let cx_id = vault.cx_id_for_input(&input.bytes, state.panel.version);
        let input_ref = InputRef {
            hash: input_hash(&input.bytes),
            pointer: input.pointer,
            redacted: false,
        };
        let mut metadata = metadata.clone();
        if let Some(event) = oracle {
            event.apply_metadata(&mut metadata)?;
        }
        let row = ExistingPlainReplayRow {
            cx_id,
            panel_version: state.panel.version,
            input_ref,
            modality: input.modality,
            metadata,
            anchors: anchors.clone(),
        };
        if !verify_existing_base_replay_row(vault, snapshot, &row, false)? {
            all_materialized = false;
            continue;
        }
        checked_existing += 1;
        out.push(row);
    }
    if all_materialized {
        Ok(Some(out))
    } else {
        ingest_runtime_log(format_args!(
            "phase=batch_existing_replay_base_only_preflight_mixed rows={} existing_materialized={} measurement_required=true slot_decode_skipped=true",
            rows.len(),
            checked_existing
        ));
        Ok(None)
    }
}

pub(crate) fn flush_plain_existing_batch_replay(
    vault: &AsterVault,
    vault_path: &std::path::Path,
    rows: Vec<ExistingPlainReplayRow>,
    summary: &mut BatchIngestSummary,
    output: IngestOutput,
) -> CliResult<()> {
    for sub in rows.chunks(EXISTING_REPLAY_CHUNK) {
        let ids = sub.iter().map(|row| row.cx_id).collect::<Vec<_>>();
        let ledger_seq = append_cli_batch_ledger(
            vault,
            EntryKind::Ingest,
            &ids,
            "cli-idempotent-ingest-batch",
        )?;
        vault.flush()?;
        calyx_aster::base_page_index::advance_base_page_index_head_if_base_unchanged(vault_path)?;
        let snapshot = vault.snapshot();
        for row in sub {
            if !verify_existing_base_replay_row(vault, snapshot, row, false)? {
                return Err(calyx_core::CalyxError::aster_corrupt_shard(format!(
                    "idempotent batch replay base readback missing for cx {} after ledger append",
                    row.cx_id
                ))
                .into());
            }
            let report = IngestReport {
                cx_id: row.cx_id.to_string(),
                new: false,
                ledger_seq,
            };
            summary.record(row.cx_id, &report);
            if output == IngestOutput::Rows {
                print_json(&report)?;
            }
        }
    }
    Ok(())
}

fn verify_existing_base_replay_row(
    vault: &AsterVault,
    snapshot: u64,
    row: &ExistingPlainReplayRow,
    allow_pointerless: bool,
) -> CliResult<bool> {
    let Some(bytes) = vault.read_cf_at(snapshot, ColumnFamily::Base, &base_key(row.cx_id))? else {
        return Ok(false);
    };
    let existing = decode_constellation_base(&bytes)?;
    let input_ref_matches = existing.input_ref == row.input_ref
        || (allow_pointerless
            && retention::input_ref_matches_or_backfillable(&existing.input_ref, &row.input_ref));
    if existing.panel_version != row.panel_version
        || !input_ref_matches
        || existing.modality != row.modality
        || existing.metadata != row.metadata
    {
        return Err(CliError::usage(format!(
            "idempotent batch replay for cx {} changed stored non-anchor identity: {}",
            row.cx_id,
            identity_mismatch_reason(
                IdentityFields {
                    panel_version: existing.panel_version,
                    input_ref: &existing.input_ref,
                    modality: existing.modality,
                    metadata: &existing.metadata,
                },
                IdentityFields {
                    panel_version: row.panel_version,
                    input_ref: &row.input_ref,
                    modality: row.modality,
                    metadata: &row.metadata,
                },
            )
        )));
    }
    if !incoming_anchors_already_materialized(vault, snapshot, row.cx_id, &row.anchors, &existing)?
    {
        return Ok(false);
    }
    Ok(true)
}

fn incoming_anchors_already_materialized(
    vault: &AsterVault,
    snapshot: u64,
    cx_id: CxId,
    incoming_anchors: &[Anchor],
    existing_base: &Constellation,
) -> CliResult<bool> {
    if incoming_anchors.is_empty() {
        return Ok(true);
    }
    let mut incoming = existing_base.clone();
    incoming.anchors = incoming_anchors.to_vec();
    if let AnchorConflictResult::Conflicting {
        anchor_type,
        reason,
    } = check_anchor_conflict(&incoming, existing_base)
    {
        return Err(calyx_core::CalyxError::aster_corrupt_shard(format!(
            "idempotent batch replay for cx {cx_id} has conflicting {anchor_type:?} anchor: {reason:?}"
        ))
        .into());
    }
    for anchor in incoming_anchors {
        if !existing_base
            .anchors
            .iter()
            .any(|existing| existing.kind == anchor.kind)
        {
            return Ok(false);
        }
        let Some(bytes) = vault.read_cf_at(
            snapshot,
            ColumnFamily::Anchors,
            &anchor_key(cx_id, &anchor.kind),
        )?
        else {
            return Err(calyx_core::CalyxError::aster_corrupt_shard(format!(
                "idempotent batch replay for cx {cx_id} found anchor {:?} in Base CF but missing from Anchors CF",
                anchor.kind
            ))
            .into());
        };
        let indexed = encode::decode_anchor(&bytes)?;
        if indexed.kind != anchor.kind || indexed.value != anchor.value {
            return Err(calyx_core::CalyxError::aster_corrupt_shard(format!(
                "idempotent batch replay for cx {cx_id} found conflicting Anchors CF value for {:?}",
                anchor.kind
            ))
            .into());
        }
    }
    Ok(true)
}

pub(crate) fn existing_batch_replay_rows(
    vault: &AsterVault,
    state: &VaultPanelState,
    vault_path: &std::path::Path,
    rows: &[BatchRow],
) -> CliResult<Option<Vec<ExistingBatchReplayRow>>> {
    let mut out = Vec::with_capacity(rows.len());
    let mut all_exist = true;
    let mut checked_existing = 0_usize;
    for (text, metadata, anchors, oracle) in rows {
        let input = retention::retained_text_input(vault_path, text)?;
        let cx_id = vault.cx_id_for_input(&input.bytes, state.panel.version);
        if !base_exists(vault, cx_id)? {
            all_exist = false;
            continue;
        }
        let input_ref = InputRef {
            hash: input_hash(&input.bytes),
            pointer: input.pointer,
            redacted: false,
        };
        let mut metadata = metadata.clone();
        if let Some(event) = oracle {
            event.apply_metadata(&mut metadata)?;
        }
        let row = ExistingBatchReplayRow {
            cx_id,
            input_ref,
            modality: input.modality,
            metadata,
            anchors: anchors.clone(),
            oracle: oracle.clone(),
        };
        verify_existing_batch_replay_identity(vault, state, &row)?;
        checked_existing += 1;
        out.push(row);
    }
    if all_exist {
        Ok(Some(out))
    } else {
        ingest_runtime_log(format_args!(
            "phase=batch_existing_replay_preflight_mixed rows={} existing_checked={} measurement_required=true",
            rows.len(),
            checked_existing
        ));
        Ok(None)
    }
}

pub(crate) fn flush_existing_batch_replay(
    vault: &AsterVault,
    state: &VaultPanelState,
    rows: Vec<ExistingBatchReplayRow>,
    summary: &mut BatchIngestSummary,
    output: IngestOutput,
) -> CliResult<()> {
    for sub in rows.chunks(EXISTING_REPLAY_CHUNK) {
        let mut order = Vec::with_capacity(sub.len());
        let mut known_anchor_kinds = BTreeMap::<CxId, BTreeSet<AnchorKind>>::new();
        for row in sub {
            let existing = verify_existing_batch_replay_identity(vault, state, row)?;
            let known = match known_anchor_kinds.entry(row.cx_id) {
                std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(current_anchor_kinds(vault, row.cx_id, true)?)
                }
            };
            let mut marker_kinds = Vec::new();
            for anchor in &row.anchors {
                if known.insert(anchor.kind.clone()) {
                    marker_kinds.push(anchor.kind.clone());
                }
            }
            let incoming = existing_replay_incoming(&existing, row);
            let expected_readback = if marker_kinds.is_empty() {
                existing
            } else {
                append_missing_batch_anchors(vault, &existing, &incoming, &marker_kinds)?
            };
            order.push(BatchOrderRow {
                cx_id: row.cx_id,
                expected_readback,
                new: false,
                marker_kinds,
                oracle: row.oracle.clone(),
            });
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
            let ledger_seq = idempotent_ledger_seq.ok_or_else(|| {
                CliError::usage("missing idempotent batch ledger seq for replay row")
            })?;
            for kind in row.marker_kinds {
                append_anchor_marker_ledger(vault, cx_id, &kind)?;
            }
            let report = IngestReport {
                cx_id: cx_id.to_string(),
                new: false,
                ledger_seq,
            };
            summary.record(cx_id, &report);
            if output == IngestOutput::Rows {
                print_json(&report)?;
            }
        }
        vault.flush()?;
    }
    Ok(())
}
