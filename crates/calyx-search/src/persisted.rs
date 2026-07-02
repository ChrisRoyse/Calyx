#[path = "persisted/dense.rs"]
mod dense;
#[path = "persisted/filter.rs"]
mod filter;
#[path = "persisted/freshness.rs"]
mod freshness;
#[path = "persisted/marker.rs"]
pub mod marker;
#[cfg(test)]
#[path = "persisted/mixed_tests.rs"]
mod mixed_tests;
#[path = "persisted/multi.rs"]
mod multi;
#[path = "persisted/pinned.rs"]
mod pinned;
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

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use calyx_aster::vault::AsterVault;
use calyx_core::{CalyxError, Constellation, CxId, SlotId, SlotVector};
use calyx_sextant::QueryFilters;
use calyx_sextant::index::IndexSearchHit;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{CliError, CliResult};
pub use marker::{
    MarkerClearOutcome, REBUILD_REQUIRED_REMEDIATION, REBUILD_REQUIRED_SCHEMA,
    RebuildRequiredMarker, clear_rebuild_required_marker, clear_rebuild_required_marker_if_owned,
    read_rebuild_required_marker, rebuild_required_marker_path, write_rebuild_required_marker,
};
pub(crate) use pinned::canonical_vault_dir as canonical_pin_vault_dir;
pub(crate) use rebuild::load_docs_at;
#[cfg(test)]
use rebuild::rebuild_from_docs;
pub use rebuild::{
    RebuildProgress, load_docs, rebuild_for_vault, rebuild_for_vault_with_fallible_progress,
    rebuild_for_vault_with_progress,
};

const MANIFEST_FORMAT: &str = "calyx-search-index-manifest-v1";
const IDMAP_FORMAT: &str = "calyx-search-index-idmap-v1";
const INDEX_ROOT: &str = "idx/search";
const MANIFEST_NAME: &str = "manifest.json";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct SearchIndexManifest {
    format: String,
    base_seq: u64,
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
        })
    }

    pub fn search(
        &self,
        slot: SlotId,
        query: &SlotVector,
        k: usize,
    ) -> CliResult<Vec<IndexSearchHit>> {
        let entry = self.require_entry(slot)?;
        match query {
            SlotVector::Dense { .. } => dense::search(&self.vault_dir, entry, slot, query, k),
            SlotVector::Sparse { .. } => sparse::search(
                &self.vault_dir,
                entry,
                self.manifest.base_seq,
                slot,
                query,
                k,
                None,
            ),
            SlotVector::Multi { .. } => multi::search(
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
        match query {
            SlotVector::Dense { .. } => {
                dense::search_filtered(&self.vault_dir, entry, slot, query, k, candidates)
            }
            SlotVector::Sparse { .. } => sparse::search(
                &self.vault_dir,
                entry,
                self.manifest.base_seq,
                slot,
                query,
                k,
                Some(candidates),
            ),
            SlotVector::Multi { .. } => multi::search(
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

    pub fn filter_candidates(&self, filters: &QueryFilters) -> CliResult<Option<BTreeSet<CxId>>> {
        filter::candidates(
            &self.vault_dir,
            self.manifest.filter.as_ref(),
            self.manifest.base_seq,
            filters,
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

    pub fn ensure_search_bounded_for_slots(
        &self,
        allowed_slots: Option<&BTreeSet<SlotId>>,
    ) -> CliResult {
        for entry in &self.manifest.slots {
            if allowed_slots
                .map(|allowed| !allowed.contains(&SlotId::new(entry.slot)))
                .unwrap_or(false)
            {
                continue;
            }
            if entry.kind == "multi_maxsim" || entry.kind == "multi_maxsim_segments" {
                multi::ensure_bounded_sidecar(&self.vault_dir, entry, SlotId::new(entry.slot))?;
            }
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
            index_rel: Some(index_rel),
            sha256: Some(sha256),
            token_count: Some(token_count),
        }
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

fn manifest_path(vault_dir: &Path) -> PathBuf {
    vault_dir.join(INDEX_ROOT).join(MANIFEST_NAME)
}

#[path = "persisted/io.rs"]
mod fs_io;
use fs_io::{
    rel, sha256_hex, stale, write_atomic_hashed, write_json_atomic, write_json_atomic_durable,
    write_json_atomic_hashed,
};
