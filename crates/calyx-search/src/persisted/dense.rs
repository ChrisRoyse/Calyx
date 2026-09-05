use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

#[cfg(test)]
use calyx_core::Constellation;
use calyx_core::{CalyxError, CxId, SlotId, SlotVector};
use calyx_sextant::index::{
    DiskAnnBuildParams, DiskAnnSearch, DiskAnnSearchParams, IndexSearchHit, SextantIndex, ranked,
};

use super::rebuild::RebuildProgress;
use super::rebuild_plan::DiskAnnBuildPolicy;
use super::{SearchIndexEntry, SlotIdMap, rel, stale, write_json_atomic};
use crate::error::CliResult;

#[path = "dense/batch.rs"]
mod batch;
#[path = "dense/flat.rs"]
mod flat;
pub(super) use batch::search_batch;
#[cfg(feature = "cuda")]
pub(super) use batch::search_gpu_forced_batch;

#[derive(Debug, Default)]
pub(super) struct DenseIndexCache {
    indexes: Mutex<BTreeMap<(u16, bool), Arc<DiskAnnSearch>>>,
    /// A bounded CAGRA cache can evict an idle generation before admitting the
    /// next one, but it cannot safely evict thirteen simultaneously-loading
    /// generations. Serialize GPU dense slots through one admitted lane; CPU,
    /// sparse, and late-interaction work remains concurrent.
    gpu_search_lane: Mutex<()>,
}

pub(super) struct DenseSearchRuntime<'a> {
    pub gpu_serving: bool,
    pub cache: &'a DenseIndexCache,
}

pub(super) fn gpu_serving_required(backend: Option<&str>) -> CliResult<bool> {
    match backend {
        Some("cuvs-cagra") => Ok(true),
        Some("cpu-vamana") | None => Ok(false),
        Some(other) => Err(stale(format!(
            "persistent search manifest has unsupported diskann_build_backend {other:?}; rebuild"
        ))),
    }
}

#[derive(Clone, Debug)]
pub(super) struct DenseSlotRows {
    pub(super) dim: u32,
    pub(super) rows: Vec<(CxId, Vec<f32>)>,
}

impl DenseSlotRows {
    pub(super) fn len(&self) -> usize {
        self.rows.len()
    }
}

#[cfg(test)]
pub(super) fn slots(docs: &BTreeMap<CxId, Constellation>) -> Vec<SlotId> {
    let mut slots = docs
        .values()
        .flat_map(|cx| {
            cx.slots.iter().filter_map(|(slot, vector)| {
                matches!(vector, SlotVector::Dense { .. }).then_some(*slot)
            })
        })
        .collect::<Vec<_>>();
    slots.sort();
    slots.dedup();
    slots
}

#[cfg(test)]
pub(super) fn collect_slot(
    docs: &BTreeMap<CxId, Constellation>,
    target: SlotId,
) -> CliResult<DenseSlotRows> {
    let mut out: Option<DenseSlotRows> = None;
    for cx in docs.values() {
        let Some(vector) = cx.slots.get(&target) else {
            continue;
        };
        let SlotVector::Dense { dim, data } = vector else {
            continue;
        };
        validate_dense(target, cx.cx_id, *dim, data)?;
        let entry = out.get_or_insert_with(|| DenseSlotRows {
            dim: *dim,
            rows: Vec::new(),
        });
        if entry.dim != *dim {
            return Err(stale(format!(
                "slot {target} has mixed dense dims: {} and {dim}",
                entry.dim
            )));
        }
        entry.rows.push((cx.cx_id, data.clone()));
    }
    out.ok_or_else(|| stale(format!("slot {target} has no dense rows")))
}

