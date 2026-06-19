use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use calyx_core::{Result, SlotId};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::index::{DiskAnnSearch, DiskAnnSearchParams, SpannCentroidIndex, open_diskann_graph};

use super::assignment::read_ids;
use super::{
    CENTROID_DIR, MANIFEST_FILE, PartitionDistanceMetric, PartitionedManifest, RegionMeta, cx,
};

/// Region-restricted searcher over a partitioned vault. Holds centroids in RAM
/// and lazily mmaps region graphs on demand (only probed regions are resident).
pub struct PartitionedSearch {
    root: PathBuf,
    dim: usize,
    manifest: PartitionedManifest,
    centroids: SpannCentroidIndex,
    region_meta: BTreeMap<u32, RegionMeta>,
    cache: Mutex<BTreeMap<u32, RegionHandle>>,
}

/// A reference-counted, opened region graph plus its local->global id map. Cloned
/// out of the cache so probed regions can be searched in parallel without the lock.
type RegionHandle = Arc<(DiskAnnSearch, Vec<u64>)>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PartitionedSearchReadback {
    pub hits: Vec<(u64, f32)>,
    pub touched_regions: Vec<u32>,
}

impl PartitionedSearch {
    pub fn open(root: &Path) -> Result<Self> {
        let bytes = std::fs::read(root.join(MANIFEST_FILE)).map_err(|e| {
            crate::error::sextant_error(crate::error::CALYX_INDEX_IO, e.to_string())
        })?;
        let manifest: PartitionedManifest = serde_json::from_slice(&bytes).map_err(|e| {
            crate::error::sextant_error(crate::error::CALYX_INDEX_IO, e.to_string())
        })?;
        validate_manifest_artifacts(root, &manifest)?;
        let centroids = SpannCentroidIndex::open(root.join(CENTROID_DIR))?;
        let region_meta = manifest.regions.iter().map(|m| (m.id, m.clone())).collect();
        Ok(Self {
            root: root.to_path_buf(),
            dim: manifest.dim,
            region_meta,
            centroids,
            manifest,
            cache: Mutex::new(BTreeMap::new()),
        })
    }

    pub fn manifest(&self) -> &PartitionedManifest {
        &self.manifest
    }

    /// Number of region graphs touched by a query is at most `n_probe` — the
    /// proof that search cost scales with region size, not N.
    pub fn search(
        &self,
        query: &[f32],
        k: usize,
        n_probe: usize,
        region_beam: usize,
    ) -> Result<Vec<(u64, f32)>> {
        Ok(self
            .search_with_readback(query, k, n_probe, region_beam)?
            .hits)
    }

    pub fn search_with_readback(
        &self,
        query: &[f32],
        k: usize,
        n_probe: usize,
        region_beam: usize,
    ) -> Result<PartitionedSearchReadback> {
        if k == 0 {
            return Ok(PartitionedSearchReadback {
                hits: Vec::new(),
                touched_regions: Vec::new(),
            });
        }
        let regions = match self.manifest.distance_metric {
            PartitionDistanceMetric::UnitL2 => {
                self.centroids.nearest_centroids(query, n_probe.max(1))
            }
            PartitionDistanceMetric::RawL2 => self
                .centroids
                .nearest_centroids_exact_l2(query, n_probe.max(1)),
        };
        let sp = DiskAnnSearchParams {
            beamwidth: region_beam.max(k),
            ef_search: region_beam.max(k),
            rescore_k: region_beam.max(k),
            rescore_from_raw: false,
        };
        // Open (or fetch from cache) every probed region's graph under the lock,
        // cloning out reference-counted handles so the actual graph searches run
        // WITHOUT holding the cache lock — and in parallel (the probed regions are
        // independent, so per-query latency tracks the slowest single region, not
        // their sum). This is the main lever that brings p99 under the SLO.
        let mut handles: Vec<RegionHandle> = Vec::with_capacity(regions.len());
        let mut touched_regions = Vec::with_capacity(regions.len());
        {
            let mut cache = self.cache.lock().expect("partitioned cache poisoned");
            for region in regions {
                let Some(meta) = self.region_meta.get(&region) else {
                    continue;
                };
                touched_regions.push(region);
                if let std::collections::btree_map::Entry::Vacant(slot) = cache.entry(region) {
                    let ids = read_ids(&self.root.join(&meta.ids_rel))?;
                    let search = DiskAnnSearch::open(
                        SlotId::new(0),
                        self.root.join(&meta.graph_rel),
                        ids.iter().map(|&i| cx(i)).collect(),
                        None,
                        sp,
                    )?;
                    slot.insert(Arc::new((search, ids)));
                }
                handles.push(cache.get(&region).expect("just inserted").clone());
            }
        }
        let per_region: Vec<Vec<(u64, f32)>> = handles
            .par_iter()
            .map(|handle| -> Result<Vec<(u64, f32)>> {
                let (search, ids) = handle.as_ref();
                let mut local = Vec::with_capacity(k);
                for (pos, dist) in search.search_ids(query, k, &sp)? {
                    if let Some(&global) = ids.get(pos as usize) {
                        local.push((global, dist));
                    }
                }
                Ok(local)
            })
            .collect::<Result<Vec<_>>>()?;
        let mut hits: Vec<(u64, f32)> = per_region.into_iter().flatten().collect();
        hits.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        hits.dedup_by_key(|(id, _)| *id);
        hits.truncate(k);
        Ok(PartitionedSearchReadback {
            hits,
            touched_regions,
        })
    }

    pub fn dim(&self) -> usize {
        self.dim
    }
}

fn validate_manifest_artifacts(root: &Path, manifest: &PartitionedManifest) -> Result<()> {
    validate_graph(
        root,
        &manifest.root_graph_rel,
        manifest.dim,
        manifest.n_regions as u64,
        "root graph",
    )?;
    for meta in &manifest.regions {
        let label = format!("region {} graph", meta.id);
        validate_graph(
            root,
            &meta.graph_rel,
            manifest.dim,
            meta.count as u64,
            &label,
        )?;
        let ids = read_ids(&root.join(&meta.ids_rel))?;
        if ids.len() != meta.count {
            return Err(corrupt(format!(
                "region {} ids {} count {} != manifest count {}",
                meta.id,
                meta.ids_rel,
                ids.len(),
                meta.count
            )));
        }
    }
    Ok(())
}

fn validate_graph(
    root: &Path,
    rel: &str,
    expected_dim: usize,
    expected_nodes: u64,
    label: &str,
) -> Result<()> {
    let path = root.join(rel);
    let reader = open_diskann_graph(&path).map_err(|error| {
        crate::error::sextant_error(
            error.code,
            format!(
                "{label} {} failed validation: {}",
                path.display(),
                error.message
            ),
        )
    })?;
    let header = reader.header();
    if header.dim as usize != expected_dim {
        return Err(corrupt(format!(
            "{label} {rel} dim {} != manifest dim {expected_dim}",
            header.dim
        )));
    }
    if header.node_count != expected_nodes {
        return Err(corrupt(format!(
            "{label} {rel} node_count {} != manifest count {expected_nodes}",
            header.node_count
        )));
    }
    Ok(())
}

fn corrupt(detail: impl std::fmt::Display) -> calyx_core::CalyxError {
    crate::error::sextant_error(crate::error::CALYX_INDEX_CORRUPT, detail.to_string())
}
