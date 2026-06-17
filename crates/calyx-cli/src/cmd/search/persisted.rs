#[path = "persisted/dense.rs"]
mod dense;
#[path = "persisted/filter.rs"]
mod filter;
#[cfg(test)]
#[path = "persisted/mixed_tests.rs"]
mod mixed_tests;
#[path = "persisted/multi.rs"]
mod multi;
#[path = "persisted/sparse.rs"]
mod sparse;
#[cfg(test)]
#[path = "persisted/tests.rs"]
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::AsterVault;
use calyx_core::{CalyxError, Constellation, CxId, SlotId, SlotVector, VaultStore};
use calyx_sextant::QueryFilters;
use calyx_sextant::index::IndexSearchHit;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{CliError, CliResult};

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
pub(crate) struct PersistedSearchIndexes {
    vault_dir: PathBuf,
    manifest: SearchIndexManifest,
}

impl PersistedSearchIndexes {
    pub(crate) fn open(vault_dir: &Path) -> CliResult<Self> {
        let manifest_path = manifest_path(vault_dir);
        if !manifest_path.is_file() {
            return Err(stale(format!(
                "persistent search index manifest missing at {}; ingest or rebuild the vault before search",
                manifest_path.display()
            )));
        }
        let manifest: SearchIndexManifest = serde_json::from_slice(&fs::read(&manifest_path)?)?;
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
        })
    }

    pub(crate) fn search(
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

    pub(crate) fn search_filtered(
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

    pub(crate) fn filter_candidates(
        &self,
        filters: &QueryFilters,
    ) -> CliResult<Option<BTreeSet<CxId>>> {
        filter::candidates(
            &self.vault_dir,
            self.manifest.filter.as_ref(),
            self.manifest.base_seq,
            filters,
        )
    }

    pub(crate) fn max_len(&self) -> usize {
        self.manifest
            .slots
            .iter()
            .map(|entry| entry.len)
            .max()
            .unwrap_or(0)
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

pub(crate) fn rebuild_for_vault(vault_dir: &Path, vault: &AsterVault) -> CliResult {
    let docs = load_docs(vault)?;
    let summary = rebuild_from_docs(vault_dir, &docs, vault.latest_seq())?;
    let _ = (summary.slots, summary.total_rows, &summary.manifest_path);
    Ok(())
}

fn rebuild_from_docs(
    vault_dir: &Path,
    docs: &BTreeMap<CxId, Constellation>,
    base_seq: u64,
) -> CliResult<RebuildSummary> {
    let root = vault_dir.join(INDEX_ROOT);
    fs::create_dir_all(&root)?;
    let mut entries = Vec::new();
    let mut total_rows = 0usize;
    for (slot, rows) in dense::collect(docs)? {
        total_rows += rows.len();
        entries.push(dense::write(vault_dir, &root, slot, rows, base_seq)?);
    }
    for (slot, rows) in sparse::collect(docs)? {
        total_rows += rows.len();
        entries.push(sparse::write(vault_dir, &root, slot, rows, base_seq)?);
    }
    for (slot, rows) in multi::collect(docs)? {
        total_rows += rows.len();
        entries.push(multi::write(vault_dir, &root, slot, rows, base_seq)?);
    }
    entries.sort_by_key(|entry| entry.slot);
    let manifest = SearchIndexManifest {
        format: MANIFEST_FORMAT.to_string(),
        base_seq,
        filter: Some(filter::write(vault_dir, &root, docs, base_seq)?),
        slots: entries,
    };
    let manifest_path = manifest_path(vault_dir);
    write_json_atomic(&manifest_path, &manifest)?;
    Ok(RebuildSummary {
        slots: manifest.slots.len(),
        total_rows,
        manifest_path,
    })
}

pub(crate) fn load_docs(vault: &AsterVault) -> CliResult<BTreeMap<CxId, Constellation>> {
    let snapshot = vault.snapshot();
    let mut docs = BTreeMap::new();
    for (key, _) in vault.scan_cf_at(snapshot, ColumnFamily::Base)? {
        let bytes: [u8; 16] = key.as_slice().try_into().map_err(|_| {
            CalyxError::vault_access_denied(format!("base CF key has {} bytes", key.len()))
        })?;
        let cx_id = CxId::from_bytes(bytes);
        docs.insert(cx_id, vault.get(cx_id, snapshot)?);
    }
    Ok(docs)
}

fn manifest_path(vault_dir: &Path) -> PathBuf {
    vault_dir.join(INDEX_ROOT).join(MANIFEST_NAME)
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> CliResult {
    write_bytes_atomic(path, &serde_json::to_vec_pretty(value)?)
}

fn write_bytes_atomic(path: &Path, bytes: &[u8]) -> CliResult {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    {
        let mut file = File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })?;
    Ok(())
}

fn rel(root: &Path, path: &Path) -> CliResult<String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|err| CliError::usage(format!("index path is outside vault root: {err}")))?;
    Ok(relative.to_string_lossy().replace('\\', "/"))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn stale(message: impl Into<String>) -> CliError {
    CalyxError::stale_derived(message).into()
}
