use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

use calyx_core::{AnchorKind, CxId, Ts};
use calyx_lodestar::{
    AssocStore, CollectionId, FilterExpr, HierarchicalKernelParams, KernelGraphParams,
    KernelParams, RegionDescriptor, RegionId, RegionStore, Scope, ScopeCache, TenantId,
    build_hierarchical_kernel,
};
use calyx_paths::AssocGraph;
use serde_json::json;

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

#[derive(Clone)]
struct MemoryRegionStore {
    graph: AssocGraph,
    anchors: BTreeMap<AnchorKind, Vec<CxId>>,
    regions: Vec<RegionDescriptor>,
}

impl AssocStore for MemoryRegionStore {
    fn full_graph(&self) -> calyx_lodestar::Result<AssocGraph> {
        Ok(self.graph.clone())
    }

    fn collection_nodes(
        &self,
        _id: &CollectionId,
    ) -> calyx_lodestar::Result<Option<BTreeSet<CxId>>> {
        Ok(None)
    }

    fn domain_anchors(&self, kind: &AnchorKind) -> calyx_lodestar::Result<Vec<CxId>> {
        Ok(self.anchors.get(kind).cloned().unwrap_or_default())
    }

    fn time_window_nodes(
        &self,
        _t0: Ts,
        _t1: Ts,
    ) -> calyx_lodestar::Result<Option<BTreeSet<CxId>>> {
        Ok(None)
    }

    fn tenant_nodes(&self, _id: &TenantId) -> calyx_lodestar::Result<Option<BTreeSet<CxId>>> {
        Ok(None)
    }

    fn filter_nodes(&self, _expr: &FilterExpr) -> calyx_lodestar::Result<BTreeSet<CxId>> {
        Ok(BTreeSet::new())
    }
}

impl RegionStore for MemoryRegionStore {
    fn regions_for_scope(&self, _scope: &Scope) -> calyx_lodestar::Result<Vec<RegionDescriptor>> {
        Ok(self.regions.clone())
    }
}

fn three_region_store() -> MemoryRegionStore {
    let mut builder = AssocGraph::builder();
    for seed in 1..=9 {
        builder.add_node(cx(seed), 1.0).unwrap();
    }
    for (a, b, c) in [(1, 2, 3), (4, 5, 6), (7, 8, 9)] {
        builder
            .add_edge(cx(a), cx(b), 1.0)
            .unwrap()
            .add_edge(cx(b), cx(c), 1.0)
            .unwrap()
            .add_edge(cx(c), cx(a), 1.0)
            .unwrap();
    }
    builder
        .add_edge(cx(3), cx(4), 1.0)
        .unwrap()
        .add_edge(cx(6), cx(7), 1.0)
        .unwrap()
        .add_edge(cx(9), cx(1), 1.0)
        .unwrap();

    MemoryRegionStore {
        graph: builder.build(),
        anchors: BTreeMap::from([(domain_anchor(), vec![cx(1)])]),
        regions: vec![
            region("r1", 1, [1, 2, 3]),
            region("r2", 4, [4, 5, 6]),
            region("r3", 7, [7, 8, 9]),
        ],
    }
}

fn single_region_store() -> MemoryRegionStore {
    let mut builder = AssocGraph::builder();
    builder.add_node(cx(1), 1.0).unwrap();
    MemoryRegionStore {
        graph: builder.build(),
        anchors: BTreeMap::from([(domain_anchor(), vec![cx(1)])]),
        regions: vec![RegionDescriptor {
            id: RegionId::from("solo"),
            centroid_cx: cx(1),
            members: BTreeSet::from([cx(1)]),
        }],
    }
}

fn region<const N: usize>(name: &str, centroid: u8, members: [u8; N]) -> RegionDescriptor {
    RegionDescriptor {
        id: RegionId::from(name),
        centroid_cx: cx(centroid),
        members: members.into_iter().map(cx).collect(),
    }
}

fn domain_anchor() -> AnchorKind {
    AnchorKind::Label("domain".to_string())
}

fn params(max_regions: usize, drill_radius: usize, panel_version: u64) -> HierarchicalKernelParams {
    HierarchicalKernelParams {
        max_regions,
        drill_radius,
        min_region_size: 1,
        anchor_kind: Some(domain_anchor()),
        kernel_params: KernelParams {
            panel_version,
            anchor_kind: Some("label:domain".to_string()),
            corpus_shard_hash: [44; 32],
            built_at_millis: 44,
            kernel_graph: KernelGraphParams {
                target_fraction: 1.0,
                max_groundedness_distance: 4,
                ..KernelGraphParams::default()
            },
            ..KernelParams::default()
        },
    }
}

fn fsv_root(case: &str) -> PathBuf {
    let base = std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-ph34-t04"));
    base.join(case)
}

fn write_readback(case: &str, name: &str, value: serde_json::Value) {
    let root = fsv_root(case);
    fs::create_dir_all(&root).expect("create readback root");
    let path = root.join(name);
    fs::write(&path, serde_json::to_vec_pretty(&value).expect("json")).expect("write readback");
    println!("PH34_T04_READBACK={}", path.display());
}

