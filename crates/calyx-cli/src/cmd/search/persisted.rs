#[path = "persisted/filter.rs"]
mod filter;
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
use calyx_sextant::index::{
    DiskAnnBuildParams, DiskAnnSearch, DiskAnnSearchParams, IndexSearchHit, SextantIndex, ranked,
};
use serde::{Deserialize, Serialize};

use crate::error::{CliError, CliResult};

const MANIFEST_FORMAT: &str = "calyx-search-index-manifest-v1";
const IDMAP_FORMAT: &str = "calyx-search-index-idmap-v1";
const INDEX_ROOT: &str = "idx/search";
const MANIFEST_NAME: &str = "manifest.json";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct SearchIndexManifest {
    format: String,
    base_seq: u64,
    #[serde(default)]
    filter: Option<FilterIndexEntry>,
    slots: Vec<SearchIndexEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SearchIndexEntry {
    slot: u16,
    kind: String,
    dim: u32,
    len: usize,
    built_at_seq: u64,
    graph_rel: String,
    id_map_rel: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SlotIdMap {
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
pub(super) struct PersistedSearchIndexes {
    vault_dir: PathBuf,
    manifest: SearchIndexManifest,
}

struct DenseSlotRows {
    dim: u32,
    rows: Vec<(CxId, Vec<f32>)>,
}

impl PersistedSearchIndexes {
    pub(super) fn open(vault_dir: &Path) -> CliResult<Self> {
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

    pub(super) fn search(
        &self,
        slot: SlotId,
        query: &SlotVector,
        k: usize,
    ) -> CliResult<Vec<IndexSearchHit>> {
        let SlotVector::Dense { dim, .. } = query else {
            return Err(stale(format!(
                "persistent search currently supports dense ANN slots only; slot {slot} query is not dense"
            )));
        };
        let Some(entry) = self.entry(slot) else {
            return Ok(Vec::new());
        };
        if entry.kind != "diskann" {
            return Err(stale(format!(
                "persistent slot {slot} index kind {} is not diskann",
                entry.kind
            )));
        }
        if entry.dim != *dim {
            return Err(stale(format!(
                "persistent slot {slot} index dim {} != query dim {dim}; reingest/backfill the vault",
                entry.dim
            )));
        }
        let ids = self.read_ids(entry)?;
        if ids.len() != entry.len {
            return Err(stale(format!(
                "persistent slot {slot} id map len {} != manifest len {}",
                ids.len(),
                entry.len
            )));
        }
        let graph = self.vault_dir.join(&entry.graph_rel);
        let mut index = DiskAnnSearch::open(slot, graph, ids, None, search_params(k.max(64)))?;
        index.set_base_seq(entry.base_seq());
        let want = k.max(1).min(entry.len);
        index
            .search(query, want, Some(want.max(64)))
            .map_err(Into::into)
    }

    pub(super) fn search_filtered(
        &self,
        slot: SlotId,
        query: &SlotVector,
        k: usize,
        candidates: &BTreeSet<CxId>,
    ) -> CliResult<Vec<IndexSearchHit>> {
        if candidates.is_empty() {
            return Ok(Vec::new());
        }
        let SlotVector::Dense { dim, data } = query else {
            return Err(stale(format!(
                "persistent filtered search currently supports dense ANN slots only; slot {slot} query is not dense"
            )));
        };
        let Some(entry) = self.entry(slot) else {
            return Ok(Vec::new());
        };
        if entry.kind != "diskann" {
            return Err(stale(format!(
                "persistent slot {slot} index kind {} is not diskann",
                entry.kind
            )));
        }
        if entry.dim != *dim {
            return Err(stale(format!(
                "persistent slot {slot} index dim {} != query dim {dim}; reingest/backfill the vault",
                entry.dim
            )));
        }
        let ids = self.read_ids(entry)?;
        if ids.len() != entry.len {
            return Err(stale(format!(
                "persistent slot {slot} id map len {} != manifest len {}",
                ids.len(),
                entry.len
            )));
        }
        let graph = self.vault_dir.join(&entry.graph_rel);
        let mut index = DiskAnnSearch::open(slot, graph, ids, None, search_params(k.max(64)))?;
        index.set_base_seq(entry.base_seq());
        exact_filtered_hits(&index, data, k, candidates)
    }

    pub(super) fn filter_candidates(
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

    pub(super) fn max_len(&self) -> usize {
        self.manifest
            .slots
            .iter()
            .map(|entry| entry.len)
            .max()
            .unwrap_or(0)
    }

    fn entry(&self, slot: SlotId) -> Option<&SearchIndexEntry> {
        self.manifest
            .slots
            .iter()
            .find(|entry| entry.slot == slot.get())
    }

    fn read_ids(&self, entry: &SearchIndexEntry) -> CliResult<Vec<CxId>> {
        let path = self.vault_dir.join(&entry.id_map_rel);
        let map: SlotIdMap = serde_json::from_slice(&fs::read(&path)?)?;
        if map.format != IDMAP_FORMAT {
            return Err(stale(format!(
                "persistent slot {} id map {} has format {}; expected {IDMAP_FORMAT}",
                entry.slot,
                path.display(),
                map.format
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
}

impl SearchIndexEntry {
    fn base_seq(&self) -> u64 {
        self.built_at_seq
    }
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

pub(super) fn rebuild_for_vault(vault_dir: &Path, vault: &AsterVault) -> CliResult {
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
    for (slot, rows) in collect_dense_slots(docs)? {
        total_rows += rows.rows.len();
        entries.push(write_dense_slot(vault_dir, &root, slot, rows, base_seq)?);
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

fn collect_dense_slots(
    docs: &BTreeMap<CxId, Constellation>,
) -> CliResult<BTreeMap<SlotId, DenseSlotRows>> {
    let mut out = BTreeMap::<SlotId, DenseSlotRows>::new();
    for cx in docs.values() {
        for (slot, vector) in &cx.slots {
            let SlotVector::Dense { dim, data } = vector else {
                continue;
            };
            validate_dense(*slot, cx.cx_id, *dim, data)?;
            let entry = out.entry(*slot).or_insert_with(|| DenseSlotRows {
                dim: *dim,
                rows: Vec::new(),
            });
            if entry.dim != *dim {
                return Err(stale(format!(
                    "slot {slot} has mixed dense dims: {} and {dim}",
                    entry.dim
                )));
            }
            entry.rows.push((cx.cx_id, data.clone()));
        }
    }
    Ok(out)
}

fn validate_dense(slot: SlotId, cx_id: CxId, dim: u32, data: &[f32]) -> CliResult {
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

fn write_dense_slot(
    vault_dir: &Path,
    root: &Path,
    slot: SlotId,
    rows: DenseSlotRows,
    base_seq: u64,
) -> CliResult<SearchIndexEntry> {
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
    fs::create_dir_all(&dir)?;
    let graph_path = dir.join("graph.cda");
    DiskAnnSearch::build(
        slot,
        &graph_path,
        &rows.rows,
        build_params(rows.dim as usize),
        None,
        search_params(rows.rows.len().max(64)),
    )?;
    let id_map_path = dir.join("ids.json");
    write_json_atomic(
        &id_map_path,
        &SlotIdMap {
            format: IDMAP_FORMAT.to_string(),
            slot: slot.get(),
            ids: rows.rows.iter().map(|(cx_id, _)| *cx_id).collect(),
        },
    )?;
    Ok(SearchIndexEntry {
        slot: slot.get(),
        kind: "diskann".to_string(),
        dim: rows.dim,
        len: rows.rows.len(),
        built_at_seq: base_seq,
        graph_rel: rel(vault_dir, &graph_path)?,
        id_map_rel: rel(vault_dir, &id_map_path)?,
    })
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

pub(super) fn load_docs(vault: &AsterVault) -> CliResult<BTreeMap<CxId, Constellation>> {
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
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(value)?;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    {
        let mut file = File::create(&tmp)?;
        file.write_all(&bytes)?;
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

fn stale(message: impl Into<String>) -> CliError {
    CalyxError::stale_derived(message).into()
}