pub(super) fn write_with_progress<F>(
    vault_dir: &Path,
    root: &Path,
    slot: SlotId,
    rows: DenseSlotRows,
    base_seq: u64,
    build_policy: DiskAnnBuildPolicy,
    mut progress: F,
) -> CliResult<SearchIndexEntry>
where
    F: FnMut(RebuildProgress<'_>) -> CliResult,
{
    if should_use_flat_dense_index(rows.rows.len()) {
        return flat::write(vault_dir, root, slot, rows, base_seq);
    }
    let dir_name = format!(
        "slot_{:05}_seq_{:020}_n_{:010}.ann",
        slot.get(),
        base_seq,
        rows.rows.len()
    );
    let dir = root.join(&dir_name);
    if dir.exists() {
        fs::remove_dir_all(&dir)?;
    }
    calyx_aster::private_fs::create_private_dir_all(&dir)?;
    let graph_path = dir.join("graph.cda");
    DiskAnnSearch::build_with_backend_and_progress(
        slot,
        &graph_path,
        &rows.rows,
        build_params(rows.dim as usize),
        None,
        search_params(rows.rows.len().max(64)),
        build_policy.backend,
        |event| {
            progress(RebuildProgress::slot(
                event.phase,
                slot,
                Some(event.rows),
                Some(base_seq),
            ))
            .map_err(CalyxError::from)
        },
    )?;
    let id_map_path = dir.join("ids.json");
    write_json_atomic(
        &id_map_path,
        &SlotIdMap {
            format: super::IDMAP_FORMAT.to_string(),
            slot: slot.get(),
            ids: rows.rows.iter().map(|(cx_id, _)| *cx_id).collect(),
        },
    )?;
    Ok(SearchIndexEntry::dense(
        slot,
        rows.dim,
        rows.rows.len(),
        base_seq,
        rel(vault_dir, &graph_path)?,
        rel(vault_dir, &id_map_path)?,
    ))
}

pub(super) fn search(
    vault_dir: &Path,
    entry: &SearchIndexEntry,
    slot: SlotId,
    query: &SlotVector,
    k: usize,
    gpu_serving: bool,
    cache: &DenseIndexCache,
) -> CliResult<Vec<IndexSearchHit>> {
    let _gpu_lane = lock_gpu_search_lane(cache, gpu_serving)?;
    if entry.kind == "flat_dense" {
        return flat::search(vault_dir, entry, slot, query, k, None, gpu_serving);
    }
    let SlotVector::Dense { dim, .. } = query else {
        return Err(stale(format!(
            "persistent dense search slot {slot} received non-dense query"
        )));
    };
    crate::engine_slot_fanout::record_slot_serving_detail(if gpu_serving {
        format!(
            "slot_backend=cuvs-cagra slot_compute_placement=cuda:0 source_rows={}",
            entry.len
        )
    } else {
        format!(
            "slot_backend=diskann-cpu-graph slot_compute_placement=cpu source_rows={}",
            entry.len
        )
    });
    open_cached(vault_dir, entry, slot, *dim, k, gpu_serving, cache)?
        .search(query, want(k, entry.len), Some(want(k, entry.len).max(64)))
        .map_err(Into::into)
}

pub(super) fn search_filtered(
    vault_dir: &Path,
    entry: &SearchIndexEntry,
    slot: SlotId,
    query: &SlotVector,
    k: usize,
    candidates: &BTreeSet<CxId>,
    runtime: DenseSearchRuntime<'_>,
) -> CliResult<Vec<IndexSearchHit>> {
    let _gpu_lane = lock_gpu_search_lane(runtime.cache, runtime.gpu_serving)?;
    if entry.kind == "flat_dense" {
        return flat::search(
            vault_dir,
            entry,
            slot,
            query,
            k,
            Some(candidates),
            runtime.gpu_serving,
        );
    }
    let SlotVector::Dense { dim, data } = query else {
        return Err(stale(format!(
            "persistent dense filtered search slot {slot} received non-dense query"
        )));
    };
    let index = open_cached(
        vault_dir,
        entry,
        slot,
        *dim,
        k,
        runtime.gpu_serving,
        runtime.cache,
    )?;
    if runtime.gpu_serving {
        let allowed = candidates
            .iter()
            .filter_map(|cx_id| index.local_id(*cx_id))
            .collect::<Vec<_>>();
        if k == 0 || allowed.is_empty() {
            return Ok(Vec::new());
        }
        let scored = index
            .search_ids_filtered_cuda(
                data,
                want(k, allowed.len()),
                &search_params(k.max(64)),
                &allowed,
            )?
            .into_iter()
            .filter_map(|(id, distance)| index.cx_id(id).map(|cx_id| (cx_id, 1.0 - distance)))
            .collect();
        return Ok(ranked(scored));
    }
    exact_filtered_hits(&index, data, k, candidates)
}

/// Forced GPU dense candidate search (#1963, weave `--candidate-device gpu`).
/// Bypasses the implicit manifest-backend / row-count gating of [`search`] and
/// requires the GPU kernel: flat sidecars route through the resident cuVS flat
/// index regardless of the `CALYX_SEARCH_FLAT_CUDA_MIN_ROWS` floor, and DiskANN
/// sidecars open in CUDA serving mode (which fails closed when the dataset-
/// inclusive CAGRA asset is absent — i.e. the index must be built cuvs-cagra).
#[cfg(feature = "cuda")]
pub(super) fn search_gpu_forced(
    vault_dir: &Path,
    entry: &SearchIndexEntry,
    slot: SlotId,
    query: &SlotVector,
    k: usize,
    cache: &DenseIndexCache,
) -> CliResult<Vec<IndexSearchHit>> {
    let _gpu_lane = lock_gpu_search_lane(cache, true)?;
    if k == 0 {
        return Ok(Vec::new());
    }
    if entry.kind == "flat_dense" {
        return flat::search_gpu_forced(vault_dir, entry, slot, query, k);
    }
    let SlotVector::Dense { dim, .. } = query else {
        return Err(stale(format!(
            "persistent dense GPU candidate search slot {slot} received non-dense query"
        )));
    };
    open_cached(vault_dir, entry, slot, *dim, k, true, cache)?
        .search(query, want(k, entry.len), Some(want(k, entry.len).max(64)))
        .map_err(Into::into)
}

pub(super) fn lock_gpu_search_lane(
    cache: &DenseIndexCache,
    gpu_serving: bool,
) -> CliResult<Option<MutexGuard<'_, ()>>> {
    gpu_serving
        .then(|| {
            cache
                .gpu_search_lane
                .lock()
                .map_err(|_| stale("persistent dense GPU search lane lock poisoned"))
        })
        .transpose()
}

