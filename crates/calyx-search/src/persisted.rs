#[path = "persisted/dense.rs"]
mod dense;
#[path = "persisted/filter.rs"]
mod filter;
#[path = "persisted/freshness.rs"]
mod freshness;
#[path = "persisted/generation.rs"]
mod generation;
#[path = "persisted/marker.rs"]
pub mod marker;
#[cfg(test)]
#[path = "persisted/mixed_tests.rs"]
mod mixed_tests;
#[path = "persisted/multi.rs"]
mod multi;
#[path = "persisted/pinned.rs"]
mod pinned;
pub use pinned::{SEARCH_PIN_BUDGET_EXCEEDED_CODE, SEARCH_PIN_BUDGET_EXCEEDED_REMEDIATION};
#[path = "persisted/rebuild.rs"]
mod rebuild;
#[path = "persisted/rebuild_plan.rs"]
mod rebuild_plan;
#[path = "persisted/rebuild_stream.rs"]
mod rebuild_stream;
#[path = "persisted/sparse.rs"]
mod sparse;
#[cfg(test)]
#[path = "persisted/tests.rs"]
mod tests;
use calyx_aster::vault::AsterVault;
use calyx_core::{CalyxError, Constellation, CxId, SlotId, SlotVector};
use calyx_sextant::QueryFilters;
use calyx_sextant::index::IndexSearchHit;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::error::{CliError, CliResult};
pub use generation::{PersistedSearchGeneration, PersistedSearchSlot};
pub use marker::{
    MarkerClearOutcome, REBUILD_REQUIRED_REMEDIATION, REBUILD_REQUIRED_SCHEMA,
    RebuildRequiredMarker, clear_rebuild_required_marker, clear_rebuild_required_marker_if_owned,
    read_rebuild_required_marker, rebuild_required_marker_path,
    replace_rebuild_required_marker_if_exact, write_rebuild_required_marker,
};
#[cfg(feature = "cuda")]
pub(crate) use multi::take_maxsim_cuda_detail;
pub(crate) use pinned::canonical_vault_dir as canonical_pin_vault_dir;
pub(crate) use rebuild::load_docs_at;
#[cfg(test)]
use rebuild::rebuild_from_docs;
pub use rebuild::{
    RebuildProgress, load_docs, rebuild_for_vault, rebuild_for_vault_with_fallible_progress,
    rebuild_for_vault_with_panel_state, rebuild_for_vault_with_panel_state_fallible_progress,
    rebuild_for_vault_with_panel_state_progress, rebuild_for_vault_with_progress,
    rebuild_for_vault_with_stored_panel_state_fallible_progress,
};

const MANIFEST_FORMAT: &str = "calyx-search-index-manifest-v1";
const IDMAP_FORMAT: &str = "calyx-search-index-idmap-v1";
const INDEX_ROOT: &str = "idx/search";
const MANIFEST_NAME: &str = "manifest.json";

/// Fail-closed error code raised when GPU candidate generation is requested
/// (weave-loom `--candidate-device gpu`, #1963) on a binary built without
/// `--features cuda`. Without the feature there is no cuVS/CUDA surface at all,
/// so the request can never be honoured and we refuse rather than silently
/// serving the CPU scan.
pub const WEAVE_CANDIDATE_CUDA_FEATURE_MISSING: &str = "CALYX_WEAVE_CANDIDATE_CUDA_FEATURE_MISSING";

