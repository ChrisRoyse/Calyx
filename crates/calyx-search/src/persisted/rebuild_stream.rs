use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::mvcc::{Freshness, Snapshot};
use calyx_aster::vault::AsterVault;
use calyx_aster::vault::encode::{decode_constellation_base, decode_slot_vector};
use calyx_core::{CalyxError, Constellation, CxId, SlotId, SlotVector};
use rayon::prelude::*;

use super::rebuild::{RebuildProgress, previous_manifest, prune_stale_index_artifacts};
use super::rebuild_plan::{
    SlotBuildPlan, bounded_parallel_slot_count, configured_rebuild_reader_lease_ms,
    slot_build_plans, validate_parallel_rebuild_config,
};
use super::*;

pub(super) fn rebuild_for_vault_with_progress<F>(
    vault_dir: &Path,
    vault: &AsterVault,
    mut progress: F,
) -> CliResult
where
    F: FnMut(RebuildProgress<'_>),
{
    validate_parallel_rebuild_config()?;
    progress(RebuildProgress::phase("load_docs_start"));
    let snapshot = vault.pin_reader(
        Freshness::FreshDerived,
        configured_rebuild_reader_lease_ms()?,
    );
    let guard = PinnedReadGuard::new(vault, snapshot);
    let base_docs = load_base_docs_at(vault, guard.snapshot())?;
    let base_seq = guard.snapshot().seq();
    progress(RebuildProgress {
        rows: Some(base_docs.len()),
        base_seq: Some(base_seq),
        ..RebuildProgress::phase("load_docs_ok")
    });
    let summary = rebuild_from_base_with_progress(
        vault_dir,
        vault,
        guard.snapshot(),
        &base_docs,
        &mut progress,
    )?;
    progress(RebuildProgress {
        rows: Some(summary.total_rows),
        base_seq: Some(base_seq),
        manifest_path: Some(&summary.manifest_path),
        ..RebuildProgress::phase("done")
    });
    let _ = (summary.slots, summary.total_rows, &summary.manifest_path);
    Ok(())
}

fn rebuild_from_base_with_progress<F>(
    vault_dir: &Path,
    vault: &AsterVault,
    snapshot: Snapshot,
    base_docs: &BTreeMap<CxId, Constellation>,
    progress: &mut F,
) -> CliResult<RebuildSummary>
where
    F: FnMut(RebuildProgress<'_>),
{
    let root = vault_dir.join(INDEX_ROOT);
    fs::create_dir_all(&root)?;
    let base_seq = snapshot.seq();
    progress(RebuildProgress::phase("previous_manifest_start"));
    let previous_manifest = previous_manifest(vault_dir)?;
    progress(RebuildProgress::phase("previous_manifest_ok"));

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
    });

    let mut entries = Vec::new();
    let mut total_rows = 0usize;
    for chunk in plans.chunks(parallelism) {
        for plan in chunk {
            progress(RebuildProgress::slot(
                "slot_build_start",
                plan.slot,
                Some(plan.expected_ids.len()),
                Some(base_seq),
            ));
        }
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
                )
            })
            .collect::<CliResult<Vec<_>>>()?;
        built.sort_by_key(|built| built.entry.slot());
        for built in built {
            total_rows += built.row_count;
            progress(RebuildProgress::slot(
                built.ok_phase(),
                SlotId::new(built.entry.slot()),
                Some(built.row_count),
                Some(base_seq),
            ));
            if let Some(entry) = built.entry.into_entry() {
                entries.push(entry);
            }
        }
    }
    entries.sort_by_key(|entry| entry.slot);

    progress(RebuildProgress {
        rows: Some(base_docs.len()),
        base_seq: Some(base_seq),
        ..RebuildProgress::phase("filter_start")
    });
    let filter = filter::write(vault_dir, &root, base_docs, base_seq)?;
    progress(RebuildProgress {
        rows: Some(base_docs.len()),
        base_seq: Some(base_seq),
        ..RebuildProgress::phase("filter_ok")
    });

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
    ));
    write_json_atomic(&manifest_path, &manifest)?;
    progress(RebuildProgress::manifest(
        "manifest_write_ok",
        &manifest_path,
        base_seq,
    ));
    progress(RebuildProgress::phase("prune_start"));
    prune_stale_index_artifacts(vault_dir, &root, &manifest)?;
    progress(RebuildProgress::phase("prune_ok"));
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

