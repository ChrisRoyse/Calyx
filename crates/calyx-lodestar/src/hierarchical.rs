use std::collections::{BTreeMap, BTreeSet};

use calyx_core::{AnchorKind, CxId, content_address};
use calyx_paths::AssocGraph;
use serde::{Deserialize, Serialize};

use crate::{
    AssocStore, Kernel, KernelParams, Result, Scope, ScopeCache, anchors_for_scope, build_kernel,
    build_kernel_pipeline, materialize_scope,
};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RegionId(pub String);

impl From<&str> for RegionId {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegionDescriptor {
    pub id: RegionId,
    pub centroid_cx: CxId,
    pub members: BTreeSet<CxId>,
}

pub trait RegionStore: AssocStore {
    fn regions_for_scope(&self, scope: &Scope) -> Result<Vec<RegionDescriptor>>;
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HierarchicalKernelParams {
    pub max_regions: usize,
    pub drill_radius: usize,
    pub min_region_size: usize,
    pub anchor_kind: Option<AnchorKind>,
    pub kernel_params: KernelParams,
}

impl Default for HierarchicalKernelParams {
    fn default() -> Self {
        Self {
            max_regions: 64,
            drill_radius: 2,
            min_region_size: 1,
            anchor_kind: None,
            kernel_params: KernelParams::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HierarchicalKernel {
    pub region_kernel: Kernel,
    pub region_drilldowns: Vec<(RegionId, Kernel)>,
}

impl HierarchicalKernel {
    pub fn all_members(&self) -> Vec<CxId> {
        self.region_drilldowns
            .iter()
            .flat_map(|(_, kernel)| kernel.members.iter().copied())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }
}

pub fn build_hierarchical_kernel(
    store: &dyn RegionStore,
    scope: Scope,
    params: &HierarchicalKernelParams,
    cache: &mut ScopeCache,
) -> Result<HierarchicalKernel> {
    let regions = bounded_regions(store.regions_for_scope(&scope)?, params);
    if regions.is_empty() {
        let kernel = build_kernel(
            store,
            scope,
            params.anchor_kind.clone(),
            params.kernel_params.clone(),
            cache,
        )?;
        return Ok(HierarchicalKernel {
            region_kernel: kernel,
            region_drilldowns: Vec::new(),
        });
    }

    let graph = materialize_scope(&scope, store)?;
    let region_graph = build_region_graph(&graph, &regions)?;
    let region_anchors = region_anchor_nodes(&scope, store, &regions, params.anchor_kind.clone())?;
    let region_kernel =
        build_kernel_pipeline(&region_graph, &region_anchors, &params.kernel_params)?;
    let selected = selected_region_nodes(&region_kernel, &regions);
    let region_by_node: BTreeMap<_, _> = regions
        .iter()
        .map(|region| (region_node_id(&region.id), region))
        .collect();
    let mut drilldowns = Vec::new();
    for node in selected {
        let Some(region) = region_by_node.get(&node) else {
            continue;
        };
        let kernel = build_kernel(
            store,
            Scope::Subgraph {
                query: region.centroid_cx,
                radius: params.drill_radius,
            },
            params.anchor_kind.clone(),
            params.kernel_params.clone(),
            cache,
        )?;
        drilldowns.push((region.id.clone(), kernel));
    }

    Ok(HierarchicalKernel {
        region_kernel,
        region_drilldowns: drilldowns,
    })
}

fn bounded_regions(
    mut regions: Vec<RegionDescriptor>,
    params: &HierarchicalKernelParams,
) -> Vec<RegionDescriptor> {
    regions.retain(|region| region.members.len() >= params.min_region_size);
    regions.sort_by(|left, right| left.id.cmp(&right.id));
    regions.truncate(params.max_regions);
    regions
}

fn build_region_graph(graph: &AssocGraph, regions: &[RegionDescriptor]) -> Result<AssocGraph> {
    let mut builder = AssocGraph::builder();
    let node_to_region = node_to_region(regions);
    for region in regions {
        builder.add_node(
            region_node_id(&region.id),
            region.members.len().max(1) as f32,
        )?;
    }
    let mut weights = BTreeMap::<(usize, usize), f32>::new();
    for edge in graph.edges() {
        let (src, dst) = graph.edge_endpoints(*edge);
        let (Some(left), Some(right)) = (node_to_region.get(&src), node_to_region.get(&dst)) else {
            continue;
        };
        if left == right {
            continue;
        }
        let left_size = regions[*left].members.len().max(1) as f32;
        let right_size = regions[*right].members.len().max(1) as f32;
        *weights.entry((*left, *right)).or_default() += edge.weight / (left_size * right_size);
    }
    for ((left, right), weight) in weights {
        builder.add_edge(
            region_node_id(&regions[left].id),
            region_node_id(&regions[right].id),
            weight.min(1.0),
        )?;
    }
    Ok(builder.build())
}

fn region_anchor_nodes(
    scope: &Scope,
    store: &dyn RegionStore,
    regions: &[RegionDescriptor],
    anchor_kind: Option<AnchorKind>,
) -> Result<Vec<CxId>> {
    let anchors = anchors_for_scope(scope, store, anchor_kind)?;
    let node_to_region = node_to_region(regions);
    Ok(anchors
        .into_iter()
        .filter_map(|anchor| {
            node_to_region
                .get(&anchor)
                .map(|index| region_node_id(&regions[*index].id))
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect())
}

fn selected_region_nodes(kernel: &Kernel, regions: &[RegionDescriptor]) -> Vec<CxId> {
    if kernel.members.is_empty() {
        regions
            .first()
            .map(|region| vec![region_node_id(&region.id)])
            .unwrap_or_default()
    } else {
        kernel.members.clone()
    }
}

fn node_to_region(regions: &[RegionDescriptor]) -> BTreeMap<CxId, usize> {
    let mut map = BTreeMap::new();
    for (index, region) in regions.iter().enumerate() {
        for member in &region.members {
            map.insert(*member, index);
        }
    }
    map
}

fn region_node_id(id: &RegionId) -> CxId {
    CxId::from_bytes(content_address([
        b"calyx-lodestar-region-v1".as_slice(),
        id.0.as_bytes(),
    ]))
}
