//! Kernel-first 3-hop funnel for server-scale vault search.

use std::collections::{BTreeMap, BTreeSet};

use calyx_core::{CxId, Result, SlotId, SlotVector};
use serde::{Deserialize, Serialize};

use crate::error::{
    CALYX_INDEX_FUNNEL_VAULT_TOO_SMALL, CALYX_INDEX_INVALID_PARAMS, CALYX_INDEX_IO,
    CALYX_INDEX_KERNEL_UNAVAILABLE, sextant_error,
};
use crate::index::{DiskAnnSearch, DiskAnnSearchParams, HnswIndex, SextantIndex, SpannSearch};

pub const FUNNEL_MIN_VAULT_SIZE: u64 = 10_000_000;
const KERNEL_REGION_SLOT: SlotId = SlotId::new(u16::MAX - 1);
const KERNEL_REGION_SEED: u64 = 0x4b45524e454c4655;

pub type KernelRegionId = u32;
pub type RegionId = u32;
pub type LocalCxId = u32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FunnelParams {
    pub n_kernel_probe: usize,
    pub n_region_beam: usize,
    pub n_cx_beam: usize,
    pub n_regions_to_expand: usize,
}

impl Default for FunnelParams {
    fn default() -> Self {
        Self {
            n_kernel_probe: 8,
            n_region_beam: 32,
            n_cx_beam: 64,
            n_regions_to_expand: 4,
        }
    }
}