fn load_base_docs_at(
    vault: &AsterVault,
    snapshot: Snapshot,
) -> CliResult<BTreeMap<CxId, Constellation>> {
    let base_rows = vault.scan_cf_snapshot(snapshot, ColumnFamily::Base)?;
    let decoded_base = base_rows
        .into_par_iter()
        .map(|(key, bytes)| {
            let cx_id = cx_id_from_cf_key(&key, "base CF")?;
            let cx = decode_constellation_base(&bytes)?;
            if cx.cx_id != cx_id {
                return Err(CalyxError::aster_corrupt_shard(format!(
                    "base CF key {cx_id} contains constellation {}",
                    cx.cx_id
                )));
            }
            Ok((cx_id, cx))
        })
        .collect::<calyx_core::Result<Vec<_>>>()?;
    Ok(decoded_base.into_iter().collect())
}

struct BuiltSlot {
    entry: OptionalSearchIndexEntry,
    row_count: usize,
}

impl BuiltSlot {
    fn ok_phase(&self) -> &'static str {
        match self.entry.kind() {
            Some("diskann" | "flat_dense") => "dense_slot_ok",
            Some("sparse_inverted") => "sparse_slot_ok",
            Some("multi_maxsim" | "multi_maxsim_segments") => "multi_slot_ok",
            _ => "slot_build_ok",
        }
    }
}

enum OptionalSearchIndexEntry {
    Some(SearchIndexEntry),
    None { slot: u16 },
}

impl OptionalSearchIndexEntry {
    fn slot(&self) -> u16 {
        match self {
            Self::Some(entry) => entry.slot,
            Self::None { slot } => *slot,
        }
    }

    fn kind(&self) -> Option<&str> {
        match self {
            Self::Some(entry) => Some(&entry.kind),
            Self::None { .. } => None,
        }
    }

    fn into_entry(self) -> Option<SearchIndexEntry> {
        match self {
            Self::Some(entry) => Some(entry),
            Self::None { .. } => None,
        }
    }
}

fn build_slot_entry(
    vault_dir: &Path,
    root: &Path,
    vault: &AsterVault,
    snapshot: Snapshot,
    plan: &SlotBuildPlan,
    previous_manifest: Option<&SearchIndexManifest>,
) -> CliResult<BuiltSlot> {
    let base_seq = snapshot.seq();
    let rows = collect_slot_rows_from_cf(vault, snapshot, plan)?;
    let row_count = rows.len();
    let entry = match rows {
        SlotRows::Dense(rows) => OptionalSearchIndexEntry::Some(dense::write(
            vault_dir, root, plan.slot, rows, base_seq,
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
                vault_dir, root, plan.slot, rows, base_seq, previous,
            )?)
        }
        SlotRows::AbsentOnly => OptionalSearchIndexEntry::None {
            slot: plan.slot.get(),
        },
    };
    Ok(BuiltSlot { entry, row_count })
}

enum SlotRows {
    Dense(dense::DenseSlotRows),
    Sparse(sparse::SparseSlotRows),
    Multi(multi::MultiSlotRows),
    AbsentOnly,
}

