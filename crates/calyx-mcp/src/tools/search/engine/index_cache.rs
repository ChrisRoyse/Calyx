use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use calyx_aster::vault::AsterVault;
use calyx_core::{Constellation, CxId, SlotId, SlotVector, VaultStore};
use calyx_sextant::{HnswIndex, IndexSearchHit, InvertedIndex, MaxSimIndex, SlotIndexMap};

use crate::server::ToolResult;

use super::{HNSW_SEED, LoadedDocs, indexable, load_docs, same_index_shape};

#[derive(Clone)]
pub(super) struct IndexedDocs {
    pub(super) docs: BTreeMap<CxId, Constellation>,
    pub(super) snapshot_seq: u64,
    indexes: SlotIndexMap,
    samples: BTreeMap<SlotId, SlotVector>,
}

#[derive(Clone, PartialEq, Eq)]
struct CacheKey {
    vault_path: PathBuf,
    snapshot_seq: u64,
}

#[derive(Clone)]
struct CacheEntry {
    key: CacheKey,
    indexed: IndexedDocs,
}

#[derive(Default)]
struct CacheState {
    entry: Option<CacheEntry>,
    builds: usize,
    hits: usize,
}

static CACHE: OnceLock<Mutex<CacheState>> = OnceLock::new();

pub(super) fn load_indexed_docs(vault_path: &Path, vault: &AsterVault) -> ToolResult<IndexedDocs> {
    let key = CacheKey {
        vault_path: vault_path.to_path_buf(),
        snapshot_seq: vault.snapshot(),
    };
    if let Some(indexed) = cached(&key) {
        return Ok(indexed);
    }
    let loaded = load_docs(vault)?;
    let indexed = index_loaded_docs(loaded)?;
    store(key, indexed.clone());
    Ok(indexed)
}

pub(super) fn search_indexed_slots(
    indexed: &IndexedDocs,
    query_vectors: &[(SlotId, SlotVector)],
) -> ToolResult<BTreeMap<SlotId, Vec<IndexSearchHit>>> {
    let mut out = BTreeMap::new();
    for (slot, query) in query_vectors {
        let hits = search_indexed_slot(indexed, *slot, query)?;
        if !hits.is_empty() {
            out.insert(*slot, hits);
        }
    }
    Ok(out)
}

pub(super) fn search_indexed_slot(
    indexed: &IndexedDocs,
    slot: SlotId,
    query: &SlotVector,
) -> ToolResult<Vec<IndexSearchHit>> {
    if indexed
        .samples
        .get(&slot)
        .is_some_and(|sample| same_index_shape(sample, query))
        && let Some(len) = indexed_len(&indexed.indexes, slot)
    {
        if len == 0 {
            return Ok(Vec::new());
        }
        return Ok(indexed
            .indexes
            .search(slot, query, len, Some(len.max(64)))?);
    }
    search_one_slot(&indexed.docs, slot, query, indexed.snapshot_seq)
}

pub(super) fn search_slots(
    docs: &BTreeMap<CxId, Constellation>,
    query_vectors: &[(SlotId, SlotVector)],
    snapshot_seq: u64,
) -> ToolResult<BTreeMap<SlotId, Vec<IndexSearchHit>>> {
    let mut out = BTreeMap::new();
    for (slot, query) in query_vectors {
        let hits = search_one_slot(docs, *slot, query, snapshot_seq)?;
        if !hits.is_empty() {
            out.insert(*slot, hits);
        }
    }
    Ok(out)
}

fn cached(key: &CacheKey) -> Option<IndexedDocs> {
    let mut cache = cache().lock().expect("search index cache poisoned");
    let hit = cache
        .entry
        .as_ref()
        .filter(|entry| entry.key == *key)
        .map(|entry| entry.indexed.clone());
    if hit.is_some() {
        cache.hits += 1;
    }
    hit
}