pub(super) fn open_cached(
    vault_dir: &Path,
    entry: &SearchIndexEntry,
    slot: SlotId,
    query_dim: u32,
    k: usize,
    gpu_serving: bool,
    cache: &DenseIndexCache,
) -> CliResult<Arc<DiskAnnSearch>> {
    entry.require_kind("diskann", slot)?;
    let indexed_dim = entry.require_dim(slot)?;
    if indexed_dim != query_dim {
        return Err(stale(format!(
            "persistent slot {slot} index dim {indexed_dim} != query dim {query_dim}; reingest/backfill the vault"
        )));
    }
    let cache_key = (slot.get(), gpu_serving);
    if let Some(index) = cache
        .indexes
        .lock()
        .map_err(|_| stale("persistent dense index cache lock poisoned"))?
        .get(&cache_key)
        .cloned()
    {
        return Ok(index);
    }
    let index = Arc::new(open(vault_dir, entry, slot, query_dim, k, gpu_serving)?);
    let mut indexes = cache
        .indexes
        .lock()
        .map_err(|_| stale("persistent dense index cache lock poisoned"))?;
    Ok(Arc::clone(indexes.entry(cache_key).or_insert(index)))
}

fn open(
    vault_dir: &Path,
    entry: &SearchIndexEntry,
    slot: SlotId,
    query_dim: u32,
    k: usize,
    gpu_serving: bool,
) -> CliResult<DiskAnnSearch> {
    entry.require_kind("diskann", slot)?;
    let dim = entry.require_dim(slot)?;
    if dim != query_dim {
        return Err(stale(format!(
            "persistent slot {slot} index dim {dim} != query dim {query_dim}; reingest/backfill the vault"
        )));
    }
    let ids = read_ids(vault_dir, entry, slot)?;
    if ids.len() != entry.len {
        return Err(stale(format!(
            "persistent slot {slot} id map len {} != manifest len {}",
            ids.len(),
            entry.len
        )));
    }
    let graph = vault_dir.join(entry.require_graph_rel(slot)?);
    let mut index = if gpu_serving {
        DiskAnnSearch::open_gpu_serving(slot, graph, ids, None, search_params(k.max(64)))?
    } else {
        DiskAnnSearch::open(slot, graph, ids, None, search_params(k.max(64)))?
    };
    index.set_base_seq(entry.built_at_seq);
    Ok(index)
}

fn read_ids(vault_dir: &Path, entry: &SearchIndexEntry, slot: SlotId) -> CliResult<Vec<CxId>> {
    let path = vault_dir.join(entry.require_id_map_rel(slot)?);
    let map: SlotIdMap = serde_json::from_slice(&fs::read(&path)?)?;
    if map.format != super::IDMAP_FORMAT {
        return Err(stale(format!(
            "persistent slot {slot} id map {} has format {}; expected {}",
            path.display(),
            map.format,
            super::IDMAP_FORMAT
        )));
    }
    if map.slot != entry.slot {
        return Err(stale(format!(
            "persistent id map slot {} != manifest slot {}",
            map.slot, entry.slot
        )));
    }
    Ok(map.ids)
}

