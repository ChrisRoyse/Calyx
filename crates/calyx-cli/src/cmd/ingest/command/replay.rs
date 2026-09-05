use super::batch_physical::{BatchPhysicalBaseState, visit_indexed_batch_base_rows};
use super::batch_support::{
    BatchOrderRow, IdentityFields, append_idempotent_batch_ledger, append_missing_batch_anchors,
    append_oracle_events, current_anchor_kinds, existing_replay_incoming, identity_mismatch_reason,
    verify_existing_batch_replay_identity,
};
use super::replay_identity::prepare_batch_identity;
use super::*;

pub(crate) struct BatchExistingPreflight {
    batch_ids: BTreeSet<CxId>,
    expected: BTreeMap<CxId, Vec<ExistingPlainReplayRow>>,
    materialized: BTreeSet<CxId>,
    distinct_existing: usize,
    exact_pointer_skipped: usize,
    pointer_backfills: BTreeMap<CxId, InputRef>,
    retained_inputs: BTreeMap<CxId, (InputRef, Vec<u8>)>,
    physical_base: BatchPhysicalBaseState,
}

impl BatchExistingPreflight {
    pub(crate) fn batch_ids(&self) -> &BTreeSet<CxId> {
        &self.batch_ids
    }

    pub(super) fn physical_base(&self) -> &BatchPhysicalBaseState {
        &self.physical_base
    }

    fn is_materialized(&self, cx_id: CxId) -> bool {
        self.materialized.contains(&cx_id)
    }
}