#[cfg(not(feature = "cuda"))]
fn weave_candidate_cuda_feature_missing() -> CliError {
    CalyxError {
        code: WEAVE_CANDIDATE_CUDA_FEATURE_MISSING,
        message:
            "weave candidate search requested --candidate-device gpu but calyx was built without \
             --features cuda; there is no GPU search surface in this binary"
                .to_string(),
        remediation:
            "rebuild calyx-cli with --features cuda for GPU candidate generation, or rerun \
             weave-loom with --candidate-device cpu",
    }
    .into()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct SearchIndexManifest {
    format: String,
    base_seq: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    diskann_build_backend: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    diskann_build_backend_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sextant_cuvs_compiled: Option<bool>,
    #[serde(default)]
    filter: Option<FilterIndexEntry>,
    slots: Vec<SearchIndexEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct SearchIndexEntry {
    slot: u16,
    kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    dim: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    token_dim: Option<u32>,
    len: usize,
    built_at_seq: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    graph_rel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    id_map_rel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    id_map_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    index_rel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    token_count: Option<usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct SlotIdMap {
    format: String,
    slot: u16,
    ids: Vec<CxId>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct FilterIndexEntry {
    built_at_seq: u64,
    len: usize,
    index_rel: String,
    sha256: String,
}

#[derive(Clone, Debug)]
struct RebuildSummary {
    slots: usize,
    total_rows: usize,
    manifest_path: PathBuf,
}

#[derive(Debug)]
pub struct PersistedSearchIndexes {
    vault_dir: PathBuf,
    manifest: SearchIndexManifest,
    manifest_sha256: String,
    dense_cache: dense::DenseIndexCache,
    #[cfg(feature = "cuda")]
    multi_token_ann_cache: multi::TokenAnnCache,
}

impl PersistedSearchIndexes {
    pub fn open(vault_dir: &Path) -> CliResult<Self> {
        let manifest_path = manifest_path(vault_dir);
        if !manifest_path.is_file() {
            return Err(stale(format!(
                "persistent search index manifest missing at {}; ingest or rebuild the vault before search{}",
                manifest_path.display(),
                marker::marker_error_context(vault_dir)
            )));
        }
        let manifest_bytes = fs::read(&manifest_path)?;
        let manifest_sha256 = sha256_hex(&manifest_bytes);
        let manifest: SearchIndexManifest = serde_json::from_slice(&manifest_bytes)?;
        if manifest.format != MANIFEST_FORMAT {
            return Err(stale(format!(
                "persistent search index manifest {} has format {}; expected {MANIFEST_FORMAT}",
                manifest_path.display(),
                manifest.format
            )));
        }
        Ok(Self {
            vault_dir: vault_dir.to_path_buf(),
            manifest,
            manifest_sha256,
            dense_cache: dense::DenseIndexCache::default(),
            #[cfg(feature = "cuda")]
            multi_token_ann_cache: multi::TokenAnnCache::default(),
        })
    }

    pub fn uses_gpu_dense_serving(&self) -> CliResult<bool> {
        dense::gpu_serving_required(self.manifest.diskann_build_backend.as_deref())
    }

    pub fn search(
        &self,
        slot: SlotId,
        query: &SlotVector,
        k: usize,
    ) -> CliResult<Vec<IndexSearchHit>> {
        let entry = self.require_entry(slot)?;
        let gpu_serving = self.uses_gpu_dense_serving()?;
        match query {
            SlotVector::Dense { .. } => dense::search(
                &self.vault_dir,
                entry,
                slot,
                query,
                k,
                gpu_serving,
                &self.dense_cache,
            ),
            SlotVector::Sparse { .. } => sparse::search(
                &self.vault_dir,
                entry,
                self.manifest.base_seq,
                slot,
                query,
                k,
                None,
            ),
            SlotVector::Multi { .. } | SlotVector::SegmentedMulti { .. } => multi::search(
                &self.vault_dir,
                entry,
                self.manifest.base_seq,
                slot,
                query,
                k,
                None,
            ),
            SlotVector::Absent { .. } => Err(stale(format!(
                "persistent search slot {slot} received an absent query vector; remeasure the active panel"
            ))),
        }
    }

    /// Shape-native batch candidate search over one slot generation. Dense
    /// DiskANN/CAGRA queries use the index's real batch API and one validated,
    /// pinned open. Sparse and multi-vector lanes retain their native search
    /// implementations but run slot-major so their verified resident indexes
    /// are reused instead of cycling generations per document.
    pub fn search_batch(
        &self,
        slot: SlotId,
        queries: &[SlotVector],
        k: usize,
    ) -> CliResult<Vec<Vec<IndexSearchHit>>> {
        if queries.is_empty() {
            return Ok(Vec::new());
        }
        let entry = self.require_entry(slot)?;
        match queries.first().expect("non-empty query batch") {
            SlotVector::Dense { .. } => {
                let gpu_serving = self.uses_gpu_dense_serving()?;
                dense::search_batch(
                    &self.vault_dir,
                    entry,
                    slot,
                    queries,
                    k,
                    gpu_serving,
                    &self.dense_cache,
                )
            }
            SlotVector::Sparse { .. }
            | SlotVector::Multi { .. }
            | SlotVector::SegmentedMulti { .. } => queries
                .iter()
                .map(|query| self.search(slot, query, k))
                .collect(),
            SlotVector::Absent { .. } => Err(stale(format!(
                "persistent search slot {slot} received an absent batch query vector; remeasure the active panel"
            ))),
        }
    }

    pub fn search_filtered(
        &self,
        slot: SlotId,
        query: &SlotVector,
        k: usize,
        candidates: &BTreeSet<CxId>,
    ) -> CliResult<Vec<IndexSearchHit>> {
        if candidates.is_empty() {
            return Ok(Vec::new());
        }
        let entry = self.require_entry(slot)?;
        let gpu_serving = self.uses_gpu_dense_serving()?;
        match query {
            SlotVector::Dense { .. } => dense::search_filtered(
                &self.vault_dir,
                entry,
                slot,
                query,
                k,
                candidates,
                dense::DenseSearchRuntime {
                    gpu_serving,
                    cache: &self.dense_cache,
                },
            ),
            SlotVector::Sparse { .. } => sparse::search(
                &self.vault_dir,
                entry,
                self.manifest.base_seq,
                slot,
                query,
                k,
                Some(candidates),
            ),
            SlotVector::Multi { .. } | SlotVector::SegmentedMulti { .. } => multi::search(
                &self.vault_dir,
                entry,
                self.manifest.base_seq,
                slot,
                query,
                k,
                Some(candidates),
            ),
            SlotVector::Absent { .. } => Err(stale(format!(
                "persistent filtered search slot {slot} received an absent query vector; remeasure the active panel"
            ))),
        }
    }

    /// Explicit GPU candidate-generation entry point for weave-loom between-doc
    /// search (#1963). Unlike [`Self::search`] — whose GPU use is *implicit*,
    /// keyed off the manifest build backend plus per-lane row/token thresholds —
    /// this **forces** the GPU search kernel for the slot's lane and fails closed
    /// (never silently degrades to the CPU scan) whenever the GPU path cannot be
    /// honoured, so a `--candidate-device gpu` run can never quietly fall back.
    ///
    /// Candidate generation is *approximate selection only*: the bit-stable
    /// per-pair similarity evidence is still produced downstream by the CPU
    /// rerank (`rank_typed_panel_candidate_refs`) over the returned union, so the
    /// persisted `SlotSimilarityEvidence` is byte-identical regardless of the
    /// device used to select the candidates (#1960 equality guarantees intact).
    pub fn search_candidates_gpu(
        &self,
        slot: SlotId,
        query: &SlotVector,
        k: usize,
    ) -> CliResult<Vec<IndexSearchHit>> {
        #[cfg(not(feature = "cuda"))]
        {
            let _ = (slot, query, k);
            Err(weave_candidate_cuda_feature_missing())
        }
        #[cfg(feature = "cuda")]
        {
            let entry = self.require_entry(slot)?;
            match query {
                SlotVector::Dense { .. } => dense::search_gpu_forced(
                    &self.vault_dir,
                    entry,
                    slot,
                    query,
                    k,
                    &self.dense_cache,
                ),
                SlotVector::Sparse { .. } => sparse::search_gpu_forced(
                    &self.vault_dir,
                    entry,
                    self.manifest.base_seq,
                    slot,
                    query,
                    k,
                ),
                SlotVector::Multi { .. } | SlotVector::SegmentedMulti { .. } => {
                    multi::search_gpu_forced(
                        &self.vault_dir,
                        entry,
                        self.manifest.base_seq,
                        slot,
                        query,
                        k,
                        &self.multi_token_ann_cache,
                    )
                }
                SlotVector::Absent { .. } => Err(stale(format!(
                    "persistent GPU candidate search slot {slot} received an absent query vector; remeasure the active panel"
                ))),
            }
        }
    }

    /// Forced-GPU counterpart of [`Self::search_batch`]. Dense lanes issue one
    /// CAGRA launch per batch; every non-dense query stays on its explicit
    /// shape-native CUDA path. Any unavailable GPU surface is an error, never a
    /// CPU substitution.
    pub fn search_candidates_gpu_batch(
        &self,
        slot: SlotId,
        queries: &[SlotVector],
        k: usize,
    ) -> CliResult<Vec<Vec<IndexSearchHit>>> {
        #[cfg(not(feature = "cuda"))]
        {
            let _ = (slot, queries, k);
            Err(weave_candidate_cuda_feature_missing())
        }
        #[cfg(feature = "cuda")]
        {
            if queries.is_empty() {
                return Ok(Vec::new());
            }
            let entry = self.require_entry(slot)?;
            match queries.first().expect("non-empty query batch") {
                SlotVector::Dense { .. } => dense::search_gpu_forced_batch(
                    &self.vault_dir,
                    entry,
                    slot,
                    queries,
                    k,
                    &self.dense_cache,
                ),
                SlotVector::Multi { .. } | SlotVector::SegmentedMulti { .. } => {
                    multi::search_gpu_forced_batch(
                        &self.vault_dir,
                        entry,
                        self.manifest.base_seq,
                        slot,
                        queries,
                        k,
                        &self.multi_token_ann_cache,
                    )
                }
                SlotVector::Sparse { .. } => queries
                    .iter()
                    .map(|query| self.search_candidates_gpu(slot, query, k))
                    .collect(),
                SlotVector::Absent { .. } => Err(stale(format!(
                    "persistent GPU candidate search slot {slot} received an absent batch query vector; remeasure the active panel"
                ))),
            }
        }
    }

    pub fn filter_candidates(&self, filters: &QueryFilters) -> CliResult<Option<BTreeSet<CxId>>> {
        filter::candidates(
            &self.vault_dir,
            self.manifest.filter.as_ref(),
            self.manifest.base_seq,
            filters,
        )
    }

    pub fn exact_metadata_candidates(&self, query: &str) -> CliResult<BTreeSet<CxId>> {
        filter::exact_metadata_candidates(
            &self.vault_dir,
            self.manifest.filter.as_ref(),
            self.manifest.base_seq,
            query,
        )
    }

    pub fn max_len(&self) -> usize {
        self.max_len_for_slots(None)
    }

    pub fn base_seq(&self) -> u64 {
        self.manifest.base_seq
    }

    pub fn manifest_sha256(&self) -> &str {
        &self.manifest_sha256
    }

    pub fn max_len_for_slots(&self, allowed_slots: Option<&BTreeSet<SlotId>>) -> usize {
        self.manifest
            .slots
            .iter()
            .filter(|entry| {
                allowed_slots
                    .map(|allowed| allowed.contains(&SlotId::new(entry.slot)))
                    .unwrap_or(true)
            })
            .map(|entry| entry.len)
            .max()
            .unwrap_or(0)
    }

    pub fn ensure_search_bounded(&self) -> CliResult {
        self.ensure_search_bounded_for_slots(None)
    }

    pub(crate) fn pin_budget_preflight_for_slots(
        &self,
        allowed_slots: Option<&BTreeSet<SlotId>>,
    ) -> CliResult<pinned::PinBudgetPreflight> {
        let mut requirements = Vec::new();
        for entry in &self.manifest.slots {
            let slot = SlotId::new(entry.slot);
            if allowed_slots
                .map(|allowed| !allowed.contains(&slot))
                .unwrap_or(false)
            {
                continue;
            }
            let requirement = match entry.kind.as_str() {
                "flat_dense" => Some((
                    entry.slot,
                    "flat_dense",
                    fs::metadata(self.vault_dir.join(entry.require_index_rel(slot)?))?.len(),
                )),
                "sparse_inverted" => Some((
                    entry.slot,
                    "sparse_inverted",
                    fs::metadata(self.vault_dir.join(entry.require_index_rel(slot)?))?.len(),
                )),
                "multi_maxsim_segments" => Some((
                    entry.slot,
                    "multi_maxsim",
                    multi::predicted_pin_bytes(entry, entry.require_token_dim(slot)?)?,
                )),
                _ => None,
            };
            if let Some(requirement) = requirement {
                requirements.push(requirement);
            }
        }
        pinned::preflight(&self.vault_dir, &requirements)
    }

    pub fn ensure_search_bounded_for_slots(
        &self,
        allowed_slots: Option<&BTreeSet<SlotId>>,
    ) -> CliResult {
        let entries = self
            .manifest
            .slots
            .iter()
            .filter(|entry| {
                allowed_slots
                    .map(|allowed| allowed.contains(&SlotId::new(entry.slot)))
                    .unwrap_or(true)
                    && (entry.kind == "multi_maxsim" || entry.kind == "multi_maxsim_segments")
            })
            .collect::<Vec<_>>();
        let outcomes = entries
            .par_iter()
            .map(|entry| {
                multi::ensure_bounded_sidecar(&self.vault_dir, entry, SlotId::new(entry.slot))
            })
            .collect::<Vec<_>>();
        // Preserve manifest-order error reporting even though independent
        // generations validate and pin concurrently.
        for outcome in outcomes {
            outcome?;
        }
        Ok(())
    }

    fn require_entry(&self, slot: SlotId) -> CliResult<&SearchIndexEntry> {
        self.manifest
            .slots
            .iter()
            .find(|entry| entry.slot == slot.get())
            .ok_or_else(|| {
                stale(format!(
                    "persistent search manifest has no index for active slot {slot}; reingest or backfill the vault before search"
                ))
            })
    }
}

pub fn validate_rebuild_config() -> CliResult {
    rebuild_plan::validate_parallel_rebuild_config()
}

impl SearchIndexEntry {
    pub(super) fn dense(
        slot: SlotId,
        dim: u32,
        len: usize,
        base_seq: u64,
        graph_rel: String,
        id_map_rel: String,
    ) -> Self {
        Self {
            slot: slot.get(),
            kind: "diskann".to_string(),
            dim: Some(dim),
            token_dim: None,
            len,
            built_at_seq: base_seq,
            graph_rel: Some(graph_rel),
            id_map_rel: Some(id_map_rel),
            id_map_sha256: None,
            index_rel: None,
            sha256: None,
            token_count: None,
        }
    }

    pub(super) fn flat_dense(
        slot: SlotId,
        dim: u32,
        len: usize,
        base_seq: u64,
        index_rel: String,
        sha256: String,
    ) -> Self {
        Self {
            slot: slot.get(),
            kind: "flat_dense".to_string(),
            dim: Some(dim),
            token_dim: None,
            len,
            built_at_seq: base_seq,
            graph_rel: None,
            id_map_rel: None,
            id_map_sha256: None,
            index_rel: Some(index_rel),
            sha256: Some(sha256),
            token_count: None,
        }
    }

    pub(super) fn sparse(
        slot: SlotId,
        dim: u32,
        len: usize,
        base_seq: u64,
        index_rel: String,
        sha256: String,
    ) -> Self {
        Self {
            slot: slot.get(),
            kind: "sparse_inverted".to_string(),
            dim: Some(dim),
            token_dim: None,
            len,
            built_at_seq: base_seq,
            graph_rel: None,
            id_map_rel: None,
            id_map_sha256: None,
            index_rel: Some(index_rel),
            sha256: Some(sha256),
            token_count: None,
        }
    }

    #[cfg(test)]
    pub(super) fn multi(
        slot: SlotId,
        token_dim: u32,
        len: usize,
        token_count: usize,
        base_seq: u64,
        index_rel: String,
        sha256: String,
    ) -> Self {
        Self {
            slot: slot.get(),
            kind: "multi_maxsim".to_string(),
            dim: None,
            token_dim: Some(token_dim),
            len,
            built_at_seq: base_seq,
            graph_rel: None,
            id_map_rel: None,
            id_map_sha256: None,
            index_rel: Some(index_rel),
            sha256: Some(sha256),
            token_count: Some(token_count),
        }
    }

    pub(super) fn multi_segments(
        slot: SlotId,
        token_dim: u32,
        len: usize,
        token_count: usize,
        base_seq: u64,
        index_rel: String,
        sha256: String,
    ) -> Self {
        Self {
            slot: slot.get(),
            kind: "multi_maxsim_segments".to_string(),
            dim: None,
            token_dim: Some(token_dim),
            len,
            built_at_seq: base_seq,
            graph_rel: None,
            id_map_rel: None,
            id_map_sha256: None,
            index_rel: Some(index_rel),
            sha256: Some(sha256),
            token_count: Some(token_count),
        }
    }

    pub(super) fn with_token_ann(
        mut self,
        graph_rel: String,
        id_map_rel: String,
        id_map_sha256: String,
    ) -> Self {
        self.graph_rel = Some(graph_rel);
        self.id_map_rel = Some(id_map_rel);
        self.id_map_sha256 = Some(id_map_sha256);
        self
    }

    pub(super) fn require_kind(&self, expected: &str, slot: SlotId) -> CliResult {
        if self.kind == expected {
            return Ok(());
        }
        Err(stale(format!(
            "persistent slot {slot} index kind {} is not {expected}; rebuild the vault search indexes",
            self.kind
        )))
    }

    pub(super) fn require_dim(&self, slot: SlotId) -> CliResult<u32> {
        self.dim.ok_or_else(|| {
            stale(format!(
                "persistent slot {slot} manifest is missing dim; rebuild the vault search indexes"
            ))
        })
    }

    pub(super) fn require_token_dim(&self, slot: SlotId) -> CliResult<u32> {
        self.token_dim.ok_or_else(|| {
            stale(format!(
                "persistent slot {slot} manifest is missing token_dim; rebuild the vault search indexes"
            ))
        })
    }

    pub(super) fn require_token_count(&self, slot: SlotId) -> CliResult<usize> {
        self.token_count.ok_or_else(|| {
            stale(format!(
                "persistent slot {slot} manifest is missing token_count; rebuild the vault search indexes"
            ))
        })
    }

    pub(super) fn require_id_map_sha256(&self, slot: SlotId) -> CliResult<&str> {
        self.id_map_sha256.as_deref().ok_or_else(|| {
            stale(format!(
                "persistent slot {slot} manifest is missing id_map_sha256; rebuild the vault search indexes"
            ))
        })
    }

    pub(super) fn require_graph_rel(&self, slot: SlotId) -> CliResult<&str> {
        self.graph_rel.as_deref().ok_or_else(|| {
            stale(format!(
                "persistent slot {slot} manifest is missing graph path; rebuild the vault search indexes"
            ))
        })
    }

    pub(super) fn require_id_map_rel(&self, slot: SlotId) -> CliResult<&str> {
        self.id_map_rel.as_deref().ok_or_else(|| {
            stale(format!(
                "persistent slot {slot} manifest is missing id map path; rebuild the vault search indexes"
            ))
        })
    }

    pub(super) fn require_index_rel(&self, slot: SlotId) -> CliResult<&str> {
        self.index_rel.as_deref().ok_or_else(|| {
            stale(format!(
                "persistent slot {slot} manifest is missing sidecar path; rebuild the vault search indexes"
            ))
        })
    }

    pub(super) fn require_sha256(&self, slot: SlotId) -> CliResult<&str> {
        self.sha256.as_deref().ok_or_else(|| {
            stale(format!(
                "persistent slot {slot} manifest is missing sidecar sha256; rebuild the vault search indexes"
            ))
        })
    }
}

#[path = "persisted/io.rs"]
mod fs_io;
use fs_io::{
    manifest_path, rel, sha256_hex, stale, write_atomic_hashed, write_json_atomic,
    write_json_atomic_durable, write_json_atomic_hashed,
};
