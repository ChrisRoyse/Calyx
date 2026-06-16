//! PH68 T06 — memory-bounded **partitioned** billion-scale vault (#550; fixes
//! #702/#703, sidesteps #701).
//!
//! The flat in-memory Vamana builder cannot reach 1e8 (it materializes the whole
//! dataset ~600 GB and the build is super-linear). This module builds a real
//! billion-scale vault whose build memory AND query cost scale with *region size*,
//! not N:
//!
//! 1. **Centroids from a sample** — `build_centroids` (k-means++) on a deterministic
//!    sample yields `R` region centroids (the routing layer; saved as
//!    `idx/slot_00.sparse/centroids.spn`).
//! 2. **Stream-assign** — every cx is generated in chunks (never all at once),
//!    assigned to its nearest centroid, and bucketed by region.
//! 3. **Per-region DiskANN graphs** — each region (<= region_cap rows, fits RAM) is
//!    regenerated and built into its own `idx/region_NNNNN.ann/graph.cda` via the
//!    existing (correct, query-distance) DiskANN builder.
//! 4. **Region-restricted search** — a query routes to its nearest `n_probe`
//!    regions via the centroid HNSW and searches ONLY those region graphs (each
//!    small + mmap'd), then merges. No full-graph scan, no post-filter, no SPANN
//!    static-score rerank.
//!
//! Row generation is per-index deterministic (`gen_row`) so build and search never
//! hold more than one region's vectors at a time.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use calyx_core::{CxId, Result, SlotId};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::index::{
    DiskAnnBuildParams, DiskAnnSearch, DiskAnnSearchParams, SpannCentroidIndex, build_centroids,
};

const MANIFEST_FILE: &str = "partitioned-manifest.json";
const CENTROID_DIR: &str = "idx/slot_00.sparse";
const ROOT_GRAPH: &str = "idx/slot_00.ann/graph.cda";
/// Mixing constant for per-index RNG seeding (splitmix64 multiplier).
const IDX_MIX: u64 = 0x9E37_79B9_7F4A_7C15;

/// Deterministic, per-index row generation. Independent of any other index, so
/// rows can be streamed/regenerated per region without materializing `0..idx`.
/// Dense-with-spike structure (cluster by `idx % dim`), unit-normalized.
pub fn gen_row(seed: u64, idx: u64, dim: usize) -> Vec<f32> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed ^ idx.wrapping_mul(IDX_MIX));
    let mut v: Vec<f32> = (0..dim)
        .map(|j| rng.gen_range(-1.0_f32..1.0) + ((idx as usize + j) % dim) as f32 * 0.001)
        .collect();
    let spike = (idx as usize) % dim;
    v[spike] += 4.0;
    normalize(&mut v);
    v
}

fn normalize(v: &mut [f32]) {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in v {
            *x /= norm;
        }
    }
}