fn store(key: CacheKey, indexed: IndexedDocs) {
    let mut cache = cache().lock().expect("search index cache poisoned");
    cache.entry = Some(CacheEntry { key, indexed });
    cache.builds += 1;
}

fn cache() -> &'static Mutex<CacheState> {
    CACHE.get_or_init(|| Mutex::new(CacheState::default()))
}

fn index_loaded_docs(loaded: LoadedDocs) -> ToolResult<IndexedDocs> {
    let indexes = SlotIndexMap::new();
    let samples = first_vectors(&loaded.docs);
    for (slot, vector) in &samples {
        match vector {
            SlotVector::Dense { dim, .. } => {
                indexes.register(HnswIndex::new(*slot, *dim, HNSW_SEED))?
            }
            SlotVector::Sparse { .. } => indexes.register(InvertedIndex::new(*slot))?,
            SlotVector::Multi { token_dim, .. } => {
                indexes.register(MaxSimIndex::new(*slot, *token_dim))?
            }
            SlotVector::Absent { .. } => {}
        }
    }
    for cx in loaded.docs.values() {
        for (slot, vector) in &cx.slots {
            if samples
                .get(slot)
                .is_some_and(|sample| same_index_shape(sample, vector))
            {
                indexes.insert(*slot, cx.cx_id, vector.clone(), loaded.snapshot_seq)?;
            }
        }
    }
    for slot in indexes.registered_slots() {
        indexes.set_base_seq(slot, loaded.snapshot_seq)?;
    }
    Ok(IndexedDocs {
        docs: loaded.docs,
        snapshot_seq: loaded.snapshot_seq,
        indexes,
        samples,
    })
}

fn search_one_slot(
    docs: &BTreeMap<CxId, Constellation>,
    slot: SlotId,
    query: &SlotVector,
    snapshot_seq: u64,
) -> ToolResult<Vec<IndexSearchHit>> {
    let mut index = new_index(slot, query)?;
    let mut inserted = 0usize;
    for cx in docs.values() {
        if let Some(vector) = cx.slots.get(&slot)
            && same_index_shape(query, vector)
        {
            index.insert(cx.cx_id, vector.clone(), snapshot_seq)?;
            inserted += 1;
        }
    }
    if inserted == 0 {
        return Ok(Vec::new());
    }
    Ok(index.search(query, inserted, Some(inserted.max(64)))?)
}

fn new_index(slot: SlotId, query: &SlotVector) -> ToolResult<Box<dyn calyx_sextant::SextantIndex>> {
    match query {
        SlotVector::Dense { dim, .. } => Ok(Box::new(HnswIndex::new(slot, *dim, HNSW_SEED))),
        SlotVector::Sparse { .. } => Ok(Box::new(InvertedIndex::new(slot))),
        SlotVector::Multi { token_dim, .. } => Ok(Box::new(MaxSimIndex::new(slot, *token_dim))),
        SlotVector::Absent { .. } => Err(crate::server::ToolError::invalid_params(
            "query slot vector must be concrete",
        )),
    }
}

fn first_vectors(docs: &BTreeMap<CxId, Constellation>) -> BTreeMap<SlotId, SlotVector> {
    let mut out = BTreeMap::new();
    for cx in docs.values() {
        for (slot, vector) in &cx.slots {
            if indexable(vector) {
                out.entry(*slot).or_insert_with(|| vector.clone());
            }
        }
    }
    out
}

fn indexed_len(indexes: &SlotIndexMap, slot: SlotId) -> Option<usize> {
    indexes
        .stats()
        .into_iter()
        .find(|stats| stats.slot == slot)
        .map(|stats| stats.len)
}

#[cfg(test)]
pub(super) fn reset_for_tests() {
    *cache().lock().expect("search index cache poisoned") = CacheState::default();
}

#[cfg(test)]
pub(super) fn stats_for_tests() -> (usize, usize) {
    let cache = cache().lock().expect("search index cache poisoned");
    (cache.builds, cache.hits)
}