impl SlotRows {
    fn len(&self) -> usize {
        match self {
            Self::Dense(rows) => rows.len(),
            Self::Sparse(rows) => rows.len(),
            Self::Multi(rows) => rows.len(),
            Self::AbsentOnly => 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SlotRowShape {
    Dense,
    Sparse,
    Multi,
}

fn collect_slot_rows_from_cf(
    vault: &AsterVault,
    snapshot: Snapshot,
    plan: &SlotBuildPlan,
) -> CliResult<SlotRows> {
    let expected = plan.expected_ids.iter().copied().collect::<BTreeSet<_>>();
    let cf_rows = vault.scan_cf_snapshot(snapshot, ColumnFamily::slot(plan.slot))?;
    let mut found = BTreeSet::new();
    let mut shape = None;
    let mut dense_dim = None;
    let mut sparse_dim = None;
    let mut multi_token_dim = None;
    let mut dense_rows = Vec::new();
    let mut sparse_rows = Vec::new();
    let mut multi_rows = Vec::new();
    for (key, bytes) in cf_rows {
        let cx_id = cx_id_from_cf_key(&key, "slot CF")?;
        if !expected.contains(&cx_id) {
            continue;
        }
        if !found.insert(cx_id) {
            return Err(stale(format!(
                "slot CF repeats row for slot {} cx_id {cx_id}",
                plan.slot
            )));
        }
        let vector = decode_slot_vector(&bytes)?;
        vector.validate_schema().map_err(|err| {
            stale(format!(
                "slot {} cx {cx_id} has invalid payload: {}",
                plan.slot, err.message
            ))
        })?;
        match vector {
            SlotVector::Dense { dim, data } => {
                require_shape(&mut shape, SlotRowShape::Dense, plan.slot, cx_id)?;
                dense::validate_dense(plan.slot, cx_id, dim, &data)?;
                match dense_dim {
                    Some(expected_dim) if expected_dim != dim => {
                        return Err(stale(format!(
                            "slot {} has mixed dense dims: {expected_dim} and {dim}",
                            plan.slot
                        )));
                    }
                    None => dense_dim = Some(dim),
                    _ => {}
                }
                dense_rows.push((cx_id, data));
            }
            SlotVector::Sparse { dim, entries } => {
                require_shape(&mut shape, SlotRowShape::Sparse, plan.slot, cx_id)?;
                match sparse_dim {
                    Some(expected_dim) if expected_dim != dim => {
                        return Err(stale(format!(
                            "slot {} has mixed sparse dims: {expected_dim} and {dim}",
                            plan.slot
                        )));
                    }
                    None => sparse_dim = Some(dim),
                    _ => {}
                }
                sparse_rows.push((cx_id, entries));
            }
            SlotVector::Multi { token_dim, tokens } => {
                require_shape(&mut shape, SlotRowShape::Multi, plan.slot, cx_id)?;
                match multi_token_dim {
                    Some(expected_dim) if expected_dim != token_dim => {
                        return Err(stale(format!(
                            "slot {} has mixed multi token dims: {expected_dim} and {token_dim}",
                            plan.slot
                        )));
                    }
                    None => multi_token_dim = Some(token_dim),
                    _ => {}
                }
                multi_rows.push((cx_id, tokens));
            }
            SlotVector::Absent { .. } => {}
        }
    }
    if found.len() != expected.len() {
        let missing = expected
            .difference(&found)
            .next()
            .map(ToString::to_string)
            .unwrap_or_else(|| "<unknown>".to_string());
        return Err(CalyxError::aster_corrupt_shard(format!(
            "slot CF row missing for slot {} cx_id {missing}",
            plan.slot
        ))
        .into());
    }
    match shape {
        Some(SlotRowShape::Dense) => Ok(SlotRows::Dense(dense::DenseSlotRows {
            dim: dense_dim.expect("dense shape has dim"),
            rows: dense_rows,
        })),
        Some(SlotRowShape::Sparse) => Ok(SlotRows::Sparse(sparse::SparseSlotRows {
            dim: sparse_dim.expect("sparse shape has dim"),
            rows: sparse_rows,
        })),
        Some(SlotRowShape::Multi) => Ok(SlotRows::Multi(multi::MultiSlotRows {
            token_dim: multi_token_dim.expect("multi shape has token dim"),
            rows: multi_rows,
        })),
        None => Ok(SlotRows::AbsentOnly),
    }
}

fn require_shape(
    current: &mut Option<SlotRowShape>,
    next: SlotRowShape,
    slot: SlotId,
    cx_id: CxId,
) -> CliResult {
    match current {
        Some(existing) if *existing != next => Err(stale(format!(
            "slot {slot} mixes {existing:?} rows with {next:?} row at cx {cx_id}; reingest/backfill the vault"
        ))),
        Some(_) => Ok(()),
        None => {
            *current = Some(next);
            Ok(())
        }
    }
}

fn cx_id_from_cf_key(key: &[u8], cf_name: &str) -> calyx_core::Result<CxId> {
    let bytes: [u8; 16] = key.try_into().map_err(|_| {
        CalyxError::vault_access_denied(format!("{cf_name} key has {} bytes", key.len()))
    })?;
    Ok(CxId::from_bytes(bytes))
}

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
