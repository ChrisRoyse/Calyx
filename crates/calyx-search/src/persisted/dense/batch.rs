use std::path::Path;

use calyx_core::{SlotId, SlotVector};
use calyx_sextant::index::{IndexSearchHit, ranked};

use super::{DenseIndexCache, flat, lock_gpu_search_lane, open_cached, search_params, want};
use crate::error::CliResult;
use crate::persisted::{SearchIndexEntry, stale};

pub(in crate::persisted) fn search_batch(
    vault_dir: &Path,
    entry: &SearchIndexEntry,
    slot: SlotId,
    queries: &[SlotVector],
    k: usize,
    gpu_serving: bool,
    cache: &DenseIndexCache,
) -> CliResult<Vec<Vec<IndexSearchHit>>> {
    let _gpu_lane = lock_gpu_search_lane(cache, gpu_serving)?;
    if queries.is_empty() {
        return Ok(Vec::new());
    }
    if entry.kind == "flat_dense" {
        return queries
            .iter()
            .map(|query| flat::search(vault_dir, entry, slot, query, k, None, gpu_serving))
            .collect();
    }
    let dim = entry.require_dim(slot)?;
    let mut flattened = Vec::new();
    flattened
        .try_reserve_exact(queries.len().saturating_mul(dim as usize))
        .map_err(|_| {
            stale(format!(
                "persistent dense slot {slot} query batch allocation failed"
            ))
        })?;
    for query in queries {
        let SlotVector::Dense {
            dim: query_dim,
            data,
        } = query
        else {
            return Err(stale(format!(
                "persistent dense batch search slot {slot} received non-dense query"
            )));
        };
        if *query_dim != dim {
            return Err(stale(format!(
                "persistent slot {slot} index dim {dim} != query dim {query_dim}; reingest/backfill the vault"
            )));
        }
        flattened.extend_from_slice(data);
    }
    let index = open_cached(vault_dir, entry, slot, dim, k, gpu_serving, cache)?;
    let params = search_params(want(k, entry.len).max(64));
    let rows = index.search_ids_batch(&flattened, queries.len(), want(k, entry.len), &params)?;
    rows.into_iter()
        .map(|row| {
            let scored = row
                .into_iter()
                .map(|(id, distance)| {
                    index
                        .cx_id(id)
                        .map(|cx_id| (cx_id, 1.0 - distance))
                        .ok_or_else(|| {
                            stale(format!(
                                "persistent dense slot {slot} returned out-of-range local id {id}"
                            ))
                        })
                })
                .collect::<CliResult<Vec<_>>>()?;
            Ok(ranked(scored))
        })
        .collect()
}

#[cfg(feature = "cuda")]
pub(in crate::persisted) fn search_gpu_forced_batch(
    vault_dir: &Path,
    entry: &SearchIndexEntry,
    slot: SlotId,
    queries: &[SlotVector],
    k: usize,
    cache: &DenseIndexCache,
) -> CliResult<Vec<Vec<IndexSearchHit>>> {
    search_batch(vault_dir, entry, slot, queries, k, true, cache)
}