#[test]
fn hierarchical_three_regions_builds_region_kernel_and_drilldown() {
    let store = three_region_store();
    let mut cache = ScopeCache::new(8);
    let hierarchical = build_hierarchical_kernel(
        &store,
        Scope::AllAssociations,
        &params(3, 2, 11),
        &mut cache,
    )
    .unwrap();
    let drilldown_members: usize = hierarchical
        .region_drilldowns
        .iter()
        .map(|(_, kernel)| kernel.members.len())
        .sum();

    println!(
        "PH34_HIERARCHICAL_REGIONS region_kernel={} drilldowns={} drilldown_members={}",
        hierarchical.region_kernel.members.len(),
        hierarchical.region_drilldowns.len(),
        drilldown_members
    );
    write_readback(
        "regions",
        "ph34-hierarchical-regions-readback.json",
        json!({
            "region_kernel_size": hierarchical.region_kernel.members.len(),
            "drilldown_count": hierarchical.region_drilldowns.len(),
            "drilldown_members": drilldown_members,
        }),
    );

    assert!(hierarchical.region_kernel.members.len() <= 3);
    assert!(!hierarchical.region_drilldowns.is_empty());
    assert!(drilldown_members > 0);
}

#[test]
fn hierarchical_all_members_are_deduplicated_union_of_drilldowns() {
    let store = three_region_store();
    let mut cache = ScopeCache::new(8);
    let hierarchical = build_hierarchical_kernel(
        &store,
        Scope::AllAssociations,
        &params(3, 2, 11),
        &mut cache,
    )
    .unwrap();
    let all_members = hierarchical.all_members();
    let summed: usize = hierarchical
        .region_drilldowns
        .iter()
        .map(|(_, kernel)| kernel.members.len())
        .sum();

    println!(
        "PH34_HIERARCHICAL_MEMBERS all={} summed={summed}",
        all_members.len()
    );
    write_readback(
        "members",
        "ph34-hierarchical-members-readback.json",
        json!({ "all_member_count": all_members.len(), "summed_drilldown_members": summed }),
    );

    assert!(all_members.len() <= summed);
}

#[test]
fn hierarchical_zero_regions_falls_back_to_direct_kernel() {
    let mut store = three_region_store();
    store.regions.clear();
    let mut cache = ScopeCache::new(8);
    let hierarchical = build_hierarchical_kernel(
        &store,
        Scope::AllAssociations,
        &params(3, 2, 11),
        &mut cache,
    )
    .unwrap();

    println!(
        "PH34_HIERARCHICAL_FALLBACK region_kernel={} drilldowns={}",
        hierarchical.region_kernel.members.len(),
        hierarchical.region_drilldowns.len()
    );
    write_readback(
        "fallback",
        "ph34-hierarchical-fallback-readback.json",
        json!({
            "fallback_kernel_size": hierarchical.region_kernel.members.len(),
            "drilldown_count": hierarchical.region_drilldowns.len(),
        }),
    );

    assert!(!hierarchical.region_kernel.members.is_empty());
    assert!(hierarchical.region_drilldowns.is_empty());
}

#[test]
fn hierarchical_second_call_reuses_drilldown_cache() {
    let store = three_region_store();
    let mut cache = ScopeCache::new(8);
    build_hierarchical_kernel(
        &store,
        Scope::AllAssociations,
        &params(3, 2, 11),
        &mut cache,
    )
    .unwrap();
    build_hierarchical_kernel(
        &store,
        Scope::AllAssociations,
        &params(3, 2, 11),
        &mut cache,
    )
    .unwrap();
    let stats = cache.stats();

    println!(
        "PH34_HIERARCHICAL_CACHE hits={} misses={} size={}",
        stats.hits, stats.misses, stats.current_size
    );
    write_readback(
        "cache",
        "ph34-hierarchical-cache-readback.json",
        json!({ "stats": stats }),
    );

    assert!(stats.hits > 0);
}

#[test]
fn hierarchical_edges_single_region_max_regions_and_radius_zero() {
    let mut cache = ScopeCache::new(8);
    let single = build_hierarchical_kernel(
        &single_region_store(),
        Scope::AllAssociations,
        &params(3, 2, 11),
        &mut cache,
    )
    .unwrap();
    let limited = build_hierarchical_kernel(
        &three_region_store(),
        Scope::AllAssociations,
        &params(1, 2, 12),
        &mut cache,
    )
    .unwrap();
    let radius_zero = build_hierarchical_kernel(
        &three_region_store(),
        Scope::AllAssociations,
        &params(1, 0, 13),
        &mut cache,
    )
    .unwrap();
    let radius_zero_members = radius_zero.region_drilldowns[0].1.members.len();

    println!(
        "PH34_HIERARCHICAL_EDGES single={} limited={} radius_zero_members={}",
        single.region_drilldowns.len(),
        limited.region_drilldowns.len(),
        radius_zero_members
    );
    write_readback(
        "edges",
        "ph34-hierarchical-edges-readback.json",
        json!({
            "single_region_drilldowns": single.region_drilldowns.len(),
            "single_region_members": single.region_drilldowns[0].1.members.len(),
            "max_regions_one_drilldowns": limited.region_drilldowns.len(),
            "radius_zero_members": radius_zero_members,
        }),
    );

    assert_eq!(single.region_drilldowns.len(), 1);
    assert!(single.region_drilldowns[0].1.members.len() <= 1);
    assert_eq!(limited.region_drilldowns.len(), 1);
    assert_eq!(radius_zero_members, 0);
}