impl FunnelParams {
    pub fn validate(&self) -> Result<()> {
        if self.n_kernel_probe == 0
            || self.n_region_beam == 0
            || self.n_cx_beam == 0
            || self.n_regions_to_expand == 0
        {
            return Err(invalid(
                "n_kernel_probe, n_region_beam, n_cx_beam, and n_regions_to_expand must be > 0",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KernelRegion {
    pub id: KernelRegionId,
    pub vector: Vec<f32>,
}

#[derive(Clone, Debug)]
pub struct KernelRegionAnn {
    dim: usize,
    rows: Vec<KernelRegion>,
    hnsw: HnswIndex,
    cx_to_region: BTreeMap<CxId, KernelRegionId>,
}

impl KernelRegionAnn {
    pub fn new(rows: Vec<KernelRegion>) -> Result<Self> {
        let dim = validate_kernel_rows(&rows)?;
        let mut hnsw = HnswIndex::new(KERNEL_REGION_SLOT, dim as u32, KERNEL_REGION_SEED);
        let mut cx_to_region = BTreeMap::new();
        for (idx, row) in rows.iter().enumerate() {
            let cx = region_cx(row.id);
            cx_to_region.insert(cx, row.id);
            hnsw.insert(
                cx,
                SlotVector::Dense {
                    dim: dim as u32,
                    data: row.vector.clone(),
                },
                idx as u64 + 1,
            )?;
        }
        hnsw.rebuild()?;
        Ok(Self {
            dim,
            rows,
            hnsw,
            cx_to_region,
        })
    }

    pub fn empty(dim: usize) -> Result<Self> {
        Ok(Self {
            dim,
            rows: Vec::new(),
            hnsw: HnswIndex::new(KERNEL_REGION_SLOT, dim as u32, KERNEL_REGION_SEED),
            cx_to_region: BTreeMap::new(),
        })
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn search(&self, query: &[f32], k: usize) -> Result<Vec<(KernelRegionId, f32)>> {
        if self.rows.is_empty() {
            return Err(kernel_unavailable("kernel region ANN has no rows"));
        }
        validate_query(self.dim, query)?;
        let hits = self.hnsw.search(
            &SlotVector::Dense {
                dim: self.dim as u32,
                data: query.to_vec(),
            },
            k.min(self.rows.len()),
            None,
        )?;
        hits.into_iter()
            .map(|hit| {
                self.cx_to_region
                    .get(&hit.cx_id)
                    .copied()
                    .map(|region| (region, hit.score))
                    .ok_or_else(|| kernel_unavailable("kernel region id map missing hit"))
            })
            .collect()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RegionPartitions {
    cx_to_region: BTreeMap<LocalCxId, RegionId>,
}

impl RegionPartitions {
    pub fn new(assignments: impl IntoIterator<Item = (LocalCxId, RegionId)>) -> Self {
        Self {
            cx_to_region: assignments.into_iter().collect(),
        }
    }

    pub fn region_for(&self, cx_id: LocalCxId) -> Option<RegionId> {
        self.cx_to_region.get(&cx_id).copied()
    }

    pub fn contains_any(&self, cx_id: LocalCxId, regions: &BTreeSet<RegionId>) -> bool {
        self.region_for(cx_id)
            .is_some_and(|region| regions.contains(&region))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegionCandidate {
    pub kernel_region: KernelRegionId,
    pub region: RegionId,
    pub score: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FunnelPath {
    pub kernel_region: KernelRegionId,
    pub region: RegionId,
    pub cx: LocalCxId,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FunnelHit {
    pub cx_id: LocalCxId,
    pub score: f32,
    pub path: FunnelPath,
}

#[derive(Debug)]
pub enum FinalCxSearch {
    DiskAnn(DiskAnnSearch),
    Spann(SpannSearch),
}

impl FinalCxSearch {
    fn search(&self, query: &[f32], k: usize, params: &FunnelParams) -> Result<Vec<(u32, f32)>> {
        match self {
            Self::DiskAnn(search) => {
                let sp = DiskAnnSearchParams {
                    beamwidth: params.n_cx_beam,
                    ef_search: params.n_cx_beam.max(k),
                    rescore_k: params.n_cx_beam.max(k),
                    rescore_from_raw: true,
                };
                let hits = search
                    .search_ids(query, k, &sp)?
                    .into_iter()
                    .map(|(id, dist)| (id, 1.0 - dist))
                    .collect();
                Ok(hits)
            }
            Self::Spann(search) => search.search(query, k, params.n_cx_beam),
        }
    }
}

#[derive(Debug)]
pub struct KernelFirstSearch {
    vault_cx_count: u64,
    min_vault_size: u64,
    kernel: Option<KernelRegionAnn>,
    region_ann: DiskAnnSearch,
    cx_search: FinalCxSearch,
    partitions: RegionPartitions,
}

impl KernelFirstSearch {
    pub fn new(
        vault_cx_count: u64,
        kernel: Option<KernelRegionAnn>,
        region_ann: DiskAnnSearch,
        cx_search: FinalCxSearch,
        partitions: RegionPartitions,
    ) -> Self {
        Self {
            vault_cx_count,
            min_vault_size: FUNNEL_MIN_VAULT_SIZE,
            kernel,
            region_ann,
            cx_search,
            partitions,
        }
    }

    pub fn with_min_vault_size(mut self, min_vault_size: u64) -> Self {
        self.min_vault_size = min_vault_size;
        self
    }

    pub fn probe_kernel(
        &self,
        query: &[f32],
        params: &FunnelParams,
    ) -> Result<Vec<KernelRegionId>> {
        params.validate()?;
        let kernel = self.kernel()?;
        Ok(kernel
            .search(query, params.n_kernel_probe)?
            .into_iter()
            .map(|(id, _)| id)
            .collect())
    }

    pub fn expand_regions(
        &self,
        kernel_hits: &[KernelRegionId],
        query: &[f32],
        params: &FunnelParams,
    ) -> Result<Vec<RegionCandidate>> {
        params.validate()?;
        if kernel_hits.is_empty() {
            return Err(kernel_unavailable("kernel probe returned no regions"));
        }
        self.validate_region_graph()?;
        let sp = DiskAnnSearchParams {
            beamwidth: params.n_region_beam,
            ef_search: params.n_region_beam.max(params.n_regions_to_expand),
            rescore_k: params.n_region_beam.max(params.n_regions_to_expand),
            ..DiskAnnSearchParams::default()
        };
        let mut out = Vec::new();
        for (region, dist) in self.region_ann.search_ids(query, sp.rescore_k, &sp)? {
            let kernel_region = if kernel_hits.contains(&region) {
                region
            } else {
                kernel_hits[0]
            };
            out.push(RegionCandidate {
                kernel_region,
                region,
                score: 1.0 - dist,
            });
            if out.len() >= params.n_regions_to_expand {
                break;
            }
        }
        Ok(out)
    }

    pub fn search_within_regions(
        &self,
        regions: &[RegionCandidate],
        query: &[f32],
        k: usize,
        params: &FunnelParams,
    ) -> Result<Vec<FunnelHit>> {
        params.validate()?;
        if k == 0 || regions.is_empty() {
            return Ok(Vec::new());
        }
        let region_ids = regions.iter().map(|r| r.region).collect::<BTreeSet<_>>();
        let by_region = regions
            .iter()
            .map(|r| (r.region, *r))
            .collect::<BTreeMap<_, _>>();
        let mut best = BTreeMap::<LocalCxId, FunnelHit>::new();
        for (cx_id, score) in self
            .cx_search
            .search(query, params.n_cx_beam.max(k), params)?
        {
            if !self.partitions.contains_any(cx_id, &region_ids) {
                continue;
            }
            let region = self.partitions.region_for(cx_id).expect("contains_any");
            let candidate = by_region.get(&region).copied().unwrap_or(RegionCandidate {
                kernel_region: region,
                region,
                score,
            });
            let hit = FunnelHit {
                cx_id,
                score,
                path: FunnelPath {
                    kernel_region: candidate.kernel_region,
                    region,
                    cx: cx_id,
                },
            };
            best.entry(cx_id)
                .and_modify(|old| {
                    if hit.score > old.score {
                        *old = hit.clone();
                    }
                })
                .or_insert(hit);
        }
        let mut hits = best.into_values().collect::<Vec<_>>();
        hits.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.cx_id.cmp(&b.cx_id))
        });
        hits.truncate(k);
        Ok(hits)
    }

    pub fn search(&self, query: &[f32], k: usize, params: &FunnelParams) -> Result<Vec<FunnelHit>> {
        if self.vault_cx_count < self.min_vault_size {
            return Err(sextant_error(
                CALYX_INDEX_FUNNEL_VAULT_TOO_SMALL,
                format!(
                    "vault has {} cx, requires at least {} for kernel-first funnel",
                    self.vault_cx_count, self.min_vault_size
                ),
            ));
        }
        let kernel_hits = self.probe_kernel(query, params)?;
        let regions = self.expand_regions(&kernel_hits, query, params)?;
        self.search_within_regions(&regions, query, k, params)
    }

    fn kernel(&self) -> Result<&KernelRegionAnn> {
        self.kernel
            .as_ref()
            .filter(|kernel| !kernel.is_empty())
            .ok_or_else(|| kernel_unavailable("kernel region ANN is unavailable"))
    }

    fn validate_region_graph(&self) -> Result<()> {
        let path = self.region_ann.persist_path();
        if path.is_file() {
            Ok(())
        } else {
            Err(sextant_error(
                CALYX_INDEX_IO,
                format!("region DiskANN graph missing at {}", path.display()),
            ))
        }
    }
}

fn validate_kernel_rows(rows: &[KernelRegion]) -> Result<usize> {
    if rows.is_empty() {
        return Err(kernel_unavailable("kernel region rows are empty"));
    }
    let dim = rows[0].vector.len();
    if dim == 0 {
        return Err(invalid(
            "kernel region vectors must have non-zero dimension",
        ));
    }
    let mut seen = BTreeSet::new();
    for row in rows {
        if !seen.insert(row.id) {
            return Err(invalid(format!("duplicate kernel region {}", row.id)));
        }
        validate_query(dim, &row.vector)?;
    }
    Ok(dim)
}

fn validate_query(dim: usize, query: &[f32]) -> Result<()> {
    if query.len() != dim {
        return Err(crate::error::sextant_error(
            crate::error::CALYX_INDEX_DIM_MISMATCH,
            format!("query dim {} expected {dim}", query.len()),
        ));
    }
    if query.iter().any(|value| !value.is_finite()) {
        return Err(invalid("query vector has non-finite component"));
    }
    Ok(())
}

fn region_cx(region: KernelRegionId) -> CxId {
    let mut bytes = [0_u8; 16];
    bytes[0..8].copy_from_slice(b"CLXKRNL0");
    bytes[12..16].copy_from_slice(&region.to_be_bytes());
    CxId::from_bytes(bytes)
}

fn invalid(detail: impl std::fmt::Display) -> calyx_core::CalyxError {
    sextant_error(
        CALYX_INDEX_INVALID_PARAMS,
        format!("kernel funnel: {detail}"),
    )
}

fn kernel_unavailable(detail: impl std::fmt::Display) -> calyx_core::CalyxError {
    sextant_error(
        CALYX_INDEX_KERNEL_UNAVAILABLE,
        format!("kernel funnel: {detail}"),
    )
}