pub(super) fn validate_entry(
    vault_dir: &Path,
    entry: &SearchIndexEntry,
    slot: SlotId,
) -> CliResult {
    if entry.kind == "flat_dense" {
        return flat::validate_entry(vault_dir, entry, slot);
    }
    entry.require_kind("diskann", slot)?;
    let graph = vault_dir.join(entry.require_graph_rel(slot)?);
    if !graph.is_file() {
        return Err(stale(format!(
            "persistent slot {slot} graph sidecar missing at {}; rebuild the vault search indexes",
            graph.display()
        )));
    }
    let graph_len = fs::metadata(&graph)?.len();
    if graph_len == 0 {
        return Err(stale(format!(
            "persistent slot {slot} graph sidecar {} is empty; rebuild the vault search indexes",
            graph.display()
        )));
    }
    let ids = read_ids(vault_dir, entry, slot)?;
    if ids.len() != entry.len {
        return Err(stale(format!(
            "persistent slot {slot} id map len {} != manifest len {}",
            ids.len(),
            entry.len
        )));
    }
    let mut seen = BTreeSet::new();
    for id in ids {
        if !seen.insert(id) {
            return Err(stale(format!(
                "persistent slot {slot} id map repeats {id}; rebuild the vault search indexes"
            )));
        }
    }
    Ok(())
}

fn exact_filtered_hits(
    index: &DiskAnnSearch,
    query: &[f32],
    k: usize,
    candidates: &BTreeSet<CxId>,
) -> CliResult<Vec<IndexSearchHit>> {
    if k == 0 {
        return Ok(Vec::new());
    }
    let mut scored = Vec::new();
    for cx_id in candidates {
        let Some(vector) = index.vector(*cx_id) else {
            continue;
        };
        let Some(values) = vector.as_dense() else {
            return Err(stale(format!(
                "persistent filtered search candidate {cx_id} is not dense; rebuild the vault search indexes"
            )));
        };
        if values.len() != query.len() {
            return Err(stale(format!(
                "persistent filtered search candidate {cx_id} dim {} != query dim {}; rebuild the vault search indexes",
                values.len(),
                query.len()
            )));
        }
        scored.push((*cx_id, cosine(query, values)));
    }
    scored.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.to_string().cmp(&right.0.to_string()))
    });
    scored.truncate(k);
    Ok(ranked(scored))
}

pub(super) fn should_use_flat_dense_index(row_count: usize) -> bool {
    flat::should_use_index(row_count)
}

pub(super) fn validate_dense(slot: SlotId, cx_id: CxId, dim: u32, data: &[f32]) -> CliResult {
    if dim == 0 || data.len() != dim as usize {
        return Err(CalyxError::lens_dim_mismatch(format!(
            "slot {slot} cx {cx_id} dense len {} != dim {dim}",
            data.len()
        ))
        .into());
    }
    if data.iter().any(|value| !value.is_finite()) {
        return Err(CalyxError::lens_numerical_invariant(format!(
            "slot {slot} cx {cx_id} has non-finite dense component"
        ))
        .into());
    }
    Ok(())
}

fn build_params(dim: usize) -> DiskAnnBuildParams {
    DiskAnnBuildParams {
        dim,
        m_max: 32,
        ef_construction: 64,
        alpha: 1.2,
    }
}

fn search_params(ef: usize) -> DiskAnnSearchParams {
    DiskAnnSearchParams {
        beamwidth: 32,
        ef_search: ef,
        rescore_k: ef,
        rescore_from_raw: false,
    }
}

fn want(k: usize, len: usize) -> usize {
    k.max(1).min(len)
}

fn cosine(left: &[f32], right: &[f32]) -> f32 {
    let (mut dot, mut left_l2, mut right_l2) = (0.0, 0.0, 0.0);
    for (left, right) in left.iter().zip(right) {
        dot += left * right;
        left_l2 += left * left;
        right_l2 += right * right;
    }
    if left_l2 == 0.0 || right_l2 == 0.0 {
        0.0
    } else {
        dot / (left_l2.sqrt() * right_l2.sqrt())
    }
}
