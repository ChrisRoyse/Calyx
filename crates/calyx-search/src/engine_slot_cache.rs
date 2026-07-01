use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;
use std::time::Instant;

use calyx_core::{CalyxError, CxId, SlotId, SlotVector};
use calyx_sextant::IndexSearchHit;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::engine::{GuardChoice, SearchFreshness};
use crate::engine_measure::slot_vector_shape;
use crate::engine_trace::SearchTracer;
use crate::error::{CliError, CliResult};
use crate::persisted::PersistedSearchIndexes;

const CACHE_KEY_SCHEMA: &str = "calyx-search-slot-cache-key-v1";
const CACHE_KEY_ERROR: &str = "CALYX_SEARCH_SLOT_CACHE_KEY_INCOMPLETE";
const CACHE_KEY_REMEDIATION: &str = "include vault path, manifest fingerprint, freshness policy, guard, candidate universe, selected slots, search_k, and measured query vector hashes before reusing slot search results";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchSlotCacheDiagnostic {
    pub entry_count: usize,
    pub lookup_count: usize,
    pub hit_count: usize,
    pub miss_count: usize,
    pub stored_slot_count: usize,
    pub stored_hit_count: usize,
    pub stored_search_elapsed_ms: u128,
    pub last_key_sha256: Option<String>,
}

#[derive(Default, Debug)]
pub struct SearchSlotCache {
    entries: BTreeMap<String, CachedSearchSlots>,
    lookup_count: usize,
    hit_count: usize,
    miss_count: usize,
    last_key_sha256: Option<String>,
}

impl SearchSlotCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn diagnostics(&self) -> SearchSlotCacheDiagnostic {
        let mut stored_slot_count = 0usize;
        let mut stored_hit_count = 0usize;
        let mut stored_search_elapsed_ms = 0u128;
        for entry in self.entries.values() {
            stored_slot_count += entry.slot_elapsed_ms.len();
            stored_hit_count += entry.per_slot.values().map(Vec::len).sum::<usize>();
            stored_search_elapsed_ms += entry.search_elapsed_ms;
        }
        SearchSlotCacheDiagnostic {
            entry_count: self.entries.len(),
            lookup_count: self.lookup_count,
            hit_count: self.hit_count,
            miss_count: self.miss_count,
            stored_slot_count,
            stored_hit_count,
            stored_search_elapsed_ms,
            last_key_sha256: self.last_key_sha256.clone(),
        }
    }
}

