use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use calyx_core::{Constellation, CxId, SlotId};

use calyx_aster::mvcc::{Freshness, Snapshot};
use calyx_aster::vault::AsterVault;
use rayon::prelude::*;

#[path = "rebuild_scan.rs"]
mod rebuild_scan;
use rebuild_scan::{SlotRows, collect_slot_rows_from_cf, load_base_docs_at};

use super::rebuild::{RebuildProgress, previous_manifest, prune_stale_index_artifacts};
use super::rebuild_plan::{
    SlotBuildPlan, bounded_parallel_slot_count, configured_rebuild_reader_lease_ms,
    configured_rebuild_scan_page_rows, slot_build_plans, validate_parallel_rebuild_config,
};
use super::*;

pub(super) type SharedRebuildProgress<'a, F> = Arc<Mutex<&'a mut F>>;

pub(super) fn emit_shared_progress<F>(
    progress: &SharedRebuildProgress<'_, F>,
    event: RebuildProgress<'_>,
) -> CliResult
where
    F: FnMut(RebuildProgress<'_>) -> CliResult + Send,
{
    let mut progress = progress
        .lock()
        .map_err(|_| stale("search rebuild progress sink lock poisoned"))?;
    (**progress)(event)
}

pub(super) fn rebuild_for_vault_with_progress<F>(
    vault_dir: &Path,
    vault: &AsterVault,
    mut progress: F,
) -> CliResult
where
    F: FnMut(RebuildProgress<'_>) -> CliResult + Send,
{
    validate_parallel_rebuild_config()?;
    let snapshot = vault.pin_reader(
        Freshness::FreshDerived,
        configured_rebuild_reader_lease_ms()?,
    );
    let guard = PinnedReadGuard::new(vault, snapshot);
    let base_seq = guard.snapshot().seq();
    // Stake the write-ahead rebuild intent before any index work so a kill at
    // any later point leaves a durable, structured record. A pre-existing
    // marker (from the mutation that made this rebuild necessary) is kept —
    // its commit context is richer than a generic rebuild record.
    match marker::read_rebuild_required_marker(vault_dir)? {
        Some(_) => progress(RebuildProgress::phase("rebuild_marker_preserved"))?,
        None => {
            let mut intent = marker::RebuildRequiredMarker::new(
                "search_index_rebuild",
                "full search-index rebuild in progress; derived indexes are unproven until the manifest is republished",
            )?;
            intent.required_base_seq = Some(base_seq);
            marker::write_rebuild_required_marker(vault_dir, &intent)?;
            progress(RebuildProgress::phase("rebuild_marker_written"))?;
        }
    }
    progress(RebuildProgress::phase("load_docs_start"))?;
    let page_rows = configured_rebuild_scan_page_rows()?;
    let base_docs = load_base_docs_at(vault, guard.snapshot(), page_rows, &mut progress)?;
    progress(RebuildProgress {
        rows: Some(base_docs.len()),
        base_seq: Some(base_seq),
        ..RebuildProgress::phase("load_docs_ok")
    })?;
    let summary = rebuild_from_base_with_progress(
        vault_dir,
        vault,
        guard.snapshot(),
        &base_docs,
        page_rows,
        &mut progress,
    )?;
    progress(RebuildProgress {
        rows: Some(summary.total_rows),
        base_seq: Some(base_seq),
        manifest_path: Some(&summary.manifest_path),
        ..RebuildProgress::phase("done")
    })?;
    let _ = (summary.slots, summary.total_rows, &summary.manifest_path);
    Ok(())
}

fn rebuild_from_base_with_progress<F>(
    vault_dir: &Path,
    vault: &AsterVault,
    snapshot: Snapshot,
    base_docs: &BTreeMap<CxId, Constellation>,
    page_rows: usize,
    progress: &mut F,
) -> CliResult<RebuildSummary>
where
    F: FnMut(RebuildProgress<'_>) -> CliResult + Send,
{
    let root = vault_dir.join(INDEX_ROOT);
    fs::create_dir_all(&root)?;
    let base_seq = snapshot.seq();
    progress(RebuildProgress::phase("previous_manifest_start"))?;
    let previous_manifest = previous_manifest(vault_dir)?;
    progress(RebuildProgress::phase("previous_manifest_ok"))?;

    let plans = slot_build_plans(base_docs, previous_manifest.as_ref());
    if plans.is_empty()
        && previous_manifest
            .as_ref()
            .is_some_and(|manifest| !manifest.slots.is_empty())
    {
        return Err(stale(
            "base CF scan produced no searchable slots but the previous search manifest was non-empty; refusing to replace it with an empty manifest",
        ));
    }
    let parallelism = bounded_parallel_slot_count(&plans)?;
    progress(RebuildProgress {
        rows: Some(plans.len()),
        base_seq: Some(base_seq),
        ..RebuildProgress::phase("slot_plan_ok")
    })?;

    let mut entries = Vec::new();
    let mut total_rows = 0usize;
    for chunk in plans.chunks(parallelism) {
        for plan in chunk {
            progress(RebuildProgress::slot(
                "slot_build_start",
                plan.slot,
                Some(plan.expected_ids.len()),
                Some(base_seq),
            ))?;
        }
        let progress_lock = Arc::new(Mutex::new(&mut *progress));
        let mut built = chunk
            .par_iter()
            .map(|plan| {
                build_slot_entry(
                    vault_dir,
                    &root,
                    vault,
                    snapshot,
                    plan,
                    previous_manifest.as_ref(),
                    page_rows,
                    Some(&progress_lock),
                )
            })
            .collect::<CliResult<Vec<_>>>()?;
        drop(progress_lock);
        built.sort_by_key(|built| built.entry.slot());
        for built in built {
            total_rows += built.row_count;
            progress(RebuildProgress::slot(
                built.ok_phase(),
                SlotId::new(built.entry.slot()),
                Some(built.row_count),
                Some(base_seq),
            ))?;
            if let Some(entry) = built.entry.into_entry() {
                entries.push(entry);
            }
        }
    }
    entries.sort_by_key(|entry| entry.slot);

    let filter = match reuse_staged_filter_entry(vault_dir, &root, base_seq)? {
        Some(entry) => {
            progress(RebuildProgress {
                rows: Some(entry.len),
                base_seq: Some(base_seq),
                ..RebuildProgress::phase("filter_reuse_ok")
            })?;
            entry
        }
        None => {
            progress(RebuildProgress {
                rows: Some(base_docs.len()),
                base_seq: Some(base_seq),
                ..RebuildProgress::phase("filter_start")
            })?;
            let entry = filter::write(vault_dir, &root, base_docs, base_seq)?;
            write_staged_filter_artifact(&root, base_seq, &entry)?;
            progress(RebuildProgress {
                rows: Some(base_docs.len()),
                base_seq: Some(base_seq),
                ..RebuildProgress::phase("filter_ok")
            })?;
            entry
        }
    };

    let manifest = SearchIndexManifest {
        format: MANIFEST_FORMAT.to_string(),
        base_seq,
        filter: Some(filter),
        slots: entries,
    };
    validate_staged_manifest_artifacts(vault_dir, &manifest)?;
    let manifest_path = manifest_path(vault_dir);
    progress(RebuildProgress::manifest(
        "manifest_write_start",
        &manifest_path,
        base_seq,
    ))?;
    // Durable (dir-fsynced) so the manifest publish is persisted strictly
    // before the marker removal below can be — otherwise a power loss could
    // surface "marker gone, manifest still old", the exact silent-stale state
    // this machinery exists to prevent.
    write_json_atomic_durable(&manifest_path, &manifest)?;
    progress(RebuildProgress::manifest(
        "manifest_write_ok",
        &manifest_path,
        base_seq,
    ))?;
    progress(RebuildProgress::phase("prune_start"))?;
    prune_stale_index_artifacts(vault_dir, &root, &manifest)?;
    progress(RebuildProgress::phase("prune_ok"))?;
    let cleared = marker::clear_rebuild_required_marker(vault_dir, base_seq)?;
    progress(RebuildProgress::phase(match cleared {
        marker::MarkerClearOutcome::Cleared => "rebuild_marker_cleared",
        marker::MarkerClearOutcome::Absent => "rebuild_marker_absent",
    }))?;
    Ok(RebuildSummary {
        slots: manifest.slots.len(),
        total_rows,
        manifest_path,
    })
}

pub(super) fn validate_staged_manifest_artifacts(
    vault_dir: &Path,
    manifest: &SearchIndexManifest,
) -> CliResult {
    if let Some(filter) = &manifest.filter {
        filter::validate_entry(vault_dir, filter, manifest.base_seq)?;
    }
    for entry in &manifest.slots {
        let slot = SlotId::new(entry.slot);
        match entry.kind.as_str() {
            "diskann" | "flat_dense" => dense::validate_entry(vault_dir, entry, slot)?,
            "sparse_inverted" => sparse::validate_entry(vault_dir, entry, manifest.base_seq, slot)?,
            "multi_maxsim" | "multi_maxsim_segments" => {
                multi::validate_entry(vault_dir, entry, manifest.base_seq, slot)?
            }
            other => {
                return Err(stale(format!(
                    "persistent slot {slot} staged index kind {other} is unsupported; rebuild the vault search indexes"
                )));
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn build_slot_entry<F>(
    vault_dir: &Path,
    root: &Path,
    vault: &AsterVault,
    snapshot: Snapshot,
    plan: &SlotBuildPlan,
    previous_manifest: Option<&SearchIndexManifest>,
    page_rows: usize,
    progress: Option<&SharedRebuildProgress<'_, F>>,
) -> CliResult<BuiltSlot>
where
    F: FnMut(RebuildProgress<'_>) -> CliResult + Send,
{
    let base_seq = snapshot.seq();
    if let Some(built) = reuse_staged_slot_entry(vault_dir, root, plan, base_seq)? {
        if let Some(progress) = progress {
            emit_shared_progress(
                progress,
                RebuildProgress::slot(
                    "slot_reuse_ok",
                    plan.slot,
                    Some(built.row_count),
                    Some(base_seq),
                ),
            )?;
        }
        return Ok(built);
    }
    let rows = collect_slot_rows_from_cf(vault, snapshot, plan, page_rows, progress)?;
    let row_count = rows.len();
    if let Some(progress) = progress {
        emit_shared_progress(
            progress,
            RebuildProgress::slot(
                "slot_rows_loaded",
                plan.slot,
                Some(row_count),
                Some(base_seq),
            ),
        )?;
        emit_shared_progress(
            progress,
            RebuildProgress::slot(
                "slot_index_write_start",
                plan.slot,
                Some(row_count),
                Some(base_seq),
            ),
        )?;
    }
    let entry = match rows {
        SlotRows::Dense(rows) => OptionalSearchIndexEntry::Some(dense::write_with_progress(
            vault_dir,
            root,
            plan.slot,
            rows,
            base_seq,
            |event| match progress {
                Some(progress) => emit_shared_progress(progress, event),
                None => Ok(()),
            },
        )?),
        SlotRows::Sparse(rows) => OptionalSearchIndexEntry::Some(sparse::write(
            vault_dir, root, plan.slot, rows, base_seq,
        )?),
        SlotRows::Multi(rows) => {
            let previous = previous_manifest.and_then(|manifest| {
                manifest
                    .slots
                    .iter()
                    .find(|entry| entry.slot == plan.slot.get())
            });
            OptionalSearchIndexEntry::Some(multi::write(
                vault_dir,
                root,
                plan.slot,
                rows,
                base_seq,
                previous,
                &mut |event| match progress {
                    Some(progress) => emit_shared_progress(progress, event),
                    None => Ok(()),
                },
            )?)
        }
        SlotRows::AbsentOnly => OptionalSearchIndexEntry::None {
            slot: plan.slot.get(),
        },
    };
    write_staged_slot_artifact(vault_dir, root, plan.slot, base_seq, &entry)?;
    if let Some(progress) = progress {
        emit_shared_progress(
            progress,
            RebuildProgress::slot(
                "slot_index_write_ok",
                plan.slot,
                Some(row_count),
                Some(base_seq),
            ),
        )?;
    }
    Ok(BuiltSlot { entry, row_count })
}

#[path = "rebuild_staged.rs"]
mod rebuild_staged;
use rebuild_staged::{
    BuiltSlot, OptionalSearchIndexEntry, reuse_staged_filter_entry, reuse_staged_slot_entry,
    write_staged_filter_artifact, write_staged_slot_artifact,
};

struct PinnedReadGuard<'a> {
    vault: &'a AsterVault,
    snapshot: Snapshot,
}

impl<'a> PinnedReadGuard<'a> {
    fn new(vault: &'a AsterVault, snapshot: Snapshot) -> Self {
        Self { vault, snapshot }
    }

    fn snapshot(&self) -> Snapshot {
        self.snapshot
    }
}

impl Drop for PinnedReadGuard<'_> {
    fn drop(&mut self) {
        let _ = self.vault.release_reader(self.snapshot.lease().id());
    }
}