pub(crate) fn preflight_batch_existing_identity(
    vault: &AsterVault,
    state: &VaultPanelState,
    vault_path: &std::path::Path,
    path: &std::path::Path,
    identity: &batch_identity::BatchSnapshotIdentity,
) -> CliResult<BatchExistingPreflight> {
    use std::io::BufRead;

    let validated_row_count = identity.nonblank_row_count;
    let started = std::time::Instant::now();
    ingest_runtime_log(format_args!(
        "phase=batch_existing_identity_preflight_start rows={validated_row_count}"
    ));
    let file = std::fs::File::open(path)
        .map_err(|err| CliError::io(format!("open batch {}: {err}", path.display())))?;
    let mut reader = std::io::BufReader::new(batch_identity::HashingReader::new(file));
    let snapshot = vault.snapshot();
    let mut expected = BTreeMap::<CxId, Vec<ExistingPlainReplayRow>>::new();
    let mut retained_inputs = BTreeMap::<CxId, (InputRef, Vec<u8>)>::new();
    let mut observed_rows = 0usize;
    for (index, line) in (&mut reader).lines().enumerate() {
        let line =
            line.map_err(|err| CliError::io(format!("read batch line {}: {err}", index + 1)))?;
        let Some(row) = parse_batch_line(index, &line)? else {
            continue;
        };
        observed_rows = observed_rows.saturating_add(1);
        let identity = prepare_batch_identity(vault, state, vault_path, &row)?;
        match retained_inputs.entry(identity.cx_id) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert((identity.input_ref.clone(), identity.retained_bytes.clone()));
            }
            std::collections::btree_map::Entry::Occupied(entry)
                if entry.get().0 != identity.input_ref
                    || entry.get().1 != identity.retained_bytes =>
            {
                return Err(CliError::from(CalyxError {
                    code: "CALYX_INGEST_BATCH_CHANGED",
                    message: format!(
                        "batch repeats cx {} with different retained source identity",
                        identity.cx_id
                    ),
                    remediation: "restore one exact source byte sequence and input reference per content-addressed constellation",
                }));
            }
            std::collections::btree_map::Entry::Occupied(_) => {}
        }
        let row = ExistingPlainReplayRow {
            cx_id: identity.cx_id,
            panel_version: state.panel.version,
            input_ref: identity.input_ref,
            modality: identity.modality,
            scalars: identity.scalars,
            metadata: identity.metadata,
            anchors: row.anchors,
        };
        expected.entry(row.cx_id).or_default().push(row);
    }
    // Cheap fast-fail, then the authoritative digest of the exact bytes read. This
    // runs before the input-pointer backfill mutation in the caller, so a mutation
    // between structured preflight and this replay preflight aborts with zero writes
    // (issue #1923).
    identity.verify_row_count(path, observed_rows, "replay preflight")?;
    let observed = reader.into_inner().finish();
    identity.verify_observed(path, &observed, "replay preflight")?;
    let batch_ids = expected.keys().copied().collect::<BTreeSet<_>>();
    let keys = batch_ids
        .iter()
        .map(|cx_id| base_key(*cx_id))
        .collect::<Vec<_>>();
    let mut materialized = BTreeSet::new();
    let mut distinct_existing = 0_usize;
    let mut checked_existing = 0_usize;
    let mut not_existing_or_incomplete = 0_usize;
    let mut exact_pointer_skipped = 0_usize;
    let mut pointer_backfills = BTreeMap::new();
    let mut visible = BTreeSet::new();
    let mut tombstoned = BTreeSet::new();
    let mut required_anchor_rows = BTreeMap::<Vec<u8>, Anchor>::new();
    let base_stats = visit_indexed_batch_base_rows(vault_path, &keys, |key, value| {
        let key_bytes: [u8; 16] = key.try_into().map_err(|_| {
            CliError::runtime(format!(
                "batch identity preflight received a {}-byte Base key; expected 16",
                key.len()
            ))
        })?;
        let cx_id = CxId::from_bytes(key_bytes);
        let rows = expected.get(&cx_id).ok_or_else(|| {
            CliError::runtime(format!(
                "batch identity preflight received unrequested Base cx_id {cx_id}"
            ))
        })?;
        let Some(value) = value else {
            not_existing_or_incomplete += rows.len();
            return Ok(());
        };
        if calyx_aster::mvcc::is_tombstone_value(&value) {
            tombstoned.insert(cx_id);
            not_existing_or_incomplete += rows.len();
            return Ok(());
        }
        let existing = decode_constellation_base(&value)?;
        if existing.cx_id != cx_id {
            return Err(calyx_core::CalyxError::aster_corrupt_shard(format!(
                "batch identity preflight Base key {cx_id} decoded as {}",
                existing.cx_id
            ))
            .into());
        }
        visible.insert(cx_id);
        distinct_existing += 1;
        let expected_input_ref = &rows[0].input_ref;
        if existing.input_ref == *expected_input_ref {
            exact_pointer_skipped += 1;
        } else if retention::input_ref_matches_or_backfillable(
            &existing.input_ref,
            expected_input_ref,
        ) {
            pointer_backfills.insert(cx_id, expected_input_ref.clone());
        }
        let mut complete = true;
        for row in rows {
            if verify_existing_base_replay_value(&existing, row, true)? {
                checked_existing += 1;
            } else {
                not_existing_or_incomplete += 1;
                complete = false;
            }
        }
        if complete {
            for row in rows {
                for anchor in &row.anchors {
                    let anchor_key = anchor_key(cx_id, &anchor.kind);
                    match required_anchor_rows.entry(anchor_key) {
                        std::collections::btree_map::Entry::Vacant(entry) => {
                            entry.insert(anchor.clone());
                        }
                        std::collections::btree_map::Entry::Occupied(entry)
                            if entry.get().kind != anchor.kind
                                || entry.get().value != anchor.value =>
                        {
                            return Err(calyx_core::CalyxError::aster_corrupt_shard(format!(
                                "batch repeats cx {cx_id} with conflicting {:?} anchor value",
                                anchor.kind
                            ))
                            .into());
                        }
                        std::collections::btree_map::Entry::Occupied(_) => {}
                    }
                }
            }
            materialized.insert(cx_id);
        }
        Ok(())
    })?;
    let required_anchor_count = required_anchor_rows.len();
    if required_anchor_count > 0 {
        // Point-read exactly the required Anchors CF keys instead of scanning
        // the entire CF page-wise (#1986). `read_cf_at` resolves one key at a
        // pinned snapshot lease; we assert the vault's latest sequence is
        // unchanged around the phase so the consistency guarantee is at least
        // as strong as the renewing-latest scan this replaces — both fail
        // closed if a writer advances the vault, and the whole preflight runs
        // under the caller's file lock. Iterating the BTreeMap keeps the
        // "first missing key" deterministic, matching the prior scan.
        let anchor_pointread_started = std::time::Instant::now();
        let cx_from_key = |key: &[u8]| {
            key.get(..16)
                .and_then(|bytes| <[u8; 16]>::try_from(bytes).ok())
                .map(CxId::from_bytes)
                .map(|id| id.to_string())
                .unwrap_or_else(|| "<malformed-key>".to_string())
        };
        let latest_before = vault.latest_seq();
        if latest_before != snapshot {
            return Err(calyx_core::CalyxError::stale_derived(format!(
                "batch identity preflight anchors point-read requires unchanged latest sequence {snapshot}, observed {latest_before}"
            ))
            .into());
        }
        for (key, expected_anchor) in &required_anchor_rows {
            let Some(value) = vault.read_cf_at(snapshot, ColumnFamily::Anchors, key)? else {
                let cx_id = cx_from_key(key);
                return Err(calyx_core::CalyxError::aster_corrupt_shard(format!(
                    "batch identity preflight Base CF anchor for cx {cx_id} is missing from Anchors CF"
                ))
                .into());
            };
            let indexed = encode::decode_anchor(&value)?;
            if indexed.kind != expected_anchor.kind || indexed.value != expected_anchor.value {
                let cx_id = cx_from_key(key);
                return Err(calyx_core::CalyxError::aster_corrupt_shard(format!(
                    "batch identity preflight Anchors CF value mismatch for cx {cx_id} key_len={}",
                    key.len()
                ))
                .into());
            }
        }
        let latest_after = vault.latest_seq();
        if latest_after != snapshot {
            return Err(calyx_core::CalyxError::stale_derived(format!(
                "batch identity preflight anchors point-read observed latest sequence {latest_after} after {snapshot} during verification"
            ))
            .into());
        }
        ingest_runtime_log(format_args!(
            "phase=preflight_anchor_pointread anchors_required={required_anchor_count} elapsed_ms={}",
            anchor_pointread_started.elapsed().as_millis()
        ));
    }
    retained_inputs.retain(|cx_id, _| visible.contains(cx_id));
    ingest_runtime_log(format_args!(
        "phase=batch_existing_identity_preflight_ok rows={} distinct_ids={} existing_checked={} not_existing_or_incomplete={} base_pages={} base_source_files={} anchors_required={} exact_pointer_skipped={} pointer_backfills={} elapsed_ms={}",
        validated_row_count,
        batch_ids.len(),
        checked_existing,
        not_existing_or_incomplete,
        base_stats.touched_pages,
        base_stats.source_files,
        required_anchor_count,
        exact_pointer_skipped,
        pointer_backfills.len(),
        started.elapsed().as_millis()
    ));
    Ok(BatchExistingPreflight {
        batch_ids,
        expected,
        materialized,
        distinct_existing,
        exact_pointer_skipped,
        pointer_backfills,
        retained_inputs,
        physical_base: BatchPhysicalBaseState {
            visible,
            tombstoned,
        },
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct BatchInputPointerBackfillStats {
    pub(crate) distinct_existing: usize,
    pub(crate) exact_pointer_skipped: usize,
    pub(crate) backfill_attempted: usize,
    pub(crate) changed: usize,
}

pub(crate) fn backfill_batch_existing_input_pointers_with_commit_intent(
    vault: &AsterVault,
    preflight: &BatchExistingPreflight,
    intent: &mut dyn calyx_aster::vault::DurableCommitIntent,
) -> CliResult<BatchInputPointerBackfillStats> {
    let mut stats = BatchInputPointerBackfillStats {
        distinct_existing: preflight.distinct_existing,
        exact_pointer_skipped: preflight.exact_pointer_skipped,
        backfill_attempted: preflight.pointer_backfills.len(),
        changed: 0,
    };
    for (cx_id, expected) in &preflight.pointer_backfills {
        let retained_bytes = preflight
            .retained_inputs
            .get(cx_id)
            .map(|(_, bytes)| bytes.as_slice())
            .ok_or_else(|| {
                CliError::runtime(format!(
                    "batch pointer backfill for {cx_id} has no exact retained source bytes"
                ))
            })?;
        if retention::apply_existing_input_pointer(vault, *cx_id, expected, retained_bytes, intent)?
        {
            stats.changed += 1;
        }
    }
    for (cx_id, (expected, retained_bytes)) in &preflight.retained_inputs {
        if preflight.pointer_backfills.contains_key(cx_id) {
            continue;
        }
        vault.ensure_retained_input(*cx_id, expected, retained_bytes)?;
    }
    ingest_runtime_log(format_args!(
        "phase=batch_retained_input_pointer_outcomes distinct_existing={} exact_pointer_skipped={} backfill_attempted={} changed={}",
        stats.distinct_existing,
        stats.exact_pointer_skipped,
        stats.backfill_attempted,
        stats.changed,
    ));
    Ok(stats)
}

#[cfg(test)]
pub(crate) fn backfill_batch_existing_input_pointers(
    vault: &AsterVault,
    preflight: &BatchExistingPreflight,
) -> CliResult<BatchInputPointerBackfillStats> {
    let mut intent = calyx_aster::vault::NoopDurableCommitIntent;
    backfill_batch_existing_input_pointers_with_commit_intent(vault, preflight, &mut intent)
}

#[derive(Clone)]
pub(crate) struct ExistingPlainReplayRow {
    cx_id: CxId,
    panel_version: u32,
    input_ref: InputRef,
    modality: Modality,
    scalars: BTreeMap<String, f64>,
    metadata: BTreeMap<String, String>,
    anchors: Vec<Anchor>,
}

pub(crate) struct ExistingBatchReplayRow {
    pub(crate) cx_id: CxId,
    pub(crate) input_ref: InputRef,
    pub(crate) modality: Modality,
    pub(crate) scalars: BTreeMap<String, f64>,
    pub(crate) metadata: BTreeMap<String, String>,
    pub(crate) anchors: Vec<Anchor>,
    pub(crate) oracle: Option<OracleEvent>,
}

pub(crate) fn existing_plain_batch_replay_rows(
    vault: &AsterVault,
    state: &VaultPanelState,
    vault_path: &std::path::Path,
    rows: &[BatchRow],
    preflight: &BatchExistingPreflight,
) -> CliResult<Option<Vec<ExistingPlainReplayRow>>> {
    let mut out = Vec::with_capacity(rows.len());
    for source_row in rows {
        let identity = prepare_batch_identity(vault, state, vault_path, source_row)?;
        let cx_id = identity.cx_id;
        if !preflight.is_materialized(cx_id) {
            ingest_runtime_log(format_args!(
                "phase=batch_existing_replay_base_only_preflight_mixed rows={} missing_or_incomplete_cx={} measurement_required=true slot_decode_skipped=true",
                rows.len(),
                cx_id
            ));
            return Ok(None);
        }
        let row = preflight
            .expected
            .get(&cx_id)
            .and_then(|candidates| {
                candidates.iter().find(|candidate| {
                    candidate.scalars == identity.scalars
                        && candidate.metadata == identity.metadata
                        && candidate.anchors.len() == source_row.anchors.len()
                        && candidate.anchors.iter().all(|expected| {
                            source_row.anchors.iter().any(|actual| {
                                actual.kind == expected.kind && actual.value == expected.value
                            })
                        })
                })
            })
            .ok_or_else(|| {
                CliError::runtime(format!(
                    "batch replay row for preflighted cx {cx_id} no longer matches its preflight metadata/anchors"
                ))
            })?;
        out.push(row.clone());
    }
    Ok(Some(out))
}

pub(crate) fn flush_plain_existing_batch_replay(
    vault: &AsterVault,
    // Retained for call-site symmetry; the content-derived Base page index
    // freshness digest (issue #1861) no longer needs a per-flush manifest
    // re-stamp for idempotent re-ingest.
    _vault_path: &std::path::Path,
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
        // Idempotent re-ingest only advances the ledger (EntryKind::Ingest)
        // without changing Base constellation bytes, so the Base page index
        // stays fresh under the content-derived digest freshness contract
        // (issue #1861). No manifest re-stamp is needed.
        for row in sub {
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

fn verify_existing_base_replay_value(
    existing: &Constellation,
    row: &ExistingPlainReplayRow,
    allow_pointerless: bool,
) -> CliResult<bool> {
    let input_ref_matches = existing.input_ref == row.input_ref
        || (allow_pointerless
            && retention::input_ref_matches_or_backfillable(&existing.input_ref, &row.input_ref));
    if existing.panel_version != row.panel_version
        || !input_ref_matches
        || existing.modality != row.modality
        || existing.scalars != row.scalars
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
                    scalars: &existing.scalars,
                    metadata: &existing.metadata,
                },
                IdentityFields {
                    panel_version: row.panel_version,
                    input_ref: &row.input_ref,
                    modality: row.modality,
                    scalars: &row.scalars,
                    metadata: &row.metadata,
                },
            )
        )));
    }
    if !incoming_anchors_already_materialized(row.cx_id, &row.anchors, existing)? {
        return Ok(false);
    }
    Ok(true)
}

fn incoming_anchors_already_materialized(
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
    for source_row in rows {
        let identity = prepare_batch_identity(vault, state, vault_path, source_row)?;
        let cx_id = identity.cx_id;
        if !base_exists(vault, cx_id)? {
            all_exist = false;
            continue;
        }
        let row = ExistingBatchReplayRow {
            cx_id,
            input_ref: identity.input_ref,
            modality: identity.modality,
            scalars: identity.scalars,
            metadata: identity.metadata,
            anchors: source_row.anchors.clone(),
            oracle: source_row.oracle.clone(),
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
    intent: &mut dyn calyx_aster::vault::DurableCommitIntent,
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
                append_missing_batch_anchors(vault, &existing, &incoming, &marker_kinds, intent)?
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