#[derive(Clone, Debug)]
struct CachedSearchSlots {
    key: SearchSlotCacheKey,
    per_slot: BTreeMap<SlotId, Vec<IndexSearchHit>>,
    slot_elapsed_ms: BTreeMap<SlotId, u128>,
    search_elapsed_ms: u128,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct SearchSlotCacheKey {
    schema: &'static str,
    vault_dir: String,
    manifest_base_seq: u64,
    manifest_sha256: String,
    freshness: &'static str,
    guard: &'static str,
    search_k: usize,
    selected_slots: Vec<u16>,
    query_vectors: Vec<QueryVectorKey>,
    candidate_universe: CandidateUniverseKey,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct QueryVectorKey {
    slot: u16,
    kind: &'static str,
    value_count: usize,
    sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct CandidateUniverseKey {
    mode: &'static str,
    count: usize,
    sha256: Option<String>,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn search_slots_with_cache(
    indexes: &PersistedSearchIndexes,
    vault_dir: &Path,
    query_vectors: &[(SlotId, SlotVector)],
    k: usize,
    guard: GuardChoice,
    freshness: SearchFreshness,
    allowed_slots: Option<&BTreeSet<SlotId>>,
    filter_candidates: Option<&BTreeSet<CxId>>,
    cache: Option<&mut SearchSlotCache>,
    trace: &mut SearchTracer<'_>,
) -> CliResult<BTreeMap<SlotId, Vec<IndexSearchHit>>> {
    let Some(cache) = cache else {
        return search_slots_uncached(indexes, query_vectors, k, filter_candidates, trace)
            .map(|(per_slot, _)| per_slot);
    };

    let key = SearchSlotCacheKey::new(
        indexes,
        vault_dir,
        query_vectors,
        k,
        guard,
        freshness,
        allowed_slots,
        filter_candidates,
    )?;
    let key_sha256 = key.digest()?;
    cache.lookup_count += 1;
    cache.last_key_sha256 = Some(key_sha256.clone());
    trace.emit_detail(
        "search_slots.cache.lookup",
        None,
        Some(query_vectors.len()),
        Some(format!(
            "key_sha256={} entries={} manifest_base_seq={} manifest_sha256={}",
            key_sha256,
            cache.entries.len(),
            key.manifest_base_seq,
            key.manifest_sha256
        )),
    );

    if let Some(entry) = cache.entries.get(&key_sha256) {
        if entry.key != key {
            return Err(cache_key_error(format!(
                "slot search cache key digest collision key_sha256={key_sha256}"
            )));
        }
        cache.hit_count += 1;
        let slot_count = entry.slot_elapsed_ms.len();
        let hit_count = entry.per_slot.values().map(Vec::len).sum::<usize>();
        trace.emit_detail(
            "search_slots.cache.hit",
            None,
            Some(slot_count),
            Some(format!(
                "key_sha256={} hit_count={} source_elapsed_ms={}",
                key_sha256, hit_count, entry.search_elapsed_ms
            )),
        );
        for (slot, elapsed_ms) in &entry.slot_elapsed_ms {
            let count = entry.per_slot.get(slot).map_or(0, Vec::len);
            trace.emit_detail(
                "search_slot.cache_hit",
                Some(*slot),
                Some(count),
                Some(format!(
                    "key_sha256={} source_slot_elapsed_ms={}",
                    key_sha256, elapsed_ms
                )),
            );
        }
        return Ok(entry.per_slot.clone());
    }

    cache.miss_count += 1;
    trace.emit_detail(
        "search_slots.cache.miss",
        None,
        Some(query_vectors.len()),
        Some(format!(
            "key_sha256={} manifest_base_seq={} manifest_sha256={}",
            key_sha256, key.manifest_base_seq, key.manifest_sha256
        )),
    );
    let started = Instant::now();
    let (per_slot, slot_elapsed_ms) =
        search_slots_uncached(indexes, query_vectors, k, filter_candidates, trace)?;
    let search_elapsed_ms = started.elapsed().as_millis();
    let hit_count = per_slot.values().map(Vec::len).sum::<usize>();
    trace.emit_detail(
        "search_slots.cache.store",
        None,
        Some(slot_elapsed_ms.len()),
        Some(format!(
            "key_sha256={} hit_count={} search_elapsed_ms={}",
            key_sha256, hit_count, search_elapsed_ms
        )),
    );
    cache.entries.insert(
        key_sha256,
        CachedSearchSlots {
            key,
            per_slot: per_slot.clone(),
            slot_elapsed_ms,
            search_elapsed_ms,
        },
    );
    Ok(per_slot)
}

type SlotHitsWithLatency = (
    BTreeMap<SlotId, Vec<IndexSearchHit>>,
    BTreeMap<SlotId, u128>,
);

fn search_slots_uncached(
    indexes: &PersistedSearchIndexes,
    query_vectors: &[(SlotId, SlotVector)],
    k: usize,
    filter_candidates: Option<&BTreeSet<CxId>>,
    trace: &mut SearchTracer<'_>,
) -> CliResult<SlotHitsWithLatency> {
    let mut out = BTreeMap::new();
    let mut elapsed_by_slot = BTreeMap::new();
    for (slot, query) in query_vectors {
        trace.emit_detail(
            "search_slot.start",
            Some(*slot),
            Some(k),
            Some(slot_vector_shape(query)),
        );
        let started = Instant::now();
        let hits = if let Some(candidates) = filter_candidates {
            indexes.search_filtered(*slot, query, k, candidates)?
        } else {
            indexes.search(*slot, query, k)?
        };
        let slot_elapsed_ms = started.elapsed().as_millis();
        elapsed_by_slot.insert(*slot, slot_elapsed_ms);
        trace.emit_detail(
            "search_slot.done",
            Some(*slot),
            Some(hits.len()),
            Some(format!("slot_elapsed_ms={slot_elapsed_ms}")),
        );
        if !hits.is_empty() {
            out.insert(*slot, hits);
        }
    }
    Ok((out, elapsed_by_slot))
}

impl SearchSlotCacheKey {
    #[allow(clippy::too_many_arguments)]
    fn new(
        indexes: &PersistedSearchIndexes,
        vault_dir: &Path,
        query_vectors: &[(SlotId, SlotVector)],
        search_k: usize,
        guard: GuardChoice,
        freshness: SearchFreshness,
        allowed_slots: Option<&BTreeSet<SlotId>>,
        filter_candidates: Option<&BTreeSet<CxId>>,
    ) -> CliResult<Self> {
        let mut query_vectors = query_vectors
            .iter()
            .map(|(slot, vector)| query_vector_key(*slot, vector))
            .collect::<CliResult<Vec<_>>>()?;
        query_vectors.sort_by_key(|entry| entry.slot);
        Ok(Self {
            schema: CACHE_KEY_SCHEMA,
            vault_dir: canonical_vault_dir(vault_dir)?,
            manifest_base_seq: indexes.base_seq(),
            manifest_sha256: indexes.manifest_sha256().to_string(),
            freshness: freshness_name(freshness),
            guard: guard_name(guard),
            search_k,
            selected_slots: selected_slots(allowed_slots, &query_vectors),
            query_vectors,
            candidate_universe: candidate_universe_key(filter_candidates)?,
        })
    }

    fn digest(&self) -> CliResult<String> {
        let bytes = serde_json::to_vec(self).map_err(|err| {
            cache_key_error(format!("slot search cache key is not serializable: {err}"))
        })?;
        Ok(sha256_hex(&bytes))
    }
}

fn selected_slots(
    allowed_slots: Option<&BTreeSet<SlotId>>,
    query_vectors: &[QueryVectorKey],
) -> Vec<u16> {
    match allowed_slots {
        Some(slots) => slots.iter().map(|slot| slot.get()).collect(),
        None => query_vectors.iter().map(|entry| entry.slot).collect(),
    }
}

fn query_vector_key(slot: SlotId, vector: &SlotVector) -> CliResult<QueryVectorKey> {
    let bytes = serde_json::to_vec(vector).map_err(|err| {
        cache_key_error(format!(
            "slot search query vector for slot {slot} is not serializable: {err}"
        ))
    })?;
    Ok(QueryVectorKey {
        slot: slot.get(),
        kind: vector_kind(vector),
        value_count: vector_value_count(vector),
        sha256: sha256_hex(&bytes),
    })
}

fn candidate_universe_key(
    filter_candidates: Option<&BTreeSet<CxId>>,
) -> CliResult<CandidateUniverseKey> {
    let Some(candidates) = filter_candidates else {
        return Ok(CandidateUniverseKey {
            mode: "unfiltered",
            count: 0,
            sha256: None,
        });
    };
    let bytes = serde_json::to_vec(candidates).map_err(|err| {
        cache_key_error(format!(
            "slot search candidate universe is not serializable: {err}"
        ))
    })?;
    Ok(CandidateUniverseKey {
        mode: "filtered",
        count: candidates.len(),
        sha256: Some(sha256_hex(&bytes)),
    })
}

fn canonical_vault_dir(vault_dir: &Path) -> CliResult<String> {
    let canonical = std::fs::canonicalize(vault_dir).map_err(|err| {
        cache_key_error(format!(
            "slot search cache cannot canonicalize vault path {}: {err}",
            vault_dir.display()
        ))
    })?;
    Ok(canonical.display().to_string())
}

fn vector_kind(vector: &SlotVector) -> &'static str {
    match vector {
        SlotVector::Dense { .. } => "dense",
        SlotVector::Sparse { .. } => "sparse",
        SlotVector::Multi { .. } => "multi",
        SlotVector::Absent { .. } => "absent",
    }
}

fn vector_value_count(vector: &SlotVector) -> usize {
    match vector {
        SlotVector::Dense { data, .. } => data.len(),
        SlotVector::Sparse { entries, .. } => entries.len(),
        SlotVector::Multi { tokens, .. } => tokens.iter().map(Vec::len).sum(),
        SlotVector::Absent { .. } => 0,
    }
}

fn guard_name(guard: GuardChoice) -> &'static str {
    match guard {
        GuardChoice::Off => "off",
        GuardChoice::InRegion => "in_region",
    }
}

fn freshness_name(freshness: SearchFreshness) -> &'static str {
    match freshness {
        SearchFreshness::Fresh => "fresh",
        SearchFreshness::StaleOk => "stale_ok",
    }
}

fn cache_key_error(message: impl Into<String>) -> CliError {
    CalyxError {
        code: CACHE_KEY_ERROR,
        message: message.into(),
        remediation: CACHE_KEY_REMEDIATION,
    }
    .into()
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(&mut out, "{byte:02x}").expect("writing to String cannot fail");
    }
    out
}
