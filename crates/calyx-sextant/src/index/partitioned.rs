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

mod balance;
mod search;

use std::path::Path;

use calyx_core::{CxId, Result, SlotId};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::index::{
    DiskAnnBuildParams, DiskAnnSearch, DiskAnnSearchParams, SpannCentroidIndex, build_centroids,
};
use balance::balance_regions;
pub use search::PartitionedSearch;

const MANIFEST_FILE: &str = "partitioned-manifest.json";
const CENTROID_DIR: &str = "idx/slot_00.sparse";
const ROOT_GRAPH: &str = "idx/slot_00.ann/graph.cda";
/// Mixing constant for per-index RNG seeding (splitmix64 multiplier).
const IDX_MIX: u64 = 0x9E37_79B9_7F4A_7C15;
/// Floor for the per-region size cap used by region balancing (#713); regions are
/// never split below this even when the mean region size is tiny.
const MIN_REGION_CAP: usize = 2_048;

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

/// Stream every cx through `centroids`, bucketing each by the SAME nearest-centroid
/// routing a query uses at search time (`assign_hnsw`, i.e. `nearest_centroids`
/// top-1). Using the identical routing path for assignment and search keeps a point's
/// region equal to the region its own query probes (#711). Memory-bounded: vectors
/// are pulled from the source in `chunk`-sized batches, never all at once.
fn stream_assign(
    centroids: &SpannCentroidIndex,
    source: &dyn VectorSource,
    chunk: usize,
) -> Vec<Vec<u64>> {
    let r = centroids.centroid_count();
    let n = source.len();
    let chunk = chunk.max(1) as u64;
    let mut buckets: Vec<Vec<u64>> = vec![Vec::new(); r];
    let mut start = 0u64;
    while start < n {
        let end = (start + chunk).min(n);
        let assigned: Vec<(u64, u32)> = (start..end)
            .into_par_iter()
            .map(|idx| (idx, centroids.assign_hnsw(&source.row(idx))))
            .collect();
        for (idx, region) in assigned {
            if let Some(bucket) = buckets.get_mut(region as usize) {
                bucket.push(idx);
            }
        }
        start = end;
    }
    buckets
}

/// Source of the vectors a partitioned vault is built from. The real, production
/// path reads genuine embeddings from a `.fbin` produced by the real embedder
/// ([`FbinSource`]). [`SyntheticSource`] exists ONLY for builder-logic unit tests
/// (does every cx land in one region? does balancing hold the cap?) and must NEVER
/// back a recall or FSV claim — recall is meaningless on fabricated geometry.
pub trait VectorSource: Sync {
    fn dim(&self) -> usize;
    fn len(&self) -> u64;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// The embedding of row `idx` (`0..len`).
    fn row(&self, idx: u64) -> Vec<f32>;
}

/// Real embeddings memory-mapped from a `.fbin` on disk — the billion-scale build
/// path. No vectors are synthesised.
pub struct FbinSource {
    vectors: crate::index::vecfile::FbinVectors,
}

impl FbinSource {
    pub fn open(path: &Path) -> Result<Self> {
        Ok(Self {
            vectors: crate::index::vecfile::FbinVectors::open(path)?,
        })
    }
}

impl VectorSource for FbinSource {
    fn dim(&self) -> usize {
        self.vectors.dim()
    }
    fn len(&self) -> u64 {
        self.vectors.count()
    }
    fn row(&self, idx: u64) -> Vec<f32> {
        self.vectors.row(idx).to_vec()
    }
}

/// Deterministic synthetic rows. Builder-logic unit tests ONLY — never validation.
pub struct SyntheticSource {
    pub seed: u64,
    pub dim: usize,
    pub n_cx: u64,
}

impl VectorSource for SyntheticSource {
    fn dim(&self) -> usize {
        self.dim
    }
    fn len(&self) -> u64 {
        self.n_cx
    }
    fn row(&self, idx: u64) -> Vec<f32> {
        gen_row(self.seed, idx, self.dim)
    }
}

/// Build a partitioned vault from a deterministic synthetic source. Unit-test /
/// builder-logic helper only — for real validation use
/// [`build_partitioned_vault_from_source`] with an [`FbinSource`].
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
    let source = SyntheticSource {
        seed: p.seed,
        dim: p.dim,
        n_cx: p.n_cx,
    };
    build_partitioned_vault_from_source(root, &source, p)
}