/// `CxId` carrying a dense `u64` index in its low 8 bytes.
pub fn cx(idx: u64) -> CxId {
    let mut bytes = [0u8; 16];
    bytes[8..16].copy_from_slice(&idx.to_be_bytes());
    CxId::from_bytes(bytes)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegionMeta {
    pub id: u32,
    pub count: usize,
    pub graph_rel: String,
    pub ids_rel: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartitionedManifest {
    pub format: String,
    pub n_cx: u64,
    pub dim: usize,
    pub n_regions: usize,
    pub seed: u64,
    pub m_max: usize,
    pub ef_construction: usize,
    pub centroids_rel: String,
    pub root_graph_rel: String,
    pub regions: Vec<RegionMeta>,
}

/// Parameters for a partitioned build.
#[derive(Debug, Clone, Copy)]
pub struct PartitionBuildParams {
    pub n_cx: u64,
    pub dim: usize,
    pub n_regions: usize,
    pub seed: u64,
    /// Sample size for centroid k-means (<= n_cx).
    pub sample: usize,
    /// Streaming assignment chunk size (rows generated per batch).
    pub chunk: usize,
    pub m_max: usize,
    pub ef_construction: usize,
}

impl PartitionBuildParams {
    pub fn new(n_cx: u64, dim: usize, n_regions: usize, seed: u64) -> Self {
        Self {
            n_cx,
            dim,
            n_regions,
            seed,
            sample: (n_cx as usize).min(200_000),
            chunk: 100_000,
            m_max: 32,
            ef_construction: 96,
        }
    }
}

fn graph_rel(region: u32) -> String {
    format!("idx/region_{region:05}.ann/graph.cda")
}
fn ids_rel(region: u32) -> String {
    format!("idx/region_{region:05}.ids")
}

/// Build the partitioned vault under `root`. Memory-bounded: never holds more
/// than `chunk` rows (assignment) or one region's rows (graph build).
pub fn build_partitioned_vault(
    root: &Path,
    p: PartitionBuildParams,
) -> Result<PartitionedManifest> {
    if p.n_cx == 0 || p.dim == 0 || p.n_regions == 0 {
        return Err(crate::error::sextant_error(
            crate::error::CALYX_INDEX_INVALID_PARAMS,
            "partitioned vault requires nonzero n_cx, dim, n_regions",
        ));
    }
    std::fs::create_dir_all(root.join(CENTROID_DIR))
        .map_err(|e| crate::error::sextant_error(crate::error::CALYX_INDEX_IO, e.to_string()))?;

    // 1. Centroids from a deterministic sample (stride over the index space).
    let stride = (p.n_cx / p.sample.max(1) as u64).max(1);
    let sample_rows: Vec<(u32, Vec<f32>)> = (0..p.sample)
        .into_par_iter()
        .map(|s| {
            let idx = (s as u64 * stride) % p.n_cx;
            (s as u32, gen_row(p.seed, idx, p.dim))
        })
        .collect();
    let centroids = build_centroids(&sample_rows, p.n_regions, p.seed);
    centroids.save(root.join(CENTROID_DIR))?;
    let r = centroids.centroid_count();

    // 2. Stream-assign every cx to its nearest centroid -> per-region buckets.
    let mut buckets: Vec<Vec<u64>> = vec![Vec::new(); r];
    let mut start = 0u64;
    while start < p.n_cx {
        let end = (start + p.chunk as u64).min(p.n_cx);
        // Route assignment through the centroid HNSW (O(log R)) — an exact scan
        // would be O(N*R) and quadratic in N once R grows with the dataset.
        let assigned: Vec<(u64, u32)> = (start..end)
            .into_par_iter()
            .map(|idx| (idx, centroids.assign_hnsw(&gen_row(p.seed, idx, p.dim))))
            .collect();
        for (idx, region) in assigned {
            buckets[region as usize].push(idx);
        }
        start = end;
    }

    // 3. Build one DiskANN graph per region (each fits RAM). Regions are built
    //    in PARALLEL across cores (#706): each region is small and serial row-gen
    //    + a small-graph Vamana build under-utilizes the machine, so the regions
    //    themselves are the unit of parallelism. Peak memory is bounded by
    //    `rayon` thread count x region size (NOT N), preserving the memory-bound
    //    guarantee. Each region writes to a distinct graph/ids path, so the
    //    concurrent builds never contend.
    let build_params = DiskAnnBuildParams {
        dim: p.dim,
        m_max: p.m_max,
        ef_construction: p.ef_construction,
        alpha: 1.2,
    };
    let search_params = DiskAnnSearchParams {
        beamwidth: 64,
        ef_search: 64,
        rescore_k: 64,
        rescore_from_raw: false,
    };
    let nonempty: Vec<(u32, &Vec<u64>)> = buckets
        .iter()
        .enumerate()
        .filter(|(_, members)| !members.is_empty())
        .map(|(region, members)| (region as u32, members))
        .collect();
    let mut regions: Vec<RegionMeta> = nonempty
        .par_iter()
        .map(|(region, members)| -> Result<RegionMeta> {
            let region = *region;
            // Serial row-gen here: the outer par_iter already saturates cores, so
            // nesting another parallel gen would only add scheduler overhead.
            let rows: Vec<(CxId, Vec<f32>)> = members
                .iter()
                .map(|&idx| (cx(idx), gen_row(p.seed, idx, p.dim)))
                .collect();
            let graph_path = root.join(graph_rel(region));
            DiskAnnSearch::build(
                SlotId::new(0),
                &graph_path,
                &rows,
                build_params,
                None,
                search_params,
            )?;
            write_ids(&root.join(ids_rel(region)), members)?;
            Ok(RegionMeta {
                id: region,
                count: members.len(),
                graph_rel: graph_rel(region),
                ids_rel: ids_rel(region),
            })
        })
        .collect::<Result<Vec<RegionMeta>>>()?;
    // `par_iter().collect()` preserves input order, but make the on-disk manifest
    // order explicit and deterministic regardless of scheduling.
    regions.sort_by_key(|m| m.id);

    // 4. Root DiskANN graph over the region centroids (card's slot_00.ann + a
    //    second routing path). Tiny (R nodes).
    let centroid_rows: Vec<(CxId, Vec<f32>)> = centroids
        .centroids()
        .iter()
        .enumerate()
        .map(|(i, c)| (cx(i as u64), c.clone()))
        .collect();
    DiskAnnSearch::build(
        SlotId::new(0),
        root.join(ROOT_GRAPH),
        &centroid_rows,
        build_params,
        None,
        search_params,
    )?;

    let manifest = PartitionedManifest {
        format: "calyx-partitioned-vault-v1".to_string(),
        n_cx: p.n_cx,
        dim: p.dim,
        n_regions: r,
        seed: p.seed,
        m_max: p.m_max,
        ef_construction: p.ef_construction,
        centroids_rel: format!("{CENTROID_DIR}/centroids.spn"),
        root_graph_rel: ROOT_GRAPH.to_string(),
        regions,
    };
    let bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|e| crate::error::sextant_error(crate::error::CALYX_INDEX_IO, e.to_string()))?;
    std::fs::write(root.join(MANIFEST_FILE), bytes)
        .map_err(|e| crate::error::sextant_error(crate::error::CALYX_INDEX_IO, e.to_string()))?;
    Ok(manifest)
}

fn write_ids(path: &Path, ids: &[u64]) -> Result<()> {
    let mut bytes = Vec::with_capacity(ids.len() * 8);
    for id in ids {
        bytes.extend_from_slice(&id.to_le_bytes());
    }
    std::fs::write(path, bytes)
        .map_err(|e| crate::error::sextant_error(crate::error::CALYX_INDEX_IO, e.to_string()))
}

fn read_ids(path: &Path) -> Result<Vec<u64>> {
    let bytes = std::fs::read(path)
        .map_err(|e| crate::error::sextant_error(crate::error::CALYX_INDEX_IO, e.to_string()))?;
    Ok(bytes
        .chunks_exact(8)
        .map(|c| u64::from_le_bytes(c.try_into().expect("8 bytes")))
        .collect())
}

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

impl PartitionedSearch {
    pub fn open(root: &Path) -> Result<Self> {
        let bytes = std::fs::read(root.join(MANIFEST_FILE)).map_err(|e| {
            crate::error::sextant_error(crate::error::CALYX_INDEX_IO, e.to_string())
        })?;
        let manifest: PartitionedManifest = serde_json::from_slice(&bytes).map_err(|e| {
            crate::error::sextant_error(crate::error::CALYX_INDEX_IO, e.to_string())
        })?;
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
        if k == 0 {
            return Ok(Vec::new());
        }
        let regions = self.centroids.nearest_centroids(query, n_probe.max(1));
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
        {
            let mut cache = self.cache.lock().expect("partitioned cache poisoned");
            for region in regions {
                let Some(meta) = self.region_meta.get(&region) else {
                    continue;
                };
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
        Ok(hits)
    }

    pub fn dim(&self) -> usize {
        self.dim
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gen_row_is_deterministic_and_normalized() {
        let a = gen_row(42, 12345, 64);
        let b = gen_row(42, 12345, 64);
        assert_eq!(a, b, "same (seed,idx) -> same row");
        let norm = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5, "unit norm, got {norm}");
        assert_ne!(gen_row(42, 1, 64), gen_row(42, 2, 64));
    }

    #[test]
    fn partitioned_self_recall_and_region_restriction() {
        let dir = std::env::temp_dir().join(format!("calyx-part-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let p = PartitionBuildParams {
            n_cx: 5_000,
            dim: 32,
            n_regions: 16,
            seed: 7,
            sample: 5_000,
            chunk: 1_000,
            m_max: 16,
            ef_construction: 64,
        };
        let manifest = build_partitioned_vault(&dir, p).expect("build");
        // Every cx landed in exactly one region.
        let total: usize = manifest.regions.iter().map(|r| r.count).sum();
        assert_eq!(total, 5_000, "all cx partitioned exactly once");

        let search = PartitionedSearch::open(&dir).expect("open");
        // Self-recall@10: a row's own cx must be in its top-10 (probe a few regions).
        let mut hits = 0;
        let n = 200;
        for s in 0..n {
            let idx = (s as u64 * 23) % p.n_cx;
            let q = gen_row(p.seed, idx, p.dim);
            let res = search.search(&q, 10, 4, 64).expect("search");
            if res.iter().any(|(c, _)| *c == idx) {
                hits += 1;
            }
        }
        let recall = hits as f32 / n as f32;
        assert!(recall >= 0.85, "self-recall@10 {recall} < 0.85");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