/// Build the partitioned vault under `root` from REAL vectors in `source`.
/// Memory-bounded: never holds more than `chunk` rows (assignment) or one region's
/// rows (graph build). `n_cx` and `dim` come from the source (the file is the source
/// of truth); `p.n_cx`/`p.dim` are ignored.
pub fn build_partitioned_vault_from_source(
    root: &Path,
    source: &dyn VectorSource,
    p: PartitionBuildParams,
) -> Result<PartitionedManifest> {
    let dim = source.dim();
    let n_cx = source.len();
    if n_cx == 0 || dim == 0 || p.n_regions == 0 {
        return Err(crate::error::sextant_error(
            crate::error::CALYX_INDEX_INVALID_PARAMS,
            "partitioned vault requires nonzero source len, dim, n_regions",
        ));
    }
    std::fs::create_dir_all(root.join(CENTROID_DIR))
        .map_err(|e| crate::error::sextant_error(crate::error::CALYX_INDEX_IO, e.to_string()))?;

    // 1. Centroids from a deterministic sample (stride over the index space).
    let sample = p.sample.min(n_cx as usize).max(1);
    let stride = (n_cx / sample as u64).max(1);
    let sample_rows: Vec<(u32, Vec<f32>)> = (0..sample)
        .into_par_iter()
        .map(|s| {
            let idx = (s as u64 * stride) % n_cx;
            (s as u32, source.row(idx))
        })
        .collect();
    let centroids = build_centroids(&sample_rows, p.n_regions, p.seed);
    let r = centroids.centroid_count();

    // 2. Stream-assign every cx to its nearest centroid -> per-region buckets.
    //    Pick the assignment method by centroid count: an exact flat scan is
    //    O(R) per point but cache-friendly/branch-free and wins for moderate R;
    //    once R grows the scan's O(N*R) becomes quadratic in N AND, at dim 512,
    //    memory-bandwidth-bound (the centroid table spills L2), so route through
    //    the centroid HNSW (O(log R)) instead. Measured: HNSW already wins by
    //    R~2500 at dim 512; keep flat only for trivially small centroid sets.
    const HNSW_ASSIGN_MIN_CENTROIDS: usize = 256;
    let use_hnsw_assign = r > HNSW_ASSIGN_MIN_CENTROIDS;
    let mut buckets: Vec<Vec<u64>> = vec![Vec::new(); r];
    let mut start = 0u64;
    while start < n_cx {
        let end = (start + p.chunk as u64).min(n_cx);
        let assigned: Vec<(u64, u32)> = (start..end)
            .into_par_iter()
            .map(|idx| {
                let row = source.row(idx);
                let region = if use_hnsw_assign {
                    centroids.assign_hnsw(&row)
                } else {
                    centroids.assign(&row)
                };
                (idx, region)
            })
            .collect();
        for (idx, region) in assigned {
            buckets[region as usize].push(idx);
        }
        start = end;
    }

    // 2b. Balance region sizes (#713). Nearest-centroid assignment is right-skewed,
    //     and a few oversized regions dominate both the (super-linear) build tail
    //     AND per-region search cost. Split any region above `cap` into sub-regions
    //     via local k-means, then rebuild the routing layer over the FINAL centroid
    //     set so search still routes correctly. cap = target mean: the recursive
    //     splitter enforces this hard bound, keeping final max/mean near 1-2x.
    let mean_region = (n_cx as usize).div_ceil(r.max(1));
    let cap = mean_region.max(MIN_REGION_CAP);
    let (final_centroids, _provisional) = balance_regions(&centroids, buckets, p.seed, dim, cap);
    let centroids =
        SpannCentroidIndex::from_parts(dim as u32, final_centroids, Vec::new(), Vec::new())?;
    centroids.save(root.join(CENTROID_DIR))?;

    // 2c. Re-bucket every cx against the FINAL centroids through the EXACT routing a
    //     query uses (`assign_hnsw` == `nearest_centroids` top-1). Load-bearing for
    //     recall (#711): step 2 bucketed against the INITIAL centroids (and, for
    //     moderate R, by exact L2), but queries route via the FINAL centroid HNSW.
    //     Those disagree, so a point's region was not the region its own query routes
    //     to — proven on REAL msmarco: probing ALL regions recalls 0.999, but probing
    //     the routed few recalls 0.38 (the right region was never probed). Re-bucketing
    //     through the same routing path makes assignment and routing consistent, so the
    //     routed regions actually contain a query's neighbours. Memory-bounded stream.
    let buckets = stream_assign(&centroids, source, p.chunk);

    // 3. Build one DiskANN graph per region (each fits RAM). Regions are built
    //    in PARALLEL across cores (#706): each region is small and serial row-gen
    //    + a small-graph Vamana build under-utilizes the machine, so the regions
    //    themselves are the unit of parallelism. Peak memory is bounded by
    //    `rayon` thread count x region size (NOT N), preserving the memory-bound
    //    guarantee. Each region writes to a distinct graph/ids path, so the
    //    concurrent builds never contend.
    let build_params = DiskAnnBuildParams {
        dim,
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
                .map(|&idx| (cx(idx), source.row(idx)))
                .collect();
            let graph_path = root.join(graph_rel(region));
            // NOTE: `build_diskann_graph` parallelizes internally. Running this
            // outer loop with `par_iter` nests rayon pools; for skewed region sizes
            // that is actually the safe choice — a few oversized regions still get
            // full-core builds. A single-thread-per-region scheme stalls badly until
            // region sizes are balanced (see #713: split oversized regions), which
            // is the prerequisite for higher build throughput.
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
        n_cx,
        dim,
        n_regions: centroids.centroid_count(),
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

#[cfg(test)]
mod tests;
